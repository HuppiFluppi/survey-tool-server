//! gRPC transport for the survey tool server.
//!
//! Implements the three generated tonic services on [`SurveyApiServer`], enforces
//! the role-based auth model per method and converts between the persistence
//! domain model and the wire types (including [`PersistenceError`] -> [`Status`]).

// make the generated proto/grpc code available to this library
mod grpc_survey_api {
    tonic::include_proto!("survey.v1");
}

use crate::grpc::grpc_survey_api as api;
use crate::grpc::grpc_survey_api::survey_data_service_server::{SurveyDataService, SurveyDataServiceServer};
use crate::grpc::grpc_survey_api::survey_results_service_server::{SurveyResultsService, SurveyResultsServiceServer};
use crate::grpc::grpc_survey_api::survey_service_server::{SurveyService, SurveyServiceServer};
use crate::shared::persistence::models as persistence;
use crate::shared::persistence::{PersistenceError, SurveyPersistenceClient};
use crate::shared::server::{AuthSetting, TlsSetting, Roles};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tonic::{transport::Server, Request, Response, Status};

/// gRPC server holding the shared persistence backend and the active auth policy.
pub struct SurveyApiServer {
    persistence: Arc<dyn SurveyPersistenceClient>,
    auth_setting: AuthSetting,
}

impl SurveyApiServer {
    /// Serve all three services on `address` with authentication and TLS disabled.
    pub async fn serve(address: SocketAddr, persistence: Arc<dyn SurveyPersistenceClient>) -> Result<(), tonic::transport::Error> {
        let server = Arc::new(SurveyApiServer { persistence, auth_setting: AuthSetting::None });

        Server::builder()
            .add_service(SurveyServiceServer::from_arc(server.clone()))
            .add_service(SurveyDataServiceServer::from_arc(server.clone()))
            .add_service(SurveyResultsServiceServer::from_arc(server.clone()))
            .serve(address)
            .await?;

        Ok(())
    }

    /// Serve all three services on `address` with the given auth and TLS configuration.
    pub async fn serve_with_config(
        address: SocketAddr,
        persistence: Arc<dyn SurveyPersistenceClient>,
        auth: AuthSetting,
        tls: TlsSetting,
    ) -> Result<(), tonic::transport::Error> {
        let mut tonic_server = Server::builder();

        let api_server = Arc::new(SurveyApiServer { persistence, auth_setting: auth });
        match tls {
            TlsSetting::None => {},
            TlsSetting::Pem { cert, key } => {
                let tls_config = tonic::transport::ServerTlsConfig::new().identity(tonic::transport::Identity::from_pem(cert, key));
                tonic_server = tonic_server.tls_config(tls_config)?;
            },
        };

        tonic_server
            .add_service(SurveyServiceServer::from_arc(api_server.clone()))
            .add_service(SurveyDataServiceServer::from_arc(api_server.clone()))
            .add_service(SurveyResultsServiceServer::from_arc(api_server.clone()))
            .serve(address)
            .await?;

        Ok(())
    }

    /// Authorize a request against `allowed_roles`.
    ///
    /// Returns `Unauthenticated` when credentials are missing or unknown and
    /// `PermissionDenied` when the caller holds none of the allowed roles.
    /// Always succeeds when auth is disabled.
    fn check_auth<T>(&self, req: &Request<T>, allowed_roles: &[Roles]) -> Result<(), Status> {
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
    /// List surveys (requires `User`), translating the optional wire type filter to the domain type.
    async fn list_surveys(&self, request: Request<api::ListSurveysRequest>) -> Result<Response<api::ListSurveysResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::User];
        self.check_auth(&request, ROLES)?;

        let active = request.get_ref().active;
        let survey_type = request.get_ref().r#type.and_then(|v| api::SurveyType::try_from(v).ok()).and_then(|v| match v {
            api::SurveyType::Unspecified => None,
            api::SurveyType::Survey => Some(persistence::SurveyType::Survey),
            api::SurveyType::Quiz => Some(persistence::SurveyType::Quiz),
        });

        match self.persistence.list_surveys(active, survey_type).await {
            Ok(sl) => Ok(Response::new(api::ListSurveysResponse { surveys: sl.into_iter().map(api::SurveySummary::try_from).collect::<Result<Vec<_>, _>>()? })),
            Err(e) => Err(Status::from(e)),
        }
    }

    /// Store an uploaded survey bundle and return its id (requires `Admin`).
    async fn create_survey(&self, request: Request<api::CreateSurveyRequest>) -> Result<Response<api::CreateSurveyResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::Admin];
        self.check_auth(&request, ROLES)?;

        let survey = request.into_inner().zip_content;
        let survey_id = self.persistence.save_survey(survey).await?;
        Ok(Response::new(api::CreateSurveyResponse { survey_id }))
    }

    /// Download a survey bundle.
    ///
    /// `User` may fetch active surveys; inactive surveys require `Admin` and are
    /// otherwise reported as `PermissionDenied` to avoid leaking their existence.
    async fn get_survey(&self, request: Request<api::GetSurveyRequest>) -> Result<Response<api::GetSurveyResponse>, Status> {
        const ROLES_INACTIVE: &[Roles] = &[Roles::Admin];
        const ROLES_ACTIVE: &[Roles] = &[Roles::User];

        // if caller doesn't have the rights for inactive surveys, return error when survey is inactive
        if self.check_auth(&request, ROLES_INACTIVE).is_err() && !self.persistence.get_survey_active(&request.get_ref().survey_id).await? {
            return Err(Status::permission_denied("survey inactive"));
        }
        // check active survey rights
        self.check_auth(&request, ROLES_ACTIVE)?;

        let zip_content = self.persistence.get_survey(&request.get_ref().survey_id).await?;
        Ok(Response::new(api::GetSurveyResponse { zip_content }))
    }

    /// Delete a survey and its data (requires `Admin`).
    async fn delete_survey(&self, request: Request<api::DeleteSurveyRequest>) -> Result<Response<api::DeleteSurveyResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::Admin];
        self.check_auth(&request, ROLES)?;

        self.persistence.delete_survey(&request.get_ref().survey_id).await?;
        Ok(Response::new(api::DeleteSurveyResponse {}))
    }

    /// Toggle a survey's active flag (requires `Admin`).
    async fn set_survey_active(&self, request: Request<api::SetSurveyActiveRequest>) -> Result<Response<api::SetSurveyActiveResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::Admin];
        self.check_auth(&request, ROLES)?;

        self.persistence.set_survey_state(&request.get_ref().survey_id, request.get_ref().active).await?;
        Ok(Response::new(api::SetSurveyActiveResponse {}))
    }
}

//--- SurveyResultsService
#[tonic::async_trait]
impl SurveyResultsService for SurveyApiServer {
    /// Return all results of a survey (requires `Admin`).
    async fn get_results(&self, request: Request<api::GetResultsRequest>) -> Result<Response<api::GetResultsResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::Admin];
        self.check_auth(&request, ROLES)?;

        let results = self.persistence.get_results(&request.get_ref().survey_id).await?;
        Ok(Response::new(api::GetResultsResponse { results: results.into_iter().map(api::SurveyResult::try_from).collect::<Result<Vec<_>, _>>()? }))
    }

    /// Submit a result (requires `User`).
    ///
    /// Rejects a missing payload with `InvalidArgument` and an inactive survey with `PermissionDenied`.
    async fn add_result(&self, request: Request<api::AddResultRequest>) -> Result<Response<api::AddResultResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::User];
        self.check_auth(&request, ROLES)?;

        let req = request.into_inner();
        //check result set
        let Some(result) = req.result else {
            return Err(Status::invalid_argument("missing result"));
        };

        //check survey active
        if !self.persistence.get_survey_active(&req.survey_id).await? {
            return Err(Status::permission_denied("survey inactive"));
        }

        self.persistence.save_result(&req.survey_id, result.try_into()?).await?;

        Ok(Response::new(api::AddResultResponse {}))
    }

    /// Delete all results of a survey (requires `Admin`).
    async fn delete_results(&self, request: Request<api::DeleteResultsRequest>) -> Result<Response<api::DeleteResultsResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::Admin];
        self.check_auth(&request, ROLES)?;

        self.persistence.delete_results(&request.get_ref().survey_id).await?;

        Ok(Response::new(api::DeleteResultsResponse {}))
    }
}

//--- SurveyDataService
#[tonic::async_trait]
impl SurveyDataService for SurveyApiServer {
    /// Return the aggregate summary of a survey (requires `User`).
    async fn get_survey_summary(&self, request: Request<api::GetSurveySummaryRequest>) -> Result<Response<api::GetSurveySummaryResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::User];
        self.check_auth(&request, ROLES)?;

        let summary = self.persistence.get_survey_summary(&request.get_ref().survey_id).await?;
        Ok(Response::new(api::GetSurveySummaryResponse { summary: Some(summary.try_into()?) }))
    }

    /// Return the highscore of a quiz (requires `User`).
    ///
    /// Returns `NotFound` for unknown surveys; `limit` defaults to 10 when unset.
    async fn get_highscore(&self, request: Request<api::GetHighscoreRequest>) -> Result<Response<api::GetHighscoreResponse>, Status> {
        const ROLES: &[Roles] = &[Roles::User];
        self.check_auth(&request, ROLES)?;

        if !self.persistence.survey_exist(&request.get_ref().survey_id).await? {
            return Err(Status::not_found("survey not found"));
        }

        let highscore = self.persistence.get_highscore(&request.get_ref().survey_id, request.get_ref().limit.unwrap_or(10)).await?;
        Ok(Response::new(api::GetHighscoreResponse { entries: highscore.into_iter().map(api::HighscoreEntry::try_from).collect::<Result<Vec<_>, _>>()? }))
    }
}

//--- Persistence <> gRPC model conversions

impl From<PersistenceError> for Status {
    /// Map persistence failures onto gRPC status codes (`NotFound` is preserved, the rest become `internal`).
    fn from(value: PersistenceError) -> Self {
        match value {
            PersistenceError::Generic(s) => Status::internal(s),
            PersistenceError::NotFound(s) => Status::not_found(s),
            //PersistenceError::NotAFile(s) => Status::internal(s),
            PersistenceError::NotADir(s) => Status::internal(s),
            PersistenceError::NotWriteable(s) => Status::internal(s),
            PersistenceError::DbError(s) => Status::internal(s),
            PersistenceError::ZipFileError(s) => Status::internal(s),
            PersistenceError::StorageError(s) => Status::internal(s),
            PersistenceError::SurveyConfigError(s) => Status::internal(s),
        }
    }
}

impl TryFrom<persistence::SurveySummary> for api::SurveySummary {
    type Error = Status;

    fn try_from(value: persistence::SurveySummary) -> Result<Self, Self::Error> {
        let first_time = match value.first_submit_time {
            None => None,
            Some(t) => Some(prost_types::Timestamp::from_str(&t).map_err(|e| Status::internal(e.to_string()))?),
        };
        let last_time = match value.last_submit_time {
            None => None,
            Some(t) => Some(prost_types::Timestamp::from_str(&t).map_err(|e| Status::internal(e.to_string()))?),
        };

        Ok(api::SurveySummary {
            id: value.id,
            title: value.title,
            description: value.description,
            survey_type: match value.survey_type {
                persistence::SurveyType::Survey => api::SurveyType::Survey as i32,
                persistence::SurveyType::Quiz => api::SurveyType::Quiz as i32,
            },
            active: value.active,
            page_count: value.page_count,
            question_count: value.question_count,
            submit_count: value.submit_count,
            conditionals: value.conditionals,
            first_submit_time: first_time,
            last_submit_time: last_time,
            min_score: value.min_score,
            max_score: value.max_score,
            avg_score: value.avg_score,
        })
    }
}

impl TryFrom<persistence::SurveyResult> for api::SurveyResult {
    type Error = Status;

    fn try_from(value: persistence::SurveyResult) -> Result<Self, Self::Error> {
        let start_time = prost_types::Timestamp::from_str(&value.start_time).map_err(|e| Status::internal(e.to_string()))?;
        let end_time = prost_types::Timestamp::from_str(&value.end_time).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Self {
            origin: value.origin,
            start_time: Some(start_time),
            end_time: Some(end_time),
            user: value.user,
            score: value.score,
            answered_pages: value.answered_pages,
            answered_questions: value.answered_questions,
            answers: value.answers.into_iter().map(api::QuestionAnswer::from).collect(),
        })
    }
}

impl From<persistence::QuestionAnswer> for api::QuestionAnswer {
    fn from(value: persistence::QuestionAnswer) -> Self {
        Self {
            question_id: value.question_id,
            question_title: value.question_title,
            question_type: api::QuestionType::from(value.question_type) as i32,
            is_answered: value.is_answered,
            answer: value.answer.map(api::question_answer::Answer::from),
        }
    }
}

impl From<persistence::QuestionType> for api::QuestionType {
    fn from(value: persistence::QuestionType) -> Self {
        match value {
            persistence::QuestionType::Text => api::QuestionType::Text,
            persistence::QuestionType::Choice => api::QuestionType::Choice,
            persistence::QuestionType::Data => api::QuestionType::Data,
            persistence::QuestionType::Rating => api::QuestionType::Rating,
            persistence::QuestionType::Likert => api::QuestionType::Likert,
            persistence::QuestionType::Information => api::QuestionType::Unspecified, //information is never in an answer
            persistence::QuestionType::DateTime => api::QuestionType::Datetime,
            persistence::QuestionType::Slider => api::QuestionType::Slider,
        }
    }
}

impl From<persistence::Answer> for api::question_answer::Answer {
    fn from(value: persistence::Answer) -> Self {
        match value {
            persistence::Answer::Data(a) => Self::StringAnswer(a),
            persistence::Answer::Choice(a) => Self::ListAnswer(api::StringList { values: a }),
            persistence::Answer::Text(a) => Self::StringAnswer(a),
            persistence::Answer::Rating(a) => Self::IntAnswer(a),
            persistence::Answer::Likert(a) => Self::MapAnswer(api::StringMap { entries: a }),
            persistence::Answer::Datetime(a) => Self::StringAnswer(a),
            persistence::Answer::Slider(a1, a2) => Self::RangeAnswer(api::RangeAnswer { first: a1, second: a2 }),
        }
    }
}

impl TryFrom<api::SurveyResult> for persistence::SurveyResult {
    type Error = Status;

    fn try_from(value: api::SurveyResult) -> Result<Self, Self::Error> {
        let start_time = value.start_time.map(|t| t.to_string()).ok_or(Status::invalid_argument("start_time missing"))?;
        let end_time = value.end_time.map(|t| t.to_string()).ok_or(Status::invalid_argument("end_time missing"))?;

        Ok(Self {
            origin: value.origin,
            start_time,
            end_time,
            user: value.user,
            score: value.score,
            answered_pages: value.answered_pages,
            answered_questions: value.answered_questions,
            answers: value.answers.into_iter().map(persistence::QuestionAnswer::try_from).collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl TryFrom<api::QuestionAnswer> for persistence::QuestionAnswer {
    type Error = Status;

    fn try_from(value: api::QuestionAnswer) -> Result<Self, Self::Error> {
        let question_type = value.question_type();
        Ok(Self {
            question_id: value.question_id,
            question_title: value.question_title,
            question_type: question_type.try_into()?,
            is_answered: value.is_answered,
            answer: map_proto_answer_to_persistence(question_type, value.answer)?,
        })
    }
}

impl TryFrom<api::QuestionType> for persistence::QuestionType {
    type Error = Status;

    fn try_from(value: api::QuestionType) -> Result<Self, Self::Error> {
        match value {
            api::QuestionType::Unspecified => Err(Status::invalid_argument("question_type unspecified")),
            api::QuestionType::Data => Ok(persistence::QuestionType::Data),
            api::QuestionType::Choice => Ok(persistence::QuestionType::Choice),
            api::QuestionType::Text => Ok(persistence::QuestionType::Text),
            api::QuestionType::Rating => Ok(persistence::QuestionType::Rating),
            api::QuestionType::Likert => Ok(persistence::QuestionType::Likert),
            api::QuestionType::Datetime => Ok(persistence::QuestionType::DateTime),
            api::QuestionType::Slider => Ok(persistence::QuestionType::Slider),
        }
    }
}

/// Validate and convert a wire answer into the persistence [`Answer`](persistence::Answer).
///
/// Enforces that the answer variant matches the declared question type; any
/// mismatch (or an answer under an unspecified type) yields `InvalidArgument`.
fn map_proto_answer_to_persistence(qtype: api::QuestionType, ans: Option<api::question_answer::Answer>) -> Result<Option<persistence::Answer>, Status> {
    match (ans, qtype) {
        (None, _) => Ok(None),
        (Some(_), api::QuestionType::Unspecified) => Err(Status::invalid_argument("question_type invalid")),
        (Some(api::question_answer::Answer::StringAnswer(a)), api::QuestionType::Data) => Ok(Some(persistence::Answer::Data(a))),
        (Some(api::question_answer::Answer::ListAnswer(a)), api::QuestionType::Choice) => Ok(Some(persistence::Answer::Choice(a.values))),
        (Some(api::question_answer::Answer::StringAnswer(a)), api::QuestionType::Text) => Ok(Some(persistence::Answer::Text(a))),
        (Some(api::question_answer::Answer::IntAnswer(a)), api::QuestionType::Rating) => Ok(Some(persistence::Answer::Rating(a))),
        (Some(api::question_answer::Answer::MapAnswer(a)), api::QuestionType::Likert) => Ok(Some(persistence::Answer::Likert(a.entries))),
        (Some(api::question_answer::Answer::StringAnswer(a)), api::QuestionType::Datetime) => Ok(Some(persistence::Answer::Datetime(a))),
        (Some(api::question_answer::Answer::RangeAnswer(a)), api::QuestionType::Slider) => Ok(Some(persistence::Answer::Slider(a.first, a.second))),
        _ => Err(Status::invalid_argument("invalid question_type and answer field combination")),
    }
}

impl TryFrom<persistence::HighscoreEntry> for api::HighscoreEntry {
    type Error = Status;

    fn try_from(value: persistence::HighscoreEntry) -> Result<Self, Self::Error> {
        let time = prost_types::Timestamp::from_str(&value.time).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Self { name: value.name, score: value.score, time: Some(time) })
    }
}
