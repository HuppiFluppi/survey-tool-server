//! Grpc client module

// make the generated proto/grpc code available to this library
mod grpc_survey_api {
    tonic::include_proto!("survey.v1");
}

use crate::grpc::grpc_survey_api as api;
pub use crate::grpc::grpc_survey_api::HighscoreEntry;
pub use crate::grpc::grpc_survey_api::SurveyResult;
pub use crate::grpc::grpc_survey_api::SurveySummary;
pub use crate::grpc::grpc_survey_api::SurveyType;
use crate::grpc::grpc_survey_api::survey_data_service_client::SurveyDataServiceClient;
use crate::grpc::grpc_survey_api::survey_results_service_client::SurveyResultsServiceClient;
use crate::grpc::grpc_survey_api::survey_service_client::SurveyServiceClient;
use core::fmt;
use std::error::Error;
use std::fmt::{Debug, Display};
use tonic::codegen::http;

pub struct SurveyApiClient {
    survey_client: SurveyServiceClient<tonic::transport::Channel>,
    survey_data_client: SurveyDataServiceClient<tonic::transport::Channel>,
    survey_results_client: SurveyResultsServiceClient<tonic::transport::Channel>,

    auth_setting: GrpcAuthSetting,
}

impl SurveyApiClient {
    pub async fn new(destination: &str) -> Result<SurveyApiClient, SurveyApiClientError> {
        Ok(SurveyApiClient {
            survey_client: SurveyServiceClient::connect(destination.to_owned()).await?,
            survey_data_client: SurveyDataServiceClient::connect(destination.to_owned()).await?,
            survey_results_client: SurveyResultsServiceClient::connect(destination.to_owned()).await?,
            auth_setting: GrpcAuthSetting::None,
        })
    }

    pub async fn with_options(destination: &str, auth: GrpcAuthSetting) -> Result<SurveyApiClient, SurveyApiClientError> {
        let tls_conf = tonic::transport::ClientTlsConfig::new().with_enabled_roots();
        let channel = tonic::transport::Channel::from_shared(destination.to_owned())?.tls_config(tls_conf)?.connect().await?;

        Ok(SurveyApiClient {
            survey_client: SurveyServiceClient::new(channel.clone()),
            survey_data_client: SurveyDataServiceClient::new(channel.clone()),
            survey_results_client: SurveyResultsServiceClient::new(channel),
            auth_setting: auth,
        })
    }

    fn set_request_auth<T>(&self, request: &mut tonic::Request<T>) {
        match &self.auth_setting {
            GrpcAuthSetting::None => {},
            GrpcAuthSetting::Simple { user, pass } => {
                request.metadata_mut().insert("user", user.parse().unwrap());
                request.metadata_mut().insert("pass", pass.parse().unwrap());
            },
        }
    }

    //--- SurveyService methods
    pub async fn list_surveys(&mut self, survey_type_filter: Option<SurveyType>, survey_active_filter: Option<bool>) -> SACResult<Vec<SurveySummary>> {
        let mut req = tonic::Request::new(api::ListSurveysRequest { r#type: survey_type_filter.map(|o| o as i32), active: survey_active_filter });
        self.set_request_auth(&mut req);

        let response = self.survey_client.list_surveys(req).await?;

        Ok(response.into_inner().surveys)
    }

    pub async fn create_survey(&mut self, package_content: Vec<u8>) -> SACResult<String> {
        let mut req = tonic::Request::new(api::CreateSurveyRequest { zip_content: package_content });
        self.set_request_auth(&mut req);

        let response = self.survey_client.create_survey(req).await?;

        Ok(response.into_inner().survey_id)
    }

    pub async fn get_survey(&mut self, survey_id: String) -> SACResult<Vec<u8>> {
        let mut req = tonic::Request::new(api::GetSurveyRequest { survey_id });
        self.set_request_auth(&mut req);

        let response = self.survey_client.get_survey(req).await?;

        Ok(response.into_inner().zip_content)
    }

    pub async fn delete_survey(&mut self, survey_id: String) -> SACResult<()> {
        let mut req = tonic::Request::new(api::DeleteSurveyRequest { survey_id });
        self.set_request_auth(&mut req);

        self.survey_client.delete_survey(req).await?;

        Ok(())
    }

    pub async fn set_survey_status(&mut self, survey_id: String, active_state: bool) -> SACResult<()> {
        let mut req = tonic::Request::new(api::SetSurveyActiveRequest { survey_id, active: active_state });
        self.set_request_auth(&mut req);

        self.survey_client.set_survey_active(req).await?;

        Ok(())
    }

    //--- SurveyResultsService
    pub async fn get_survey_results(&mut self, survey_id: String) -> SACResult<Vec<SurveyResult>> {
        let mut req = tonic::Request::new(api::GetResultsRequest { survey_id });
        self.set_request_auth(&mut req);

        let response = self.survey_results_client.get_results(req).await?;

        Ok(response.into_inner().results)
    }

    pub async fn add_survey_result(&mut self, survey_id: String, result: SurveyResult) -> SACResult<()> {
        let mut req = tonic::Request::new(api::AddResultRequest { survey_id, result: Some(result) });
        self.set_request_auth(&mut req);

        self.survey_results_client.add_result(req).await?;

        Ok(())
    }

    pub async fn delete_survey_results(&mut self, survey_id: String) -> SACResult<()> {
        let mut req = tonic::Request::new(api::DeleteResultsRequest { survey_id });
        self.set_request_auth(&mut req);

        self.survey_results_client.delete_results(req).await?;

        Ok(())
    }

    //--- SurveyDataService
    pub async fn get_survey_summary(&mut self, survey_id: String) -> SACResult<SurveySummary> {
        let mut req = tonic::Request::new(api::GetSurveySummaryRequest { survey_id });
        self.set_request_auth(&mut req);

        let response = self.survey_data_client.get_survey_summary(req).await?;

        Ok(response.into_inner().summary.expect("summary must always be set"))
    }

    pub async fn get_survey_highscore(&mut self, survey_id: String, limit: Option<u32>) -> SACResult<Vec<HighscoreEntry>> {
        let mut req = tonic::Request::new(api::GetHighscoreRequest { survey_id, limit });
        self.set_request_auth(&mut req);

        let response = self.survey_data_client.get_highscore(req).await?;

        Ok(response.into_inner().entries)
    }
}

pub enum GrpcAuthSetting {
    None,
    Simple { user: String, pass: String },
}

// --- Error model
#[derive(Debug)]
pub enum SurveyApiClientError {
    TonicTransportError(tonic::transport::Error),
    GrpcInvalidArgument(tonic::Status),
    GrpcUnauthenticated(tonic::Status),
    GrpcPermissionDenied(tonic::Status),
    GrpcNotFound(tonic::Status),
    GrpcInternal(tonic::Status),
    GrpcFailureStatus(tonic::Status),
    InvalidUri(http::uri::InvalidUri),
}

impl Display for SurveyApiClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error: {self:?}")
    }
}

impl Error for SurveyApiClientError {}

impl From<tonic::transport::Error> for SurveyApiClientError {
    fn from(error: tonic::transport::Error) -> Self {
        SurveyApiClientError::TonicTransportError(error)
    }
}

impl From<http::uri::InvalidUri> for SurveyApiClientError {
    fn from(value: http::uri::InvalidUri) -> Self {
        SurveyApiClientError::InvalidUri(value)
    }
}

impl From<tonic::Status> for SurveyApiClientError {
    fn from(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::InvalidArgument => Self::GrpcInvalidArgument(status),
            tonic::Code::Unauthenticated => Self::GrpcUnauthenticated(status),
            tonic::Code::PermissionDenied => Self::GrpcPermissionDenied(status),
            tonic::Code::NotFound => Self::GrpcNotFound(status),
            tonic::Code::Internal => Self::GrpcInternal(status),
            _ => Self::GrpcFailureStatus(status),
        }
    }
}

// --- Result type
pub type SACResult<T> = Result<T, SurveyApiClientError>;
