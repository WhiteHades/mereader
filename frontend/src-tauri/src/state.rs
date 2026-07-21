use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;
use rusqlite::{Connection, OpenFlags};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppErrorKind, AppResult};

pub const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_GENERATION_MODEL: &str = "llama3.2:latest";
pub const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text:latest";

#[derive(Default)]
struct AiRequestRegistry {
    active: HashMap<String, CancellationToken>,
    pending: HashSet<String>,
    finished: HashSet<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub db: Arc<Mutex<Connection>>,
    pub status_http: Client,
    pub embedding_http: Client,
    pub generation_http: Client,
    pub generation_model: String,
    pub embedding_model: String,
    pub import_slots: Arc<Semaphore>,
    pub ai_slots: Arc<Semaphore>,
    ai_requests: Arc<Mutex<AiRequestRegistry>>,
    indexing_books: Arc<Mutex<HashSet<String>>>,
}

pub struct IndexClaim {
    book_id: String,
    indexing_books: Arc<Mutex<HashSet<String>>>,
}

impl Drop for IndexClaim {
    fn drop(&mut self) {
        if let Ok(mut books) = self.indexing_books.lock() {
            books.remove(&self.book_id);
        }
    }
}

impl AppState {
    pub fn new(root: PathBuf, connection: Connection) -> AppResult<Self> {
        let client = |timeout: Option<Duration>| {
            let builder = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2));
            match timeout {
                Some(timeout) => builder.timeout(timeout),
                None => builder,
            }
            .build()
        };
        let status_http = client(Some(Duration::from_secs(5))).map_err(|error| {
            tracing::error!(%error, "failed to create status HTTP client");
            AppError::new(
                AppErrorKind::Internal,
                "The AI client could not be initialized.",
            )
        })?;
        let embedding_http = client(Some(Duration::from_secs(60))).map_err(|error| {
            tracing::error!(%error, "failed to create embedding HTTP client");
            AppError::new(
                AppErrorKind::Internal,
                "The AI client could not be initialized.",
            )
        })?;
        let generation_http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5 * 60))
            .build()
            .map_err(|error| {
                tracing::error!(%error, "failed to create generation HTTP client");
                AppError::new(
                    AppErrorKind::Internal,
                    "The AI client could not be initialized.",
                )
            })?;

        Ok(Self {
            db_path: root.join("library.sqlite3"),
            root,
            db: Arc::new(Mutex::new(connection)),
            status_http,
            embedding_http,
            generation_http,
            generation_model: DEFAULT_GENERATION_MODEL.to_owned(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_owned(),
            import_slots: Arc::new(Semaphore::new(1)),
            ai_slots: Arc::new(Semaphore::new(2)),
            ai_requests: Arc::new(Mutex::new(AiRequestRegistry::default())),
            indexing_books: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn lock_db(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.db.lock().map_err(|_| AppError::database())
    }

    pub fn open_read_db(&self) -> AppResult<Connection> {
        let connection = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to open SQLite read connection");
            AppError::database()
        })?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|_| AppError::database())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|_| AppError::database())?;
        Ok(connection)
    }

    pub fn register_ai_request(&self, request_id: &str) -> AppResult<CancellationToken> {
        let mut requests = self
            .ai_requests
            .lock()
            .map_err(|_| AppError::new(AppErrorKind::Internal, "AI work is unavailable."))?;
        if requests.active.contains_key(request_id) {
            return Err(AppError::invalid(
                "The AI request identifier is already active.",
            ));
        }
        let token = CancellationToken::new();
        requests.finished.remove(request_id);
        if requests.pending.remove(request_id) {
            token.cancel();
        }
        requests.active.insert(request_id.to_owned(), token.clone());
        Ok(token)
    }

    pub fn cancel_ai_request(&self, request_id: &str) -> AppResult<bool> {
        let mut requests = self
            .ai_requests
            .lock()
            .map_err(|_| AppError::new(AppErrorKind::Internal, "AI work is unavailable."))?;
        if let Some(token) = requests.active.get(request_id) {
            token.cancel();
            return Ok(true);
        }
        if requests.finished.contains(request_id) {
            return Ok(false);
        }
        if requests.pending.len() >= 128 && !requests.pending.contains(request_id) {
            if let Some(oldest) = requests.pending.iter().next().cloned() {
                requests.pending.remove(&oldest);
            }
        }
        requests.pending.insert(request_id.to_owned());
        Ok(true)
    }

    pub fn finish_ai_request(&self, request_id: &str) {
        if let Ok(mut requests) = self.ai_requests.lock() {
            requests.active.remove(request_id);
            requests.pending.remove(request_id);
            if requests.finished.len() >= 128 && !requests.finished.contains(request_id) {
                if let Some(oldest) = requests.finished.iter().next().cloned() {
                    requests.finished.remove(&oldest);
                }
            }
            requests.finished.insert(request_id.to_owned());
        }
    }

    pub fn claim_indexing(&self, book_id: &str) -> AppResult<IndexClaim> {
        let mut books = self
            .indexing_books
            .lock()
            .map_err(|_| AppError::new(AppErrorKind::Internal, "AI work is unavailable."))?;
        if !books.insert(book_id.to_owned()) {
            return Err(AppError::ai_request("This book is already being indexed."));
        }
        Ok(IndexClaim {
            book_id: book_id.to_owned(),
            indexing_books: self.indexing_books.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new(PathBuf::new(), Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn indexing_claims_are_unique_until_released() {
        let state = state();
        let claim = state.claim_indexing("book").unwrap();
        assert!(state.claim_indexing("book").is_err());
        drop(claim);
        assert!(state.claim_indexing("book").is_ok());
    }

    #[test]
    fn ai_requests_can_be_cancelled_and_reused_after_finish() {
        let state = state();
        let token = state.register_ai_request("request").unwrap();
        assert!(state.register_ai_request("request").is_err());
        assert!(state.cancel_ai_request("request").unwrap());
        assert!(token.is_cancelled());
        state.finish_ai_request("request");
        assert!(state.register_ai_request("request").is_ok());
    }

    #[test]
    fn cancellation_before_registration_is_not_lost() {
        let state = state();
        assert!(state.cancel_ai_request("request").unwrap());

        let token = state.register_ai_request("request").unwrap();

        assert!(token.is_cancelled());
        state.finish_ai_request("request");
    }

    #[test]
    fn cancellation_after_finish_does_not_create_a_pending_request() {
        let state = state();
        state.register_ai_request("request").unwrap();
        state.finish_ai_request("request");

        assert!(!state.cancel_ai_request("request").unwrap());
        assert!(!state.register_ai_request("request").unwrap().is_cancelled());
    }
}
