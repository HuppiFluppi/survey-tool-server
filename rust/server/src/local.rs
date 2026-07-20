//! Local persistence module

use crate::shared::persistence::models::{Answer, HighscoreEntry, QuestionAnswer, SurveyResult, SurveySummary, SurveyType};
use crate::shared::persistence::{PersistenceError, SurveyPersistenceClient};
use rusqlite::{Connection, Row, params};
use std::collections::HashMap;
use std::io;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::fs;
use tokio::fs::File;
use uuid::Uuid;
use zip::ZipArchive;

pub async fn new(storage_folder: &str, db_folder: &str) -> Result<Arc<dyn SurveyPersistenceClient>, PersistenceError> {
    //check storage folder exists and is writable directory
    let storage_path = Path::new(storage_folder);
    if !storage_path.exists() {
        return Err(PersistenceError::NotFound(storage_folder.to_string()));
    }
    if !storage_path.is_dir() {
        return Err(PersistenceError::NotADir(storage_folder.to_string()));
    }
    let mut storage_tmp_path = PathBuf::from(storage_path);
    storage_tmp_path.push("tmp.txt");
    match File::create(&storage_tmp_path).await {
        Ok(f) => {
            drop(f);
            let _ = fs::remove_file(storage_tmp_path).await;
        },
        Err(_) => return Err(PersistenceError::NotWriteable(storage_folder.to_string())),
    }

    //check db folder exists and init db
    let mut db_path = PathBuf::from(db_folder);
    if !db_path.exists() {
        return Err(PersistenceError::NotFound(db_folder.to_string()));
    }
    if !db_path.is_dir() {
        return Err(PersistenceError::NotADir(db_folder.to_string()));
    }
    db_path.push("survey-tool-server.sqlite");

    Ok(Arc::new(LocalSurveyPersistenceClient { storage_path: storage_folder.to_string(), db_conn: Mutex::new(init_db(&db_path)?) }))
}

const DB_INIT: &str = "CREATE TABLE IF NOT EXISTS surveys (
                            id TEXT NOT NULL PRIMARY KEY,
                            name TEXT NOT NULL,
                            desc TEXT NOT NULL,
                            type TEXT NOT NULL,
                            active INTEGER NOT NULL,
                            page_count INTEGER NOT NULL,
                            question_count INTEGER NOT NULL,
                            conditionals INTEGER NOT NULL
                        );
                        CREATE TABLE IF NOT EXISTS results (
                            id INTEGER NOT NULL PRIMARY KEY,
                            survey_id TEXT NOT NULL,
                            origin TEXT NOT NULL,
                            start_time TEXT NOT NULL,
                            end_time TEXT NOT NULL,
                            answered_pages INTEGER NOT NULL,
                            answered_questions INTEGER NOT NULL,
                            user TEXT,
                            score INTEGER
                        );
                        CREATE INDEX IF NOT EXISTS result_surveyid_index ON results (survey_id);
                        CREATE TABLE IF NOT EXISTS answers (
                            id INTEGER PRIMARY KEY,
                            survey_id TEXT NOT NULL,
                            result_id INTEGER NOT NULL,
                            question_id TEXT NOT NULL,
                            question_title TEXT NOT NULL,
                            question_type TEXT NOT NULL,
                            answered INTEGER NOT NULL,
                            string_answer TEXT,
                            int_answer INTEGER,
                            string_vec_answer TEXT,
                            string_map_answer TEXT,
                            float_answer1 REAL,
                            float_answer2 REAL
                        );
                        CREATE INDEX IF NOT EXISTS answer_survey_result_index ON answers (survey_id, result_id);
                        ";

fn init_db(db_path: &Path) -> Result<Connection, PersistenceError> {
    let conn = Connection::open(db_path).map_err(db_err)?;
    conn.execute_batch(DB_INIT).map_err(db_err)?;
    Ok(conn)
}

#[derive(Debug)]
pub struct LocalSurveyPersistenceClient {
    storage_path: String,
    db_conn: Mutex<Connection>,
}

#[tonic::async_trait]
impl SurveyPersistenceClient for LocalSurveyPersistenceClient {
    async fn save_survey(&self, survey: Vec<u8>) -> Result<String, PersistenceError> {
        let survey_id = Uuid::new_v4().to_string();

        // extract survey information
        let config = {
            let cursor = io::Cursor::new(&survey);

            let mut archive = ZipArchive::new(cursor).map_err(|e| PersistenceError::ZipFileError(e.to_string()))?;
            let mut config_file = archive.by_name("survey_config.yaml").map_err(|e| PersistenceError::ZipFileError(e.to_string()))?;
            let mut file_content = String::new();
            config_file.read_to_string(&mut file_content).map_err(|e| PersistenceError::ZipFileError(e.to_string()))?;

            survey_tool_cli::load_config_from_string(&file_content).map_err(|e| PersistenceError::SurveyConfigError(e.to_string()))?
        };

        // save file to storage
        let path = Path::new(&self.storage_path).join(&survey_id);
        tokio::fs::write(path, survey).await.map_err(|e| PersistenceError::StorageError(e.to_string()))?;

        // insert survey into database
        let question_cnt: usize = config.pages.iter().map(|p| p.content.len()).sum();
        let conn = self.db_conn.lock().map_err(lock_err)?;
        conn.execute(
            "INSERT INTO surveys (id, name, desc, type, active, page_count, question_count, conditionals) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                survey_id,
                config.title,
                config.description,
                config.survey_type.to_string(),
                true,
                config.pages.len() as i64,
                question_cnt as i64,
                has_conditionals(&config)
            ],
        )
        .map_err(db_err)?;

        Ok(survey_id)
    }

    async fn get_survey(&self, id: &str) -> Result<Vec<u8>, PersistenceError> {
        let path = Path::new(&self.storage_path).join(id);
        tokio::fs::read(path).await.map_err(|e| match e.kind() {
            ErrorKind::NotFound => PersistenceError::NotFound(e.to_string()),
            _ => PersistenceError::StorageError(e.to_string()),
        })
    }

    async fn get_survey_summary(&self, id: &str) -> Result<SurveySummary, PersistenceError> {
        let conn = self.db_conn.lock().map_err(lock_err)?;

        conn.query_one(
            "SELECT s.id, s.name, s.desc, s.type, s.active, s.page_count, s.question_count, s.conditionals, \
                    COUNT(r.id) AS cnt, \
                    MIN(r.end_time) AS first_time, MAX(r.end_time) AS last_time, \
                    MIN(r.score) AS min_score, MAX(r.score) AS max_score, AVG(r.score) AS avg_score \
             FROM surveys s \
             LEFT JOIN results r ON r.survey_id = s.id \
             WHERE s.id = ?1 \
             GROUP BY s.id",
            [id],
            |row| {
                let survey_type: String = row.get("type")?;
                let survey_type = match survey_type.as_str() {
                    "quiz" => SurveyType::Quiz,
                    _ => SurveyType::Survey,
                };

                Ok(SurveySummary {
                    id: row.get("id")?,
                    title: row.get("name")?,
                    description: row.get("desc")?,
                    survey_type,
                    active: row.get("active")?,
                    page_count: row.get("page_count")?,
                    question_count: row.get("question_count")?,
                    conditionals: row.get("conditionals")?,
                    submit_count: row.get("cnt")?,
                    first_submit_time: row.get("first_time")?,
                    last_submit_time: row.get("last_time")?,
                    min_score: row.get::<_, Option<i32>>("min_score")?,
                    max_score: row.get::<_, Option<i32>>("max_score")?,
                    avg_score: row.get::<_, Option<f64>>("avg_score")?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::NotFound(id.to_string()),
            other => PersistenceError::DbError(other.to_string()),
        })
    }

    async fn list_surveys(&self, active: Option<bool>, survey_type: Option<SurveyType>) -> Result<Vec<SurveySummary>, PersistenceError> {
        let where_clause = match (active, survey_type) {
            (Some(a), Some(t)) => format!("WHERE s.active = {a} AND s.type = '{t}'"),
            (Some(a), None) => format!("WHERE s.active = {a}"),
            (None, Some(t)) => format!("WHERE s.type = '{t}'"),
            (None, None) => String::from(""),
        };

        let conn = self.db_conn.lock().map_err(lock_err)?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT s.id, s.name, s.desc, s.type, s.active, s.page_count, s.question_count, s.conditionals, \
                    COUNT(r.id) AS submit_cnt, \
                    MIN(r.end_time) AS first_submit, MAX(r.end_time) AS last_submit, \
                    MIN(r.score) AS min_score, MAX(r.score) AS max_score, AVG(r.score) AS avg_score \
                    FROM surveys s \
                    LEFT JOIN results r ON r.survey_id = s.id \
                    {where_clause} \
                    GROUP BY s.id",
            ))
            .map_err(db_err)?;

        let results = stmt
            .query_map([], |row| {
                let survey_type: String = row.get("type")?;
                let survey_type = match survey_type.as_str() {
                    "quiz" => SurveyType::Quiz,
                    _ => SurveyType::Survey,
                };

                Ok(SurveySummary {
                    id: row.get("id")?,
                    title: row.get("name")?,
                    description: row.get("desc")?,
                    survey_type,
                    active: row.get("active")?,
                    page_count: row.get("page_count")?,
                    question_count: row.get("question_count")?,
                    conditionals: row.get("conditionals")?,
                    submit_count: row.get("submit_cnt")?,
                    first_submit_time: row.get("first_submit")?,
                    last_submit_time: row.get("last_submit")?,
                    min_score: row.get::<_, Option<i32>>("min_score")?,
                    max_score: row.get::<_, Option<i32>>("max_score")?,
                    avg_score: row.get::<_, Option<f64>>("avg_score")?,
                })
            })
            .map_err(db_err)?
            .map(|r| r.map_err(db_err))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    async fn delete_survey(&self, id: &str) -> Result<(), PersistenceError> {
        //remove from db
        {
            let mut conn = self.db_conn.lock().map_err(lock_err)?;
            let tx = conn.transaction().map_err(db_err)?;

            tx.execute("DELETE FROM answers WHERE survey_id = ?1", [id]).map_err(db_err)?;
            tx.execute("DELETE FROM results WHERE survey_id = ?1", [id]).map_err(db_err)?;
            let res = tx.execute("DELETE FROM surveys WHERE id = ?1", [id]).map_err(db_err)?;
            if res == 0 {
                return Err(PersistenceError::NotFound("Not found in table".to_string()));
            }

            tx.commit().map_err(db_err)?;
        }

        //remove from filesystem
        fs::remove_file(Path::new(&self.storage_path).join(id)).await.map_err(|e| match e.kind() {
            ErrorKind::NotFound => PersistenceError::NotFound(e.to_string()),
            _ => PersistenceError::Generic(e.to_string()),
        })?;

        Ok(())
    }

    async fn set_survey_state(&self, id: &str, active: bool) -> Result<(), PersistenceError> {
        let conn = self.db_conn.lock().map_err(lock_err)?;

        let res = conn.execute("UPDATE surveys SET active = ?1 WHERE id = ?2", params![active, id]).map_err(db_err)?;
        if res == 0 {
            return Err(PersistenceError::NotFound("survey not found".to_string()));
        }

        Ok(())
    }

    async fn get_survey_active(&self, id: &str) -> Result<bool, PersistenceError> {
        let conn = self.db_conn.lock().map_err(lock_err)?;

        conn.query_one("SELECT active FROM surveys WHERE id = ?1", [id], |row| row.get::<_, bool>(0)).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::NotFound(id.to_string()),
            other => PersistenceError::DbError(other.to_string()),
        })
    }

    async fn save_result(&self, id: &str, result: SurveyResult) -> Result<(), PersistenceError> {
        let mut conn = self.db_conn.lock().map_err(lock_err)?;
        let tx = conn.transaction().map_err(db_err)?;

        tx.execute(
            "INSERT INTO results (survey_id, origin, start_time, end_time, answered_pages, answered_questions, user, score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, result.origin, result.start_time, result.end_time, result.answered_pages, result.answered_questions, result.user, result.score,],
        )
        .map_err(db_err)?;

        let result_id = tx.last_insert_rowid();

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO answers (survey_id, result_id, question_id, question_title, question_type, answered, \
                     string_answer, int_answer, string_vec_answer, string_map_answer, float_answer1, float_answer2) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                )
                .map_err(db_err)?;

            for qa in &result.answers {
                let enc = encode_answer(&qa.answer);
                stmt.execute(params![
                    id,
                    result_id,
                    qa.question_id,
                    qa.question_title,
                    enc.question_type,
                    qa.is_answered,
                    enc.string_answer,
                    enc.int_answer,
                    enc.string_vec_answer,
                    enc.string_map_answer,
                    enc.float_answer1,
                    enc.float_answer2,
                ])
                .map_err(db_err)?;
            }
        }

        tx.commit().map_err(db_err)?;

        Ok(())
    }

    async fn get_results(&self, id: &str) -> Result<Vec<SurveyResult>, PersistenceError> {
        let conn = self.db_conn.lock().map_err(lock_err)?;

        // Fetch every result together with all of its answers in a single query.
        // A LEFT JOIN keeps results that do not have any answers yet (their answer
        // columns come back as NULL). Ordering by the result id groups the rows of
        // one result together so they can be folded into a single SurveyResult.
        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.origin, r.start_time, r.end_time, r.answered_pages, r.answered_questions, r.user, r.score, \
                        a.question_id, a.question_title, a.question_type, a.answered, \
                        a.string_answer, a.int_answer, a.string_vec_answer, a.string_map_answer, a.float_answer1, a.float_answer2 \
                 FROM results r \
                 LEFT JOIN answers a ON a.survey_id = r.survey_id AND a.result_id = r.id \
                 WHERE r.survey_id = ?1 \
                 ORDER BY r.id, a.id",
            )
            .map_err(db_err)?;

        let mut rows = stmt.query([id]).map_err(db_err)?;

        let mut results: Vec<SurveyResult> = Vec::new();
        let mut current_id: Option<i64> = None;

        while let Some(row) = rows.next().map_err(db_err)? {
            let result_id: i64 = row.get("id").map_err(db_err)?;

            // Start a new SurveyResult whenever the result id changes.
            if current_id != Some(result_id) {
                current_id = Some(result_id);
                results.push(SurveyResult {
                    origin: row.get("origin").map_err(db_err)?,
                    start_time: row.get("start_time").map_err(db_err)?,
                    end_time: row.get("end_time").map_err(db_err)?,
                    user: row.get("user").map_err(db_err)?,
                    score: row.get("score").map_err(db_err)?,
                    answered_pages: row.get("answered_pages").map_err(db_err)?,
                    answered_questions: row.get("answered_questions").map_err(db_err)?,
                    answers: Vec::new(),
                });
            }

            // Attach the answer carried by this row (NULL for answer-less results).
            if let Some(answer) = decode_answer(row)?
                && let Some(current) = results.last_mut()
            {
                current.answers.push(answer);
            }
        }

        Ok(results)
    }

    async fn delete_results(&self, id: &str) -> Result<(), PersistenceError> {
        let mut conn = self.db_conn.lock().map_err(lock_err)?;
        let tx = conn.transaction().map_err(db_err)?;

        tx.execute("DELETE FROM answers WHERE survey_id = ?1", [id]).map_err(db_err)?;
        tx.execute("DELETE FROM results WHERE survey_id = ?1", [id]).map_err(db_err)?;

        tx.commit().map_err(db_err)?;

        Ok(())
    }

    async fn get_highscore(&self, id: &str, limit: u32) -> Result<Vec<HighscoreEntry>, PersistenceError> {
        let conn = self.db_conn.lock().map_err(lock_err)?;

        let mut stmt = conn.prepare("SELECT user, score, end_time FROM results WHERE survey_Id = ?1 ORDER BY score DESC LIMIT ?2").map_err(db_err)?;

        let results = stmt
            .query_map(params![id, limit], |row| Ok(HighscoreEntry { name: row.get("user")?, score: row.get("score")?, time: row.get("end_time")? }))
            .map_err(db_err)?
            .map(|r| r.map_err(db_err))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }
}

/// Separator between elements of a collection answer (ASCII record separator).
const RECORD_SEP: &str = "\u{1e}";
/// Separator between a key and its value in a map answer (ASCII unit separator).
const UNIT_SEP: &str = "\u{1f}";

/// Map a rusqlite error into a [`PersistenceError::DbError`].
fn db_err(e: rusqlite::Error) -> PersistenceError {
    PersistenceError::DbError(e.to_string())
}

fn lock_err(e: PoisonError<MutexGuard<Connection>>) -> PersistenceError {
    PersistenceError::Generic(e.to_string())
}

struct EncodedAnswer {
    question_type: &'static str,
    string_answer: Option<String>,
    int_answer: Option<i32>,
    string_vec_answer: Option<String>,
    string_map_answer: Option<String>,
    float_answer1: Option<f32>,
    float_answer2: Option<f32>,
}

impl EncodedAnswer {
    fn empty(question_type: &'static str) -> Self {
        EncodedAnswer {
            question_type,
            string_answer: None,
            int_answer: None,
            string_vec_answer: None,
            string_map_answer: None,
            float_answer1: None,
            float_answer2: None,
        }
    }
}

/// Map a typed [`Answer`] onto the columns of the `answers` table.
fn encode_answer(answer: &Answer) -> EncodedAnswer {
    match answer {
        Answer::Data(s) => EncodedAnswer { string_answer: Some(s.clone()), ..EncodedAnswer::empty("Data") },
        Answer::Text(s) => EncodedAnswer { string_answer: Some(s.clone()), ..EncodedAnswer::empty("Text") },
        Answer::Datetime(s) => EncodedAnswer { string_answer: Some(s.clone()), ..EncodedAnswer::empty("Datetime") },
        Answer::Rating(r) => EncodedAnswer { int_answer: Some(*r), ..EncodedAnswer::empty("Rating") },
        Answer::Choice(v) => EncodedAnswer { string_vec_answer: Some(v.join(RECORD_SEP)), ..EncodedAnswer::empty("Choice") },
        Answer::Likert(m) => EncodedAnswer {
            string_map_answer: Some(m.iter().map(|(k, v)| format!("{k}{UNIT_SEP}{v}")).collect::<Vec<_>>().join(RECORD_SEP)),
            ..EncodedAnswer::empty("Likert")
        },
        Answer::Slider(a, b) => EncodedAnswer { float_answer1: Some(*a), float_answer2: *b, ..EncodedAnswer::empty("Slider") },
    }
}

/// Reconstruct a typed [`QuestionAnswer`] from a result row.
///
/// Returns `None` when the row carries no answer
fn decode_answer(row: &Row) -> Result<Option<QuestionAnswer>, PersistenceError> {
    let Some(question_type) = row.get::<_, Option<String>>("question_type").map_err(db_err)? else {
        return Ok(None);
    };

    let answer = match question_type.as_str() {
        "Data" => {
            let value = row.get::<_, Option<String>>("string_answer").map_err(db_err)?;
            Answer::Data(value.ok_or_else(|| PersistenceError::DbError("answer of type 'Data' is missing required column 'string_answer'".to_string()))?)
        },
        "Text" => {
            let value = row.get::<_, Option<String>>("string_answer").map_err(db_err)?;
            Answer::Text(value.ok_or_else(|| PersistenceError::DbError("answer of type 'Text' is missing required column 'string_answer'".to_string()))?)
        },
        "Datetime" => {
            let value = row.get::<_, Option<String>>("string_answer").map_err(db_err)?;
            Answer::Datetime(
                value.ok_or_else(|| PersistenceError::DbError("answer of type 'Datetime' is missing required column 'string_answer'".to_string()))?,
            )
        },
        "Rating" => {
            let value = row.get::<_, Option<i32>>("int_answer").map_err(db_err)?;
            Answer::Rating(value.ok_or_else(|| PersistenceError::DbError("answer of type 'Rating' is missing required column 'int_answer'".to_string()))?)
        },
        "Choice" => {
            let value = row
                .get::<_, Option<String>>("string_vec_answer")
                .map_err(db_err)?
                .ok_or_else(|| PersistenceError::DbError("answer of type 'Choice' is missing required column 'string_vec_answer'".to_string()))?;
            if value.is_empty() { Answer::Choice(Vec::new()) } else { Answer::Choice(value.split(RECORD_SEP).map(str::to_string).collect()) }
        },
        "Likert" => {
            let value = row
                .get::<_, Option<String>>("string_map_answer")
                .map_err(db_err)?
                .ok_or_else(|| PersistenceError::DbError("answer of type 'Likert' is missing required column 'string_map_answer'".to_string()))?;
            if value.is_empty() {
                Answer::Likert(HashMap::new())
            } else {
                Answer::Likert(value.split(RECORD_SEP).filter_map(|entry| entry.split_once(UNIT_SEP).map(|(k, v)| (k.to_string(), v.to_string()))).collect())
            }
        },
        "Slider" => {
            let value = row.get::<_, Option<f32>>("float_answer1").map_err(db_err)?;
            Answer::Slider(
                value.ok_or_else(|| PersistenceError::DbError("answer of type 'Slider' is missing required column 'float_answer1'".to_string()))?,
                // The upper bound of a slider answer is optional by design.
                row.get::<_, Option<f32>>("float_answer2").map_err(db_err)?,
            )
        },
        other => return Err(PersistenceError::Generic(format!("unknown answer type: {other}"))),
    };

    Ok(Some(QuestionAnswer {
        question_id: row.get("question_id").map_err(db_err)?,
        question_title: row.get("question_title").map_err(db_err)?,
        is_answered: row.get("answered").map_err(db_err)?,
        answer,
    }))
}

fn has_conditionals(config: &survey_tool_cli::SurveyConfig) -> bool {
    for page in &config.pages {
        if page.conditional.is_some() {
            return true;
        }
        if page.content.iter().any(|c| c.get_header().conditional.is_some()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use survey_tool_cli::{ConditionalSettings, SurveyConfig, SurveyPage, SurveyPageContent, SurveyPageContentHeader};

    // ----------------------------------------------------------------------
    // Test helpers
    // ----------------------------------------------------------------------

    /// A temporary directory that is removed again when the test finishes.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("survey-tool-test-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("could not create temp dir");
            TempDir { path }
        }

        fn as_str(&self) -> &str {
            self.path.to_str().unwrap()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Build a client backed by an in-memory database and the given storage folder.
    fn client_with_storage(storage: &TempDir) -> LocalSurveyPersistenceClient {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DB_INIT).unwrap();
        LocalSurveyPersistenceClient { storage_path: storage.as_str().to_string(), db_conn: Mutex::new(conn) }
    }

    /// Insert a survey row directly, bypassing the file storage.
    fn insert_survey(client: &LocalSurveyPersistenceClient, id: &str, name: &str, active: bool, survey_type: &str) {
        let conn = client.db_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO surveys (id, name, desc, type, active, page_count, question_count, conditionals) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, name, "some description", survey_type, active, 3i64, 7i64, false],
        )
        .unwrap();
    }

    /// Insert a result row directly with an integer score.
    fn insert_result(client: &LocalSurveyPersistenceClient, survey_id: &str, user: Option<&str>, score: Option<i64>, end_time: &str) {
        let conn = client.db_conn.lock().unwrap();
        conn.execute(
            "INSERT INTO results (survey_id, origin, start_time, end_time, answered_pages, answered_questions, user, score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![survey_id, "web", "t0", end_time, 1i64, 2i64, user, score],
        )
        .unwrap();
    }

    fn qa(id: &str, answered: bool, answer: Answer) -> QuestionAnswer {
        QuestionAnswer { question_id: id.to_string(), question_title: format!("title-{id}"), is_answered: answered, answer }
    }

    /// Build an in-memory zip archive containing the survey config yaml plus one page.
    fn survey_zip(title: &str, description: &str, survey_type: &str) -> Vec<u8> {
        let yaml = format!(
            "title: {title}\ndescription: {description}\ntype: {survey_type}\n---\ncontent:\n- type: text\n  title: Q1\n  config:\n    multiline: false\n"
        );
        let mut buf = Vec::new();
        {
            let cursor = io::Cursor::new(&mut buf);
            let mut zip = zip::ZipWriter::new(cursor);
            zip.start_file("survey_config.yaml", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(yaml.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf
    }

    // ----------------------------------------------------------------------
    // encode_answer: column mapping and separator encoding
    // ----------------------------------------------------------------------

    #[test]
    fn encode_answer_maps_each_variant_to_the_right_columns() {
        let text = encode_answer(&Answer::Text("hello".to_string()));
        assert_eq!(text.question_type, "Text");
        assert_eq!(text.string_answer.as_deref(), Some("hello"));
        assert_eq!(text.int_answer, None);

        let rating = encode_answer(&Answer::Rating(4));
        assert_eq!(rating.question_type, "Rating");
        assert_eq!(rating.int_answer, Some(4));
        assert_eq!(rating.string_answer, None);

        // Collection answers are flattened using the record separator.
        let choice = encode_answer(&Answer::Choice(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(choice.question_type, "Choice");
        assert_eq!(choice.string_vec_answer.as_deref(), Some("a\u{1e}b"));

        // A single-entry map keeps the assertion deterministic (unit + record separators).
        let likert = encode_answer(&Answer::Likert(HashMap::from([("s1".to_string(), "agree".to_string())])));
        assert_eq!(likert.question_type, "Likert");
        assert_eq!(likert.string_map_answer.as_deref(), Some("s1\u{1f}agree"));

        let slider = encode_answer(&Answer::Slider(0.5, Some(1.5)));
        assert_eq!(slider.question_type, "Slider");
        assert_eq!(slider.float_answer1, Some(0.5));
        assert_eq!(slider.float_answer2, Some(1.5));

        // The upper slider bound is optional.
        let slider_open = encode_answer(&Answer::Slider(2.0, None));
        assert_eq!(slider_open.float_answer1, Some(2.0));
        assert_eq!(slider_open.float_answer2, None);
    }

    // ----------------------------------------------------------------------
    // save_result + get_results: full round trip through the database
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn save_and_get_results_round_trips_every_answer_variant() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);

        let answers = vec![
            qa("q0", true, Answer::Data("participant".to_string())),
            qa("q1", true, Answer::Text("free text".to_string())),
            qa("q2", true, Answer::Datetime("2024-01-02T03:04:05".to_string())),
            qa("q3", true, Answer::Rating(4)),
            qa("q4", true, Answer::Choice(vec!["x".to_string(), "y".to_string()])),
            qa("q5", false, Answer::Choice(Vec::new())),
            qa("q6", true, Answer::Likert(HashMap::from([("s1".to_string(), "agree".to_string())]))),
            qa("q7", false, Answer::Likert(HashMap::new())),
            qa("q8", true, Answer::Slider(0.5, Some(1.5))),
            qa("q9", true, Answer::Slider(2.0, None)),
        ];

        let result = SurveyResult {
            origin: "web".to_string(),
            start_time: "start".to_string(),
            end_time: "end".to_string(),
            user: Some("alice".to_string()),
            score: Some(7.5),
            answered_pages: 2,
            answered_questions: 8,
            answers,
        };

        client.save_result("survey-1", result).await.unwrap();

        let loaded = client.get_results("survey-1").await.unwrap();
        assert_eq!(loaded.len(), 1);
        let r = &loaded[0];

        // Result-level fields survive the round trip.
        assert_eq!(r.origin, "web");
        assert_eq!(r.user.as_deref(), Some("alice"));
        assert_eq!(r.score, Some(7.5));
        assert_eq!(r.answered_pages, 2);
        assert_eq!(r.answered_questions, 8);

        // Answers come back in insertion order (ORDER BY a.id).
        assert_eq!(r.answers.len(), 10);
        assert!(matches!(&r.answers[0].answer, Answer::Data(s) if s == "participant"));
        assert!(matches!(&r.answers[1].answer, Answer::Text(s) if s == "free text"));
        assert!(matches!(&r.answers[2].answer, Answer::Datetime(s) if s == "2024-01-02T03:04:05"));
        assert!(matches!(&r.answers[3].answer, Answer::Rating(4)));
        assert!(matches!(&r.answers[4].answer, Answer::Choice(v) if v == &vec!["x".to_string(), "y".to_string()]));
        assert!(matches!(&r.answers[5].answer, Answer::Choice(v) if v.is_empty()));
        assert!(matches!(&r.answers[6].answer, Answer::Likert(m) if m.get("s1").map(String::as_str) == Some("agree")));
        assert!(matches!(&r.answers[7].answer, Answer::Likert(m) if m.is_empty()));
        assert!(matches!(&r.answers[8].answer, Answer::Slider(a, Some(b)) if *a == 0.5 && *b == 1.5));
        assert!(matches!(&r.answers[9].answer, Answer::Slider(a, None) if *a == 2.0));

        // The is_answered flag is preserved per answer.
        assert!(r.answers[0].is_answered);
        assert!(!r.answers[5].is_answered);
        assert_eq!(r.answers[0].question_title, "title-q0");
    }

    #[tokio::test]
    async fn get_results_groups_rows_and_keeps_answerless_results() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);

        let with_answers = SurveyResult {
            origin: "web".to_string(),
            start_time: "s".to_string(),
            end_time: "e".to_string(),
            user: None,
            score: None,
            answered_pages: 1,
            answered_questions: 2,
            answers: vec![qa("a", true, Answer::Rating(1)), qa("b", true, Answer::Text("t".to_string()))],
        };
        let without_answers = SurveyResult {
            origin: "cli".to_string(),
            start_time: "s".to_string(),
            end_time: "e".to_string(),
            user: None,
            score: None,
            answered_pages: 0,
            answered_questions: 0,
            answers: Vec::new(),
        };

        client.save_result("s1", with_answers).await.unwrap();
        client.save_result("s1", without_answers).await.unwrap();

        let loaded = client.get_results("s1").await.unwrap();
        assert_eq!(loaded.len(), 2);
        // Ordered by result id: the first inserted result comes first.
        assert_eq!(loaded[0].answers.len(), 2);
        assert_eq!(loaded[1].origin, "cli");
        assert!(loaded[1].answers.is_empty());
    }

    // ----------------------------------------------------------------------
    // get_survey_summary: aggregation and not-found handling
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn get_survey_summary_aggregates_result_statistics() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);

        insert_survey(&client, "s1", "My Quiz", true, "quiz");
        insert_result(&client, "s1", Some("alice"), Some(10), "2024-01-01");
        insert_result(&client, "s1", Some("bob"), Some(30), "2024-01-03");
        insert_result(&client, "s1", Some("carol"), Some(20), "2024-01-02");

        let summary = client.get_survey_summary("s1").await.unwrap();
        assert_eq!(summary.title, "My Quiz");
        assert!(matches!(summary.survey_type, SurveyType::Quiz));
        assert_eq!(summary.submit_count, 3);
        assert_eq!(summary.min_score, Some(10));
        assert_eq!(summary.max_score, Some(30));
        assert_eq!(summary.avg_score, Some(20.0));
        assert_eq!(summary.first_submit_time.as_deref(), Some("2024-01-01"));
        assert_eq!(summary.last_submit_time.as_deref(), Some("2024-01-03"));
    }

    #[tokio::test]
    async fn get_survey_summary_without_results_reports_zero_and_nulls() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        insert_survey(&client, "s1", "Empty", true, "survey");

        let summary = client.get_survey_summary("s1").await.unwrap();
        assert_eq!(summary.submit_count, 0);
        assert_eq!(summary.min_score, None);
        assert_eq!(summary.max_score, None);
        assert_eq!(summary.avg_score, None);
    }

    #[tokio::test]
    async fn get_survey_summary_unknown_id_is_not_found() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        match client.get_survey_summary("missing").await {
            Err(PersistenceError::NotFound(_)) => {},
            _ => panic!("expected NotFound for unknown survey id"),
        }
    }

    // ----------------------------------------------------------------------
    // list_surveys: filtering by active flag and type
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn list_surveys_filters_by_active_and_type() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        insert_survey(&client, "a", "Active Quiz", true, "quiz");
        insert_survey(&client, "b", "Inactive Survey", false, "survey");

        assert_eq!(client.list_surveys(None, None).await.unwrap().len(), 2);

        let active_only = client.list_surveys(Some(true), None).await.unwrap();
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].id, "a");

        let surveys_only = client.list_surveys(None, Some(SurveyType::Survey)).await.unwrap();
        assert_eq!(surveys_only.len(), 1);
        assert_eq!(surveys_only[0].id, "b");

        // Combined filter that matches nothing.
        assert!(client.list_surveys(Some(true), Some(SurveyType::Survey)).await.unwrap().is_empty());
    }

    // ----------------------------------------------------------------------
    // set / get survey state
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn set_and_get_survey_state_toggles_active_flag() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        insert_survey(&client, "s1", "Toggle", true, "survey");

        assert!(client.get_survey_active("s1").await.unwrap());
        client.set_survey_state("s1", false).await.unwrap();
        assert!(!client.get_survey_active("s1").await.unwrap());
    }

    #[tokio::test]
    async fn survey_state_operations_on_unknown_id_are_not_found() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        assert!(matches!(client.get_survey_active("missing").await.unwrap_err(), PersistenceError::NotFound(_)));
        assert!(matches!(client.set_survey_state("missing", true).await.unwrap_err(), PersistenceError::NotFound(_)));
    }

    // ----------------------------------------------------------------------
    // get_highscore: ordering and limit
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn get_highscore_orders_by_score_descending_and_limits() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        insert_result(&client, "s1", Some("low"), Some(10), "t1");
        insert_result(&client, "s1", Some("high"), Some(30), "t2");
        insert_result(&client, "s1", Some("mid"), Some(20), "t3");

        let top = client.get_highscore("s1", 2).await.unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "high");
        assert_eq!(top[0].score, 30);
        assert_eq!(top[1].name, "mid");
    }

    // ----------------------------------------------------------------------
    // delete operations
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_results_removes_results_but_keeps_survey() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        insert_survey(&client, "s1", "Quiz", true, "quiz");
        insert_result(&client, "s1", Some("alice"), Some(5), "t1");

        client.delete_results("s1").await.unwrap();

        assert!(client.get_results("s1").await.unwrap().is_empty());
        // The survey itself is untouched.
        assert!(client.get_survey_active("s1").await.unwrap());
    }

    #[tokio::test]
    async fn delete_survey_removes_db_row_and_stored_file() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);

        let id = client.save_survey(survey_zip("Del", "to be deleted", "survey")).await.unwrap();
        // File exists before deletion.
        assert!(client.get_survey(&id).await.is_ok());

        client.delete_survey(&id).await.unwrap();

        assert!(matches!(client.get_survey_active(&id).await.unwrap_err(), PersistenceError::NotFound(_)));
        assert!(matches!(client.get_survey(&id).await.unwrap_err(), PersistenceError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_survey_unknown_id_is_not_found() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        assert!(matches!(client.delete_survey("missing").await.unwrap_err(), PersistenceError::NotFound(_)));
    }

    // ----------------------------------------------------------------------
    // save_survey + get_survey: zip parsing and file storage
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn save_survey_stores_file_and_indexes_config() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);

        let bytes = survey_zip("Great Quiz", "the description", "quiz");
        let id = client.save_survey(bytes.clone()).await.unwrap();

        // The exact archive bytes are returned unchanged.
        assert_eq!(client.get_survey(&id).await.unwrap(), bytes);

        // Config metadata was extracted into the summary.
        let summary = client.get_survey_summary(&id).await.unwrap();
        assert_eq!(summary.title, "Great Quiz");
        assert_eq!(summary.description, "the description");
        assert!(matches!(summary.survey_type, SurveyType::Quiz));
        assert!(summary.active);
        assert_eq!(summary.page_count, 1);
        assert_eq!(summary.question_count, 1);
    }

    #[tokio::test]
    async fn save_survey_rejects_non_zip_input() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        let err = client.save_survey(b"not a zip file".to_vec()).await.unwrap_err();
        assert!(matches!(err, PersistenceError::ZipFileError(_)));
    }

    #[tokio::test]
    async fn get_survey_missing_file_is_storage_error() {
        let storage = TempDir::new();
        let client = client_with_storage(&storage);
        assert!(matches!(client.get_survey("does-not-exist").await.unwrap_err(), PersistenceError::NotFound(_)));
    }

    // ----------------------------------------------------------------------
    // has_conditionals: detection at page and content level
    // ----------------------------------------------------------------------

    fn info_content(conditional: Option<ConditionalSettings>) -> SurveyPageContent {
        SurveyPageContent::Information {
            header: SurveyPageContentHeader { title: "info".to_string(), required: false, conditional },
            description: None,
            image: None,
        }
    }

    #[test]
    fn has_conditionals_is_false_without_any_conditionals() {
        let mut config = SurveyConfig::new("t".to_string(), "d".to_string(), None, None, None, None);
        let mut page = SurveyPage::default();
        page.add_content(info_content(None));
        config.add_page(page);
        assert!(!has_conditionals(&config));
    }

    #[test]
    fn has_conditionals_detects_a_page_level_conditional() {
        let mut config = SurveyConfig::new("t".to_string(), "d".to_string(), None, None, None, None);
        let page = SurveyPage::new(None, None, None, Some(ConditionalSettings::new("k".to_string(), vec!["v".to_string()])));
        config.add_page(page);
        assert!(has_conditionals(&config));
    }

    #[test]
    fn has_conditionals_detects_a_content_level_conditional() {
        let mut config = SurveyConfig::new("t".to_string(), "d".to_string(), None, None, None, None);
        let mut page = SurveyPage::default();
        page.add_content(info_content(Some(ConditionalSettings::new("k".to_string(), vec!["v".to_string()]))));
        config.add_page(page);
        assert!(has_conditionals(&config));
    }

    // ----------------------------------------------------------------------
    // new(): storage folder validation
    // ----------------------------------------------------------------------

    #[tokio::test]
    async fn new_succeeds_for_writable_directories() {
        let storage = TempDir::new();
        let db = TempDir::new();
        let client = new(storage.as_str(), db.as_str()).await.unwrap();
        assert!(client.list_surveys(None, None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn new_fails_when_storage_folder_is_missing() {
        let db = TempDir::new();
        match new("this/path/does/not/exist", db.as_str()).await {
            Err(PersistenceError::NotFound(_)) => {},
            _ => panic!("expected NotFound for missing storage folder"),
        }
    }
}
