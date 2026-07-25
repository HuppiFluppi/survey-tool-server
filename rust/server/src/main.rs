use crate::shared::persistence::{PersistenceError, SurveyPersistenceClient};
use crate::shared::server::{AuthSetting, TlsSetting};
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, ValueEnum};
use std::error::Error;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

#[cfg(feature = "grpc")]
mod grpc;

#[cfg(feature = "rest")]
mod rest;

#[cfg(feature = "aws")]
mod aws;

#[cfg(feature = "local")]
mod local;

mod shared;

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Opt {
    /// gRPC server address
    #[arg(long, env, default_value = "127.0.0.1:1504")]
    #[cfg(feature = "grpc")]
    grpc_address: SocketAddr,

    /// Http/Rest server address
    #[arg(long, env, default_value = "127.0.0.1:80")]
    #[cfg(feature = "rest")]
    rest_address: String,

    #[command(flatten)]
    auth: CliAuthSetting,

    #[command(flatten)]
    tls: CliTlsSetting,

    #[command(flatten)]
    persistence: CliPersistenceSetting,
}

#[derive(Args, Debug)]
struct CliPersistenceSetting {
    /// Persistence type
    #[arg(long, env, value_enum)]
    #[cfg_attr(feature = "aws", arg(default_value_t = CliPersistenceType::AWS))]
    #[cfg_attr(feature = "local", arg(default_value_t = CliPersistenceType::Local))]
    persistence_type: CliPersistenceType,

    /// The name of the bucket. This stores the survey configuration files. Required if persistence_type is 'aws'
    #[arg(long, env, required_if_eq("persistence_type", "aws"))]
    #[cfg(feature = "aws")]
    persistence_aws_bucket: Option<String>,

    /// The name of the bucket. This stores the survey configuration files. Required if persistence_type is 'aws'
    #[arg(long, env, default_value = "/survey-tool/server/storage/")]
    #[cfg(feature = "aws")]
    persistence_aws_bucket_prefix: String,

    /// The name of the dynamo db table. This stores the survey results and metadata. Required if persistence_type is 'aws'
    #[arg(long, env, required_if_eq("persistence_type", "aws"))]
    #[cfg(feature = "aws")]
    persistence_aws_dynamo_table: Option<String>,

    /// Path where to store the survey configuration files. Used for 'local' persistence
    #[arg(long, env, default_value = "./sts/files/")]
    #[cfg(feature = "local")]
    persistence_local_storage_folder: String,

    /// Path where to store the survey results and metadata db. Used for 'local' persistence
    #[arg(long, env, default_value = "./sts/db/")]
    #[cfg(feature = "local")]
    persistence_local_db_folder: String,

    /// Set if folders should not be created. Execution will then fail if folders don't exist. Used for 'local' persistence
    #[arg(long, env)]
    #[cfg(feature = "local")]
    persistence_local_no_create: bool,
}

#[derive(Args, Debug)]
struct CliAuthSetting {
    /// Type of auth
    #[arg(long, env, value_enum, default_value_t = CliAuthType::None)]
    auth_setting: CliAuthType,

    /// Auth configuration string. Required when auth_setting is 'simple'.
    /// Format: 'user1:pass1:role1,role2;user2:pass2:role1,role2'
    #[arg(long, env, required_if_eq("auth_setting", "simple"))]
    auth_config: Option<String>,
}

#[derive(Args, Debug)]
struct CliTlsSetting {
    /// Tls setting
    #[arg(long, env, value_enum, default_value_t = CliTlsType::Off)]
    tls_setting: CliTlsType,

    /// File path for cert pem. Required when tls_setting is 'pem'
    #[arg(long, env, required_if_eq("tls_setting", "pem"))]
    tls_cert_pem_file: Option<String>,

    /// File path for key pem. Required when tls_setting is 'pem'
    #[arg(long, env, required_if_eq("tls_setting", "pem"))]
    tls_key_pem_file: Option<String>,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliPersistenceType {
    /// Local setup. Save files and db to filesystem
    #[cfg(feature = "local")]
    Local,
    /// AWS setup. Use S3 and DynamoDB for persistence
    #[cfg(feature = "aws")]
    AWS,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliAuthType {
    /// Disable authentication
    None,
    /// Enable simple authentication
    Simple,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliTlsType {
    /// Turn TLS off
    Off,
    /// Enable TLS via PEM files
    Pem,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // --- init config & setup
    let opt = Opt::parse();

    // auth
    let auth = get_auth_setting(&opt.auth);

    // tls
    let tls = get_tls_setting(&opt.tls).await;

    // persistence
    let persistence = setup_persistence(&opt.persistence)
        .await
        .unwrap_or_else(|e| Opt::command().error(ErrorKind::ValueValidation, format!("Error setting up persistence: {e}")).exit());

    // --- welcome banner
    welcome_banner();

    // --- start operation
    let mut handles = tokio::task::JoinSet::new();

    // setup grpc
    #[cfg(feature = "grpc")]
    {
        println!("Starting gRPC service...");
        handles.spawn(grpc::SurveyApiServer::serve_with_config(opt.grpc_address, persistence.clone(), auth, tls));
        println!("  ✅  running ({})", opt.grpc_address);
    }

    // setup rest
    #[cfg(feature = "rest")]
    {
        println!("Starting Rest service...");
        handles.spawn(rest::SurveyApiServer::serve_with_config(opt.rest_address, persistence, auth, tls));
        println!("  ✅  running ({})", opt.rest_address)
    }

    handles.join_all().await;

    Ok(())
}

fn welcome_banner() {
    let mut features = Vec::new();
    if cfg!(feature = "grpc") {
        features.push("grpc")
    }
    if cfg!(feature = "rest") {
        features.push("rest")
    }
    if cfg!(feature = "local") {
        features.push("local")
    }
    if cfg!(feature = "aws") {
        features.push("aws")
    }
    println!("|>--------------------------------------------------------<|");
    println!("|>               ~~~ Survey Tool Server ~~~               <|");
    println!("|>                                                        <|");
    println!("|>  Version: {:10}                                   <|", env!("CARGO_PKG_VERSION"));
    println!("|>  Enabled features: {:32}    <|", features.join(","));
    println!("|>--------------------------------------------------------<|");
}

async fn get_tls_setting(tls: &CliTlsSetting) -> TlsSetting {
    match (&tls.tls_setting, &tls.tls_cert_pem_file, &tls.tls_key_pem_file) {
        (CliTlsType::Off, _, _) => TlsSetting::off(),
        (CliTlsType::Pem, Some(cert_file), Some(key_file)) => {
            let cert = match tokio::fs::read_to_string(cert_file).await {
                Ok(s) => s,
                Err(e) => Opt::command().error(ErrorKind::ValueValidation, format!("Error reading cert file: {e}")).exit(),
            };
            let key = match tokio::fs::read_to_string(key_file).await {
                Ok(s) => s,
                Err(e) => Opt::command().error(ErrorKind::ValueValidation, format!("Error reading key file: {e}")).exit(),
            };
            TlsSetting::pem(cert, key)
        },
        _ => Opt::command().error(ErrorKind::MissingRequiredArgument, "Invalid tls config").exit(),
    }
}

fn get_auth_setting(auth: &CliAuthSetting) -> AuthSetting {
    match (&auth.auth_setting, &auth.auth_config) {
        (CliAuthType::None, _) => AuthSetting::None,
        (CliAuthType::Simple, Some(config)) => {
            let entries = config
                .split(';')
                .map(|e| {
                    let elements: Vec<_> = e.split(':').collect();
                    let [u, p, r] = elements.as_slice() else { Opt::command().error(ErrorKind::ValueValidation, "Invalid auth config").exit() };
                    if u.is_empty() || p.is_empty() || r.is_empty() {
                        Opt::command().error(ErrorKind::ValueValidation, "Invalid auth config").exit()
                    }
                    let roles: Vec<_> = r
                        .split(',')
                        .map(|r| {
                            shared::server::Roles::from_str(r)
                                .unwrap_or_else(|f| Opt::command().error(ErrorKind::ValueValidation, format!("Invalid roles: {f}")).exit())
                        })
                        .collect();
                    (u.to_string(), p.to_string(), roles)
                })
                .collect();
            AuthSetting::simple(entries)
        },
        _ => Opt::command().error(ErrorKind::MissingRequiredArgument, "Invalid auth config").exit(),
    }
}

async fn setup_persistence(persistence: &CliPersistenceSetting) -> Result<Arc<dyn SurveyPersistenceClient>, PersistenceError> {
    match persistence.persistence_type {
        #[cfg(feature = "local")]
        CliPersistenceType::Local => {
            Ok(local::new(&persistence.persistence_local_storage_folder, &persistence.persistence_local_db_folder, persistence.persistence_local_no_create)
                .await?)
        },
        #[cfg(feature = "aws")]
        CliPersistenceType::AWS => {
            Ok(aws::new(&persistence.persistence_aws_bucket, &persistence.persistence_aws_bucket_prefix, &persistence.persistence_aws_dynamo_table).await?)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::server::Roles;

    // ----------------------------------------------------------------------
    // Test helpers
    // ----------------------------------------------------------------------

    /// A temporary PEM file that is removed again when the test finishes.
    struct TempFile {
        path: std::path::PathBuf,
    }

    impl TempFile {
        fn with_content(content: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("survey-tool-main-test-{}.pem", uuid::Uuid::new_v4()));
            std::fs::write(&path, content).expect("could not write temp pem file");
            TempFile { path }
        }

        fn as_str(&self) -> &str {
            self.path.to_str().unwrap()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    // ----------------------------------------------------------------------
    // get_auth_setting: parsing of the 'user:pass:roles;...' config string
    // ----------------------------------------------------------------------

    #[test]
    fn auth_none_yields_none_variant() {
        // The config value is irrelevant when auth is disabled.
        let setting = CliAuthType::None;
        let config = Some("ignored:ignored:ADMIN");
        let result = get_auth_setting(&CliAuthSetting { auth_setting: setting, auth_config: config.map(str::to_string) });
        assert!(matches!(result, AuthSetting::None));
    }

    #[test]
    fn auth_simple_parses_user_password_key_and_multiple_roles() {
        let setting = CliAuthType::Simple;
        let config = Some("alice:secret:ADMIN,USER");
        let result = get_auth_setting(&CliAuthSetting { auth_setting: setting, auth_config: config.map(str::to_string) });

        let AuthSetting::Simple { auth_mapping } = result else { panic!("expected simple auth") };
        assert_eq!(auth_mapping.len(), 1);
        // The map key combines user and password as 'user:pass', roles keep their order.
        assert_eq!(auth_mapping.get("alice:secret"), Some(&vec![Roles::Admin, Roles::User]));
    }

    #[test]
    fn auth_simple_parses_multiple_entries() {
        let setting = CliAuthType::Simple;
        let config = Some("alice:pw1:ADMIN;bob:pw2:USER");
        let result = get_auth_setting(&CliAuthSetting { auth_setting: setting, auth_config: config.map(str::to_string) });

        let AuthSetting::Simple { auth_mapping } = result else { panic!("expected simple auth") };
        assert_eq!(auth_mapping.len(), 2);
        assert_eq!(auth_mapping.get("alice:pw1"), Some(&vec![Roles::Admin]));
        assert_eq!(auth_mapping.get("bob:pw2"), Some(&vec![Roles::User]));
    }

    // ----------------------------------------------------------------------
    // get_tls_setting: off short-circuits, pem reads both files
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn tls_off_yields_none_variant() {
        let setting = CliTlsSetting { tls_setting: CliTlsType::Off, tls_cert_pem_file: None, tls_key_pem_file: None };
        assert!(matches!(get_tls_setting(&setting).await, TlsSetting::None));
    }

    #[tokio::test]
    async fn tls_pem_reads_cert_and_key_file_contents() {
        let cert = TempFile::with_content("CERT-CONTENT");
        let key = TempFile::with_content("KEY-CONTENT");

        let setting = CliTlsSetting {
            tls_setting: CliTlsType::Pem,
            tls_cert_pem_file: Some(cert.as_str().to_string()),
            tls_key_pem_file: Some(key.as_str().to_string()),
        };

        let TlsSetting::Pem { cert, key } = get_tls_setting(&setting).await else { panic!("expected pem tls") };
        assert_eq!(cert, "CERT-CONTENT");
        assert_eq!(key, "KEY-CONTENT");
    }
}
