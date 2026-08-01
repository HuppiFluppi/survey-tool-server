//! Shared harness for the integration tests.
//!
//! The tests exercise *real* communication between the [`survey-tool-api-client`]
//! library and the `survey-tool-server` **binary**: the server is launched as a
//! child process (default `grpc` + `local` features, i.e. gRPC over a SQLite +
//! filesystem backend) and the client library connects to it over the network.
//!
//! Each [`TestServer`] gets its own free TCP port and its own throwaway
//! persistence directory so the tests are fully isolated and can run in
//! parallel. All temporary resources are cleaned up on drop, and the child
//! process is killed when its [`TestServer`] goes out of scope.

// Not every test binary uses every helper; that is expected for a shared module.
#![allow(dead_code)]

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use survey_tool_api_client::grpc::SurveyResult;

// ---------------------------------------------------------------------------
// Locating / building the server binary
// ---------------------------------------------------------------------------

static BUILD: Once = Once::new();
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Path of the cargo executable, falling back to the one on `PATH`.
fn cargo() -> String {
    option_env!("CARGO").unwrap_or("cargo").to_string()
}

/// The profile output directory (`target/debug` or `target/release`) derived
/// from the currently running test binary.
fn profile_dir() -> PathBuf {
    // current_exe: <target>/<profile>/deps/<test-binary>
    let mut p = std::env::current_exe().expect("could not resolve current_exe");
    p.pop(); // -> <target>/<profile>/deps
    p.pop(); // -> <target>/<profile>
    p
}

/// Ensure the `survey-tool-server` binary is built (once per test process).
///
/// Running `cargo test` from the workspace already builds it, but building it
/// explicitly makes the tests robust when run in isolation (e.g. `cargo test
/// -p survey-tool-test`).
fn ensure_server_built() {
    BUILD.call_once(|| {
        let release = profile_dir().file_name().map(|n| n == "release").unwrap_or(false);
        let mut cmd = Command::new(cargo());
        cmd.args(["build", "-p", "survey-tool-server"]);
        if release {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("failed to run `cargo build -p survey-tool-server`");
        assert!(status.success(), "building the survey-tool-server binary failed");
    });
}

/// Absolute path of the compiled server binary.
fn server_binary() -> PathBuf {
    let mut p = profile_dir();
    p.push(if cfg!(windows) { "survey-tool-server.exe" } else { "survey-tool-server" });
    p
}

// ---------------------------------------------------------------------------
// Temp resources with automatic cleanup
// ---------------------------------------------------------------------------

/// Build a unique, not-yet-created path inside the system temp directory.
fn unique_temp_path() -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("survey-tool-it-{}-{nanos}-{n}", std::process::id()));
    p
}

/// A temporary directory removed again when dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let path = unique_temp_path();
        std::fs::create_dir_all(&path).expect("could not create temp dir");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A temporary file (used for the PEM cert/key) removed again when dropped.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn with_content(content: &str) -> Self {
        let mut path = unique_temp_path();
        path.set_extension("pem");
        std::fs::write(&path, content).expect("could not write temp pem file");
        TempFile { path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Test server
// ---------------------------------------------------------------------------

/// Builder for a [`TestServer`].
pub struct Builder {
    auth_config: Option<String>,
    tls: bool,
}

impl Builder {
    /// Enable basic authentication with the given `user:pass:roles;...` config.
    pub fn auth(mut self, config: &str) -> Self {
        self.auth_config = Some(config.to_string());
        self
    }

    /// Enable TLS using a freshly generated self-signed certificate.
    pub fn tls(mut self) -> Self {
        self.tls = true;
        self
    }

    /// Spawn the server and wait until it accepts connections.
    pub async fn start(self) -> TestServer {
        TestServer::spawn(self)
    }
}

/// A running `survey-tool-server` child process backed by throwaway storage.
pub struct TestServer {
    child: Child,
    port: u16,
    // Kept alive for the lifetime of the server so they are cleaned up on drop.
    _storage: TempDir,
    _db: TempDir,
    _tls_files: Vec<TempFile>,
}

impl TestServer {
    /// Start configuring a server.
    pub fn builder() -> Builder {
        Builder { auth_config: None, tls: false }
    }

    /// The base URL a plaintext client uses to reach this server.
    pub fn addr(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn spawn(builder: Builder) -> TestServer {
        ensure_server_built();

        let port = free_port();
        let storage = TempDir::new();
        let db = TempDir::new();

        let mut cmd = Command::new(server_binary());
        cmd.arg("--grpc-address")
            .arg(format!("127.0.0.1:{port}"))
            .arg("--persistence-local-storage-folder")
            .arg(&storage.path)
            .arg("--persistence-local-db-folder")
            .arg(&db.path)
            // Keep the test output clean; surface server errors via stderr.
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        if let Some(config) = &builder.auth_config {
            cmd.args(["--auth-setting", "basic", "--auth-config", config]);
        }

        let mut tls_files = Vec::new();
        if builder.tls {
            let (cert, key) = generate_self_signed_pem();
            let cert_file = TempFile::with_content(&cert);
            let key_file = TempFile::with_content(&key);
            cmd.args(["--tls-setting", "pem"]);
            cmd.arg("--tls-cert-pem-file").arg(&cert_file.path);
            cmd.arg("--tls-key-pem-file").arg(&key_file.path);
            tls_files.push(cert_file);
            tls_files.push(key_file);
        }

        let mut child = cmd.spawn().expect("failed to spawn survey-tool-server binary");

        wait_ready(&mut child, port);

        TestServer { child, port, _storage: storage, _db: db, _tls_files: tls_files }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Block until the server's TCP port accepts connections (transport-agnostic, so
/// it works for both plaintext and TLS servers). Fails fast if the child exits
/// during startup (e.g. an invalid configuration).
fn wait_ready(child: &mut Child, port: u16) {
    let addr = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().expect("could not poll server process") {
            panic!("survey-tool-server exited during startup with {status}");
        }
        if TcpStream::connect(&addr).is_ok() {
            return;
        }
        if Instant::now() > deadline {
            panic!("survey-tool-server did not become ready on {addr} within the timeout");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Grab a currently-free TCP port on the loopback interface.
fn free_port() -> u16 {
    // Binding to port 0 lets the OS pick a free port; we drop the listener and
    // hand the port to the server. The reuse window is negligible in tests.
    std::net::TcpListener::bind("127.0.0.1:0").expect("could not bind an ephemeral port").local_addr().unwrap().port()
}

/// Generate a self-signed certificate + PKCS#8 key as PEM strings.
fn generate_self_signed_pem() -> (String, String) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("failed to generate self-signed cert");
    (cert.pem(), signing_key.serialize_pem())
}

// ---------------------------------------------------------------------------
// Test data helpers
// ---------------------------------------------------------------------------

/// Read the example survey bundle shipped in the test crate root.
pub fn survey_zip_bytes() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("survey_config.zip");
    std::fs::read(&path).unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Build a quiz result for `user` with the given `score`.
///
/// `start_seconds` seeds the (RFC 3339) start/end timestamps so callers can
/// produce results with a well-defined ordering; answers are left empty, which
/// is sufficient to exercise submission, summaries and the highscore.
pub fn quiz_result(user: &str, score: i32, start_seconds: i64) -> SurveyResult {
    SurveyResult {
        origin: "integration-test".to_string(),
        start_time: Some(prost_types::Timestamp { seconds: start_seconds, nanos: 0 }),
        end_time: Some(prost_types::Timestamp { seconds: start_seconds + 60, nanos: 0 }),
        user: Some(user.to_string()),
        score: Some(score),
        answered_pages: 2,
        answered_questions: 9,
        answers: Vec::new(),
    }
}
