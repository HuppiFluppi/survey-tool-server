pub mod server {
    use strum::EnumString;

    #[derive(PartialEq, Debug, Clone, EnumString)]
    pub enum Roles {
        Admin,
        User,
    }

    #[derive(Debug, Clone)]
    pub enum AuthSetting {
        None,
        Simple { auth_mapping: std::collections::HashMap<String, Vec<Roles>> },
    }

    impl AuthSetting {
        pub fn simple(entries: Vec<(String, String, Vec<Roles>)>) -> Self {
            Self::Simple { auth_mapping: entries.into_iter().map(|t| (format!("{}:{}", t.0, t.1), t.2)).collect() }
        }
    }

    #[derive(Debug, Clone)]
    pub enum TlsSetting {
        None,
        Pem { cert: String, key: String },
    }

    impl TlsSetting {
        pub fn off() -> Self {
            TlsSetting::None
        }
        pub fn pem(cert: String, key: String) -> Self {
            TlsSetting::Pem { cert, key }
        }
    }
}

pub mod persistence {
    use models::*;
    use std::error::Error;
    use std::fmt::{Display, Formatter};

    #[tonic::async_trait]
    pub trait SurveyPersistenceClient: Send + Sync + 'static {
        async fn save_survey(&self, survey: Vec<u8>) -> Result<String, PersistenceError>;
        async fn get_survey(&self, id: &str) -> Result<Vec<u8>, PersistenceError>;
        async fn get_survey_summary(&self, id: &str) -> Result<SurveySummary, PersistenceError>;
        async fn survey_exist(&self, id: &str) -> Result<bool, PersistenceError>;

        async fn list_surveys(&self, active: Option<bool>, survey_type: Option<SurveyType>) -> Result<Vec<SurveySummary>, PersistenceError>;
        async fn delete_survey(&self, id: &str) -> Result<(), PersistenceError>;
        async fn set_survey_state(&self, id: &str, active: bool) -> Result<(), PersistenceError>;
        async fn get_survey_active(&self, id: &str) -> Result<bool, PersistenceError>;

        async fn save_result(&self, id: &str, result: SurveyResult) -> Result<(), PersistenceError>;
        async fn get_results(&self, id: &str) -> Result<Vec<SurveyResult>, PersistenceError>;
        async fn delete_results(&self, id: &str) -> Result<(), PersistenceError>;

        async fn get_highscore(&self, id: &str, limit: u32) -> Result<Vec<HighscoreEntry>, PersistenceError>;
    }

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

    pub mod models {
        use std::collections::HashMap;
        pub use survey_tool_cli::SurveyContentType as QuestionType;
        pub use survey_tool_cli::SurveyType;

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

        pub struct HighscoreEntry {
            pub name: String,
            pub score: i32,
            pub time: String, // format RFC 3339: {year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z
        }

        pub struct QuestionAnswer {
            pub question_id: String,
            pub question_title: String,
            pub question_type: QuestionType,
            pub is_answered: bool,
            pub answer: Option<Answer>,
        }

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
