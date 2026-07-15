use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;
use rusqlite::Connection;

use crate::error::{AppError, AppErrorKind, AppResult};

pub const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_GENERATION_MODEL: &str = "llama3.2:latest";
pub const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text:latest";

#[derive(Clone)]
pub struct AppState {
    pub root: PathBuf,
    pub db: Arc<Mutex<Connection>>,
    pub http: Client,
    pub generation_model: String,
    pub embedding_model: String,
}

impl AppState {
    pub fn new(root: PathBuf, connection: Connection) -> AppResult<Self> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| {
                tracing::error!(%error, "failed to create HTTP client");
                AppError::new(
                    AppErrorKind::Internal,
                    "The AI client could not be initialized.",
                )
            })?;

        Ok(Self {
            root,
            db: Arc::new(Mutex::new(connection)),
            http,
            generation_model: DEFAULT_GENERATION_MODEL.to_owned(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_owned(),
        })
    }

    pub fn lock_db(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.db.lock().map_err(|_| AppError::database())
    }
}
