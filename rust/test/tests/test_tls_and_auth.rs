//! Integration tests for the authentication and TLS layers over real gRPC.
//!
//! Two complementary angles are covered:
//! * Negative paths through the anonymous, plaintext [`SurveyApiClient::new`]:
//!   an auth-enabled server rejects anonymous callers, and a TLS-enabled server
//!   is unreachable over plaintext.
//! * Positive paths through [`SurveyApiClient::with_options`], which can pin the
//!   server's self-signed certificate as a custom CA
//!   ([`GrpcTlsSetting::WithCustomCerts`]). This makes the encrypted,
//!   authenticated channel reachable end to end, so we can assert that valid
//!   credentials succeed, wrong credentials are rejected with `Unauthenticated`,
//!   and the role model is enforced — all over TLS.

mod common;

use common::{TestServer, survey_zip_bytes};
use survey_tool_api_client::grpc::{GrpcAuthSetting, GrpcTlsSetting, SurveyApiClient, SurveyApiClientError};

/// The `user:pass:roles;...` config used by the auth tests: an `admin` holding
/// both roles and a plain `user` holding only `User`.
const AUTH_CONFIG: &str = "admin:adminpw:Admin,User;user:userpw:User";

/// Build the client TLS setting that trusts the given server's self-signed
/// certificate. The server binds `127.0.0.1`, but the certificate's SAN is
/// `localhost`, so the verification domain is overridden accordingly.
fn trust_server_cert(server: &TestServer) -> GrpcTlsSetting {
    GrpcTlsSetting::WithCustomCerts {
        certificate: server.cert_pem().expect("server must be started with .tls()").as_bytes().to_vec(),
        domain: Some("localhost".to_string()),
        trust_platform_certs: false,
    }
}

/// With authentication enabled, the anonymous plaintext client is rejected with
/// `Unauthenticated` across the different services.
#[tokio::test]
async fn auth_enabled_server_rejects_anonymous_calls() {
    let server = TestServer::builder().auth(AUTH_CONFIG).start().await;
    let mut client = SurveyApiClient::new(&server.addr()).await.unwrap();

    match client.list_surveys(None, None).await {
        Err(SurveyApiClientError::GrpcUnauthenticated(_)) => {},
        other => panic!("expected Unauthenticated for list_surveys, got {other:?}"),
    }
    match client.create_survey(survey_zip_bytes()).await {
        Err(SurveyApiClientError::GrpcUnauthenticated(_)) => {},
        other => panic!("expected Unauthenticated for create_survey, got {other:?}"),
    }
    match client.get_survey_summary("whatever".to_string()).await {
        Err(SurveyApiClientError::GrpcUnauthenticated(_)) => {},
        other => panic!("expected Unauthenticated for get_survey_summary, got {other:?}"),
    }
}

/// A plaintext client cannot successfully communicate with a TLS-enabled server:
/// either the connection or the first request must fail.
#[tokio::test]
async fn plaintext_client_cannot_talk_to_tls_server() {
    let server = TestServer::builder().tls().start().await;

    let outcome: Result<(), SurveyApiClientError> = async {
        let mut client = SurveyApiClient::new(&server.addr()).await?;
        client.list_surveys(None, None).await?;
        Ok(())
    }
    .await;

    assert!(outcome.is_err(), "a plaintext client must not be able to reach a TLS server");
}

/// A TLS client that trusts the server's self-signed certificate can complete a
/// real round trip over the encrypted channel (no auth configured).
#[tokio::test]
async fn tls_client_with_pinned_cert_can_talk_to_tls_server() {
    let server = TestServer::builder().tls().start().await;

    let mut client = SurveyApiClient::with_options(&server.tls_addr(), GrpcAuthSetting::None, trust_server_cert(&server))
        .await
        .expect("a client pinning the server cert should establish the TLS channel");

    // Getting real data back proves the encrypted request/response cycle works.
    let surveys = client.list_surveys(None, None).await.expect("list_surveys over TLS should succeed");
    assert!(surveys.is_empty(), "a fresh server has no surveys");
}

/// With both TLS and auth enabled, an admin client (correct credentials + pinned
/// cert) can drive the full admin/user workflow over the secured channel.
#[tokio::test]
async fn tls_and_auth_admin_can_run_full_workflow() {
    let server = TestServer::builder().tls().auth(AUTH_CONFIG).start().await;

    let admin_auth = GrpcAuthSetting::Basic { user: "admin".to_string(), pass: "adminpw".to_string() };
    let mut client = SurveyApiClient::with_options(&server.tls_addr(), admin_auth, trust_server_cert(&server)).await.expect("admin TLS client should connect");

    // Admin-only create, followed by User-level reads, all over authenticated TLS.
    let survey_id = client.create_survey(survey_zip_bytes()).await.expect("admin create over TLS should succeed");

    let surveys = client.list_surveys(None, None).await.expect("list over TLS should succeed");
    assert_eq!(surveys.len(), 1, "the created survey should be listed");
    assert_eq!(surveys[0].id, survey_id);

    let summary = client.get_survey_summary(survey_id.clone()).await.expect("summary over TLS should succeed");
    assert_eq!(summary.id, survey_id);
}

/// Over an otherwise-valid TLS channel, wrong credentials are still rejected
/// with `Unauthenticated`: the TLS handshake and the auth check are independent.
#[tokio::test]
async fn tls_and_auth_rejects_wrong_credentials() {
    let server = TestServer::builder().tls().auth(AUTH_CONFIG).start().await;

    let bad_auth = GrpcAuthSetting::Basic { user: "admin".to_string(), pass: "wrong".to_string() };
    let mut client = SurveyApiClient::with_options(&server.tls_addr(), bad_auth, trust_server_cert(&server))
        .await
        .expect("the TLS channel is established; only the credentials are wrong");

    match client.list_surveys(None, None).await {
        Err(SurveyApiClientError::GrpcUnauthenticated(_)) => {},
        other => panic!("expected Unauthenticated for wrong credentials over TLS, got {other:?}"),
    }
}

/// Over authenticated TLS the role model is enforced: a `User` may read but the
/// `Admin`-only create is rejected with `PermissionDenied`.
#[tokio::test]
async fn tls_and_auth_enforces_roles() {
    let server = TestServer::builder().tls().auth(AUTH_CONFIG).start().await;

    let user_auth = GrpcAuthSetting::Basic { user: "user".to_string(), pass: "userpw".to_string() };
    let mut client = SurveyApiClient::with_options(&server.tls_addr(), user_auth, trust_server_cert(&server)).await.expect("user TLS client should connect");

    // `User` is allowed to list surveys...
    client.list_surveys(None, None).await.expect("the User role may list surveys over TLS");

    // ...but creating a survey requires `Admin`.
    match client.create_survey(survey_zip_bytes()).await {
        Err(SurveyApiClientError::GrpcPermissionDenied(_)) => {},
        other => panic!("expected PermissionDenied for a non-admin create over TLS, got {other:?}"),
    }
}
