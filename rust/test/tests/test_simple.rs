//! Integration tests over real gRPC with authentication and TLS disabled.
//!
//! Each test launches a fresh `survey-tool-server` process and drives it through
//! the `survey-tool-api-client` library, exercising the full survey lifecycle
//! and the important error paths end to end.

mod common;

use common::{TestServer, quiz_result, survey_zip_bytes};
use survey_tool_api_client::grpc::{SurveyApiClient, SurveyApiClientError, SurveyType};

/// Upload, inspect, submit results to, and delete a survey — the happy path
/// through every service, verified over the wire.
#[tokio::test]
async fn full_survey_lifecycle_round_trips_through_grpc() {
    let server = TestServer::builder().start().await;
    let mut client = SurveyApiClient::new(&server.addr()).await.expect("client should connect");

    // Nothing there to begin with.
    assert!(client.list_surveys(None, None).await.unwrap().is_empty());

    // Upload the example quiz bundle.
    let zip = survey_zip_bytes();
    let id = client.create_survey(zip.clone()).await.unwrap();
    assert!(!id.is_empty(), "server must return a survey id");

    // It now shows up and the metadata indexed from survey_config.yaml matches.
    let all = client.list_surveys(None, None).await.unwrap();
    assert_eq!(all.len(), 1);
    let summary = &all[0];
    assert_eq!(summary.id, id);
    assert_eq!(summary.title, "Template Survey");
    assert_eq!(summary.survey_type(), SurveyType::Quiz);
    assert!(summary.active);
    assert_eq!(summary.page_count, 2);
    assert_eq!(summary.question_count, 9);
    assert!(summary.conditionals);
    assert_eq!(summary.submit_count, 0);

    // The stored bundle is returned byte-for-byte.
    assert_eq!(client.get_survey(id.clone()).await.unwrap(), zip);

    // Server-side type filtering works.
    assert_eq!(client.list_surveys(Some(SurveyType::Quiz), None).await.unwrap().len(), 1);
    assert!(client.list_surveys(Some(SurveyType::Survey), None).await.unwrap().is_empty());

    // Submit two quiz results with different scores.
    client.add_survey_result(id.clone(), quiz_result("alice", 30, 1_700_000_000)).await.unwrap();
    client.add_survey_result(id.clone(), quiz_result("bob", 10, 1_700_000_100)).await.unwrap();

    // The summary aggregates them.
    let summary = client.get_survey_summary(id.clone()).await.unwrap();
    assert_eq!(summary.submit_count, 2);
    assert_eq!(summary.min_score, Some(10));
    assert_eq!(summary.max_score, Some(30));
    assert_eq!(summary.avg_score, Some(20.0));
    assert!(summary.first_submit_time.is_some());
    assert!(summary.last_submit_time.is_some());

    // The highscore is ordered best-first.
    let highscore = client.get_survey_highscore(id.clone(), None).await.unwrap();
    assert_eq!(highscore.len(), 2);
    assert_eq!(highscore[0].name, "alice");
    assert_eq!(highscore[0].score, 30);
    assert_eq!(highscore[1].name, "bob");
    assert_eq!(highscore[1].score, 10);

    // The limit is honoured.
    assert_eq!(client.get_survey_highscore(id.clone(), Some(1)).await.unwrap().len(), 1);

    // The raw results are retrievable and carry the submitted data.
    let results = client.get_survey_results(id.clone()).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|r| r.user.as_deref() == Some("alice") && r.score == Some(30)));
    assert!(results.iter().all(|r| r.origin == "integration-test"));

    // Deleting the results keeps the survey but clears its stats.
    client.delete_survey_results(id.clone()).await.unwrap();
    assert!(client.get_survey_results(id.clone()).await.unwrap().is_empty());
    assert_eq!(client.get_survey_summary(id.clone()).await.unwrap().submit_count, 0);

    // Delete the survey; it disappears entirely.
    client.delete_survey(id.clone()).await.unwrap();
    assert!(client.list_surveys(None, None).await.unwrap().is_empty());
    match client.get_survey(id.clone()).await {
        Err(SurveyApiClientError::GrpcNotFound(_)) => {},
        other => panic!("expected NotFound after delete, got {other:?}"),
    }
}

/// An inactive survey rejects new results with `PermissionDenied`, and accepts
/// them again once reactivated.
#[tokio::test]
async fn inactive_survey_rejects_new_results() {
    let server = TestServer::builder().start().await;
    let mut client = SurveyApiClient::new(&server.addr()).await.unwrap();

    let id = client.create_survey(survey_zip_bytes()).await.unwrap();
    client.set_survey_status(id.clone(), false).await.unwrap();

    match client.add_survey_result(id.clone(), quiz_result("carol", 5, 1_700_000_000)).await {
        Err(SurveyApiClientError::GrpcPermissionDenied(_)) => {},
        other => panic!("expected PermissionDenied for inactive survey, got {other:?}"),
    }

    // Reactivating makes submissions succeed again.
    client.set_survey_status(id.clone(), true).await.unwrap();
    client.add_survey_result(id.clone(), quiz_result("carol", 5, 1_700_000_000)).await.unwrap();
    assert_eq!(client.get_survey_summary(id).await.unwrap().submit_count, 1);
}

/// Operations on unknown survey ids surface as the typed `NotFound` error.
#[tokio::test]
async fn unknown_survey_ids_map_to_not_found() {
    let server = TestServer::builder().start().await;
    let mut client = SurveyApiClient::new(&server.addr()).await.unwrap();

    match client.get_survey("does-not-exist".to_string()).await {
        Err(SurveyApiClientError::GrpcNotFound(_)) => {},
        other => panic!("expected NotFound for get_survey, got {other:?}"),
    }
    match client.get_survey_summary("does-not-exist".to_string()).await {
        Err(SurveyApiClientError::GrpcNotFound(_)) => {},
        other => panic!("expected NotFound for get_survey_summary, got {other:?}"),
    }
    match client.get_survey_highscore("does-not-exist".to_string(), None).await {
        Err(SurveyApiClientError::GrpcNotFound(_)) => {},
        other => panic!("expected NotFound for get_survey_highscore, got {other:?}"),
    }
}
