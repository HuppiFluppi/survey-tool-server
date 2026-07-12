use models::*;
use std::error::Error;
use std::fmt::{Display, Formatter};

pub struct SurveyPersistenceClient {}

impl SurveyPersistenceClient {
    pub async fn new() -> Self {
        Self {}
    }

    async fn save_survey(&self, survey: Vec<u8>) -> Result<String, PersistenceError> {
        todo!()
    }
    async fn get_survey(&self, id: String) -> Result<Vec<u8>, PersistenceError> {
        todo!()
    }
    async fn get_survey_summary(&self, id: String) -> Result<SurveySummary, PersistenceError> {
        todo!()
    }

    async fn list_surveys(&self, active: bool, survey_type: SurveyType) -> Result<Vec<SurveySummary>, PersistenceError> {
        todo!()
    }
    async fn delete_survey(&self, id: String) -> Result<(), PersistenceError> {
        todo!()
    }
    async fn set_survey_state(&self, id: String, active: bool) -> Result<(), PersistenceError> {
        todo!()
    }

    async fn save_result(&self, id: String, result: SurveyResult) -> Result<(), PersistenceError> {
        todo!()
    }
    async fn get_results(&self, id: String) -> Result<Vec<SurveyResult>, PersistenceError> {
        todo!()
    }
    async fn delete_results(&self, id: String) -> Result<(), PersistenceError> {
        todo!()
    }

    async fn get_highscore(&self, id: String, limit: u32) -> Result<Vec<HighscoreEntry>, PersistenceError> {
        todo!()
    }
}

#[derive(Debug)]
enum PersistenceError {
    NotFound,
}

impl Display for PersistenceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl Error for PersistenceError {}

pub mod models {
    use std::collections::HashMap;

    pub enum SurveyType {
        Survey,
        Quiz,
    }

    pub struct SurveySummary {
        pub title: String,
        pub description: String,
        pub survey_type: SurveyType,
        pub active: bool,
        pub page_count: u32,
        pub question_count: u32,
        pub submit_count: u32,
        pub conditionals: bool,
        pub first_submit_time: String,
        pub last_submit_time: String,
        pub min_score: Option<i32>,
        pub max_score: Option<i32>,
        pub avg_score: Option<f32>,
    }

    pub struct SurveyResult {
        pub origin: String,
        pub start_time: String,
        pub end_time: String,
        pub user: Option<String>,
        pub score: Option<f32>,
        pub answered_pages: u32,
        pub answered_questions: u32,
        pub answers: Vec<QuestionAnswer>,
    }

    pub struct HighscoreEntry {
        pub name: String,
        pub score: i32,
        pub time: String,
    }

    pub struct QuestionAnswer {
        pub question_id: String,
        pub question_title: String,
        pub is_answered: bool,
        pub answer: Answer,
    }

    pub enum Answer {
        Data(String),
        Choice(Vec<String>),
        Text(String),
        Rating(i32),
        Likert(HashMap<String, String>),
        Datetime(String),
        Slider(f32, f32),
    }
}
