use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AppErrorKind {
    InvalidInput,
    NotFound,
    AlreadyExists,
    InvalidEpub,
    Storage,
    Database,
    AiUnavailable,
    AiRequest,
    Internal,
}

#[derive(Debug, Clone, Error, Serialize)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub kind: AppErrorKind,
    pub message: String,
}

impl AppError {
    pub fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::NotFound, message)
    }

    pub fn database() -> Self {
        Self::new(
            AppErrorKind::Database,
            "The library database operation failed.",
        )
    }

    pub fn storage() -> Self {
        Self::new(
            AppErrorKind::Storage,
            "The book files could not be accessed.",
        )
    }

    pub fn invalid_epub(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::InvalidEpub, message)
    }

    pub fn ai_unavailable(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::AiUnavailable, message)
    }

    pub fn ai_request(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::AiRequest, message)
    }
}

pub type AppResult<T> = Result<T, AppError>;
