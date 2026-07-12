//! Grpc client module

// make the generated proto/grpc code available to this library
mod grpc_survey_api {
    tonic::include_proto!("survey.v1");
}

use crate::grpc::grpc_survey_api as api;
use crate::grpc::grpc_survey_api::survey_data_service_server::{SurveyDataService, SurveyDataServiceServer};
use crate::grpc::grpc_survey_api::survey_results_service_server::{SurveyResultsService, SurveyResultsServiceServer};
use crate::grpc::grpc_survey_api::survey_service_server::{SurveyService, SurveyServiceServer};
use crate::persistence;
use crate::shared::AuthSetting;
use crate::shared::TlsSetting;
use crate::shared::ROLES;
use std::net::SocketAddr;
use std::sync::Arc;

use tonic::{transport::Server, Request, Response, Status};

pub struct SurveyApiServer {
    persistence: persistence::SurveyPersistenceClient,
    auth_setting: AuthSetting,
}

impl SurveyApiServer {
    pub async fn serve(address: SocketAddr, persistence: persistence::SurveyPersistenceClient) -> Result<(), tonic::transport::Error> {
        let server = Arc::new(SurveyApiServer { persistence, auth_setting: AuthSetting::None });

        Server::builder()
            .add_service(SurveyServiceServer::from_arc(server.clone()))
            .add_service(SurveyDataServiceServer::from_arc(server.clone()))
            .add_service(SurveyResultsServiceServer::from_arc(server.clone()))
            .serve(address)
            .await?;

        Ok(())
    }

    pub async fn serve_with_config(
        address: SocketAddr,
        persistence: persistence::SurveyPersistenceClient,
        auth: AuthSetting,
        tls: TlsSetting,
    ) -> Result<(), tonic::transport::Error> {
        let server = Arc::new(SurveyApiServer { persistence, auth_setting: auth });
        let tls_config = match tls {
            TlsSetting::None => tonic::transport::ServerTlsConfig::new(),
            TlsSetting::Pem { cert, key } => tonic::transport::ServerTlsConfig::new().identity(tonic::transport::Identity::from_pem(cert, key)),
        };

        Server::builder()
            .tls_config(tls_config)?
            .add_service(SurveyServiceServer::from_arc(server.clone()))
            .add_service(SurveyDataServiceServer::from_arc(server.clone()))
            .add_service(SurveyResultsServiceServer::from_arc(server.clone()))
            .serve(address)
            .await?;

        Ok(())
    }

    fn check_auth<T>(&self, req: &Request<T>, allowed_roles: &[ROLES]) -> Result<(), Status> {
        match &self.auth_setting {
            AuthSetting::None => Ok(()),
            AuthSetting::Simple { auth_mapping } => {
                let req_user = req.metadata().get("user").and_then(|u| u.to_str().ok());
                let req_pass = req.metadata().get("pass").and_then(|u| u.to_str().ok());
                if req_user.is_none() || req_pass.is_none() {
                    return Err(Status::unauthenticated("Missing auth header 'user' and/or 'pass'"));
                }
                let userpass = format!("{}:{}", req_user.unwrap(), req_pass.unwrap());
                match auth_mapping.get(&userpass) {
                    None => Err(Status::unauthenticated("Invalid 'user' & 'pass'")),
                    Some(roles) if roles.iter().any(|r| allowed_roles.contains(r)) => Ok(()),
                    _ => Err(Status::permission_denied("method not allowed for user")),
                }
            },
        }
    }
}

//--- SurveyService methods
#[tonic::async_trait]
impl SurveyService for SurveyApiServer {
    async fn list_surveys(&self, request: Request<api::ListSurveysRequest>) -> Result<Response<api::ListSurveysResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::USER];
        self.check_auth(&request, ROLES)?;

        println!("blub");

        Ok(Response::new(api::ListSurveysResponse { surveys: Vec::new() }))
    }

    async fn create_survey(&self, request: Request<api::CreateSurveyRequest>) -> Result<Response<api::CreateSurveyResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::ADMIN];
        self.check_auth(&request, ROLES)?;

        todo!()
    }

    async fn get_survey(&self, request: Request<api::GetSurveyRequest>) -> Result<Response<api::GetSurveyResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::USER];
        self.check_auth(&request, ROLES)?;

        todo!()
    }

    async fn delete_survey(&self, request: Request<api::DeleteSurveyRequest>) -> Result<Response<api::DeleteSurveyResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::ADMIN];
        self.check_auth(&request, ROLES)?;

        todo!()
    }

    async fn set_survey_active(&self, request: Request<api::SetSurveyActiveRequest>) -> Result<Response<api::SetSurveyActiveResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::ADMIN];
        self.check_auth(&request, ROLES)?;

        todo!()
    }
}

//--- SurveyResultsService
#[tonic::async_trait]
impl SurveyResultsService for SurveyApiServer {
    async fn get_results(&self, request: Request<api::GetResultsRequest>) -> Result<Response<api::GetResultsResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::USER];
        self.check_auth(&request, ROLES)?;

        todo!()
    }

    async fn add_result(&self, request: Request<api::AddResultRequest>) -> Result<Response<api::AddResultResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::USER];
        self.check_auth(&request, ROLES)?;

        todo!()
    }

    async fn delete_results(&self, request: Request<api::DeleteResultsRequest>) -> Result<Response<api::DeleteResultsResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::ADMIN];
        self.check_auth(&request, ROLES)?;

        todo!()
    }
}

//--- SurveyDataService
#[tonic::async_trait]
impl SurveyDataService for SurveyApiServer {
    async fn get_survey_summary(&self, request: Request<api::GetSurveySummaryRequest>) -> Result<Response<api::GetSurveySummaryResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::USER];
        self.check_auth(&request, ROLES)?;

        todo!()
    }

    async fn get_highscore(&self, request: Request<api::GetHighscoreRequest>) -> Result<Response<api::GetHighscoreResponse>, Status> {
        const ROLES: &[ROLES] = &[ROLES::USER];
        self.check_auth(&request, ROLES)?;

        todo!()
    }
}
