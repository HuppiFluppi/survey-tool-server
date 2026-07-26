//! Transport-agnostic building blocks shared across the server.
//!
//! [`server`] holds runtime configuration (auth, TLS, roles); [`persistence`]
//! defines the [`persistence::SurveyPersistenceClient`] backend trait and the
//! domain model that every transport and backend converts to and from.

/// Server runtime configuration: role model, authentication and TLS settings.
pub mod server {
    use strum::EnumString;

    /// Access role a caller can hold. Parsed from the CLI auth config via [`std::str::FromStr`].
    #[derive(PartialEq, Debug, Clone, EnumString)]
    pub enum Roles {
        Admin,
        User,
    }

    /// Authentication mode. `Simple` maps `"user:pass"` keys to the roles they grant.
    #[derive(Debug, Clone)]
    pub enum AuthSetting {
        None,
        Simple { auth_mapping: std::collections::HashMap<String, Vec<Roles>> },
    }

    impl AuthSetting {
        /// Build a [`AuthSetting::Simple`] from `(user, pass, roles)` tuples, keying by `"user:pass"`.
        pub fn simple(entries: Vec<(String, String, Vec<Roles>)>) -> Self {
            Self::Simple { auth_mapping: entries.into_iter().map(|t| (format!("{}:{}", t.0, t.1), t.2)).collect() }
        }
    }

    /// TLS mode. `Pem` carries the certificate and key as in-memory PEM strings.
    #[derive(Debug, Clone)]
    pub enum TlsSetting {
        None,
        Pem { cert: String, key: String },
    }

    impl TlsSetting {
        /// TLS disabled.
        pub fn off() -> Self {
            TlsSetting::None
        }
        /// TLS enabled from PEM-encoded certificate and key strings.
        pub fn pem(cert: String, key: String) -> Self {
            TlsSetting::Pem { cert, key }
        }
    }
}

/// Persistence abstraction: the backend trait, its error type and the domain model.
pub mod persistence {
    use models::*;
    use std::error::Error;
    use std::fmt::{Display, Formatter};

    /// Storage backend contract implemented by each persistence variant (local, aws).
    ///
    /// Object-safe and `Send + Sync` so it can be shared as `Arc<dyn ..>` across transports.
    #[tonic::async_trait]
    pub trait SurveyPersistenceClient: Send + Sync + 'static {
        /// Store a survey ZIP bundle, index its config metadata and return the new id.
        async fn save_survey(&self, survey: Vec<u8>) -> Result<String, PersistenceError>;
        /// Return the raw ZIP bundle for a survey id.
        async fn get_survey(&self, id: &str) -> Result<Vec<u8>, PersistenceError>;
        /// Return aggregate statistics for a survey.
        async fn get_survey_summary(&self, id: &str) -> Result<SurveySummary, PersistenceError>;
        /// Report whether a survey with the given id exists.
        async fn survey_exist(&self, id: &str) -> Result<bool, PersistenceError>;

        /// List survey summaries, optionally filtered by active state and/or type.
        async fn list_surveys(&self, active: Option<bool>, survey_type: Option<SurveyType>) -> Result<Vec<SurveySummary>, PersistenceError>;
        /// Delete a survey along with its bundle and all associated results.
        async fn delete_survey(&self, id: &str) -> Result<(), PersistenceError>;
        /// Set a survey's active flag.
        async fn set_survey_state(&self, id: &str, active: bool) -> Result<(), PersistenceError>;
        /// Return a survey's active flag.
        async fn get_survey_active(&self, id: &str) -> Result<bool, PersistenceError>;

        /// Persist one participant's result and its answers.
        async fn save_result(&self, id: &str, result: SurveyResult) -> Result<(), PersistenceError>;
        /// Return all results (with answers) for a survey.
        async fn get_results(&self, id: &str) -> Result<Vec<SurveyResult>, PersistenceError>;
        /// Delete all results of a survey while keeping the survey.
        async fn delete_results(&self, id: &str) -> Result<(), PersistenceError>;

        /// Return the top `limit` scoring entries for a quiz, best score first.
        async fn get_highscore(&self, id: &str, limit: u32) -> Result<Vec<HighscoreEntry>, PersistenceError>;
    }

    /// Backend-agnostic failure surfaced by [`SurveyPersistenceClient`] and mapped to a transport error.
    #[derive(Debug)]
    pub(crate) enum PersistenceError {
        Generic(String),
        NotFound(String),
        //NotAFile(String),
        NotADir(String),
        NotWriteable(String),
        DbError(String),
        ZipFileError(String),
        StorageError(String),
        SurveyConfigError(String),
    }

    impl Display for PersistenceError {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                PersistenceError::NotFound(p) => write!(f, "Element '{p}' not found"),
                //PersistenceError::NotAFile(p) => write!(f, "Element '{p}' is not a file"),
                PersistenceError::NotADir(p) => write!(f, "Element '{p}' is not a directory"),
                PersistenceError::NotWriteable(p) => write!(f, "Element '{p}' is not writable"),
                PersistenceError::Generic(e) => write!(f, "Error: {e}"),
                PersistenceError::DbError(e) => write!(f, "DB Error: {e}"),
                PersistenceError::ZipFileError(e) => write!(f, "Zip File Error: {e}"),
                PersistenceError::StorageError(e) => write!(f, "Storage Error: {e}"),
                PersistenceError::SurveyConfigError(e) => write!(f, "Error with survey config: {e}"),
            }
        }
    }

    impl Error for PersistenceError {}

    /// Domain model shared by all backends and transports.
    ///
    /// `QuestionType` and `SurveyType` are re-exported from the `survey-tool-cli`
    /// crate so the config authored there and the stored results stay in sync.
    /// Timestamp fields are RFC 3339 strings.
    pub mod models {
        use std::collections::HashMap;
        pub use survey_tool_cli::SurveyContentType as QuestionType;
        pub use survey_tool_cli::SurveyType;

        /// Server-computed aggregates for one survey (counts, submit times, score stats).
        pub struct SurveySummary {
            pub id: String,
            pub title: String,
            pub description: String,
            pub survey_type: SurveyType,
            pub active: bool,
            pub page_count: u32,
            pub question_count: u32,
            pub submit_count: u32,
            pub conditionals: bool,
            pub first_submit_time: Option<String>, // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
            pub last_submit_time: Option<String>,  // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
            pub min_score: Option<i32>,
            pub max_score: Option<i32>,
            pub avg_score: Option<f32>,
        }

        /// One participant's completed run: timing, origin, optional user/score and per-question answers.
        pub struct SurveyResult {
            pub origin: String,
            pub start_time: String, // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
            pub end_time: String,   // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
            pub user: Option<String>,
            pub score: Option<i32>,
            pub answered_pages: u32,
            pub answered_questions: u32,
            pub answers: Vec<QuestionAnswer>,
        }

        /// A single ranked quiz entry (participant name, score, submit time).
        pub struct HighscoreEntry {
            pub name: String,
            pub score: i32,
            pub time: String, // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
        }

        /// One question within a result. `answer` is `None` when the question was skipped.
        pub struct QuestionAnswer {
            pub question_id: String,
            pub question_title: String,
            pub question_type: QuestionType,
            pub is_answered: bool,
            pub answer: Option<Answer>,
        }

        /// Typed answer value; the active variant must match the question's [`QuestionType`].
        pub enum Answer {
            Data(String),
            Choice(Vec<String>),
            Text(String),
            Rating(i32),
            Likert(HashMap<String, String>),
            Datetime(String), // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
            Slider(f32, Option<f32>),
        }
    }
}
