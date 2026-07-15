use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;
use rusqlite::Connection;
use tokio::sync::Semaphore;

use crate::error::{AppError, AppErrorKind, AppResult};

pub const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_GENERATION_MODEL: &str = "llama3.2:latest";
pub const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text:latest";

#[derive(Clone)]
pub struct AppState {
    pub root: PathBuf,
    pub db: Arc<Mutex<Connection>>,
    pub status_http: Client,
    pub embedding_http: Client,
    pub generation_http: Client,
    pub generation_model: String,
    pub embedding_model: String,
    pub import_slots: Arc<Semaphore>,
    pub ai_slots: Arc<Semaphore>,
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
            .build()
            .map_err(|error| {
                tracing::error!(%error, "failed to create generation HTTP client");
                AppError::new(
                    AppErrorKind::Internal,
                    "The AI client could not be initialized.",
                )
            })?;

        Ok(Self {
            root,
            db: Arc::new(Mutex::new(connection)),
            status_http,
            embedding_http,
            generation_http,
            generation_model: DEFAULT_GENERATION_MODEL.to_owned(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_owned(),
            import_slots: Arc::new(Semaphore::new(1)),
            ai_slots: Arc::new(Semaphore::new(2)),
        })
    }

    pub fn lock_db(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.db.lock().map_err(|_| AppError::database())
    }
}
