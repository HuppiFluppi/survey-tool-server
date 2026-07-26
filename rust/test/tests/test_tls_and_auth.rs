//! Integration tests for the authentication and TLS layers over real gRPC.
//!
//! Note on scope: the bundled client sends credentials only over a TLS channel
//! validated against the system's native root store
//! ([`SurveyApiClient::with_options`]); [`SurveyApiClient::new`] is plaintext and
//! anonymous. A self-signed test certificate cannot be validated against the
//! system roots, so the *positive* auth/TLS paths are not reachable through the
//! public client API. These tests therefore verify what the client can observe
//! end to end: that the server actually enforces both layers.

mod common;

use common::{TestServer, survey_zip_bytes};
use survey_tool_api_client::grpc::{SurveyApiClient, SurveyApiClientError};

/// With authentication enabled, the anonymous plaintext client is rejected with
/// `Unauthenticated` across the different services.
#[tokio::test]
async fn auth_enabled_server_rejects_anonymous_calls() {
    let server = TestServer::builder().auth("admin:adminpw:Admin,User;user:userpw:User").start().await;
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
