use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Errors surfaced to API clients. Each variant maps to a stable `code`
/// string the frontend uses to show a helpful message.
///
/// Messages must never contain the DJI API key.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(
        "This flight log is encrypted (format v{0}). Set the DJI_API_KEY environment variable to decrypt it."
    )]
    MissingApiKey(u8),
    #[error(
        "DJI rejected the configured API key. Check DJI_API_KEY and that the app is activated in the DJI Developer portal."
    )]
    InvalidApiKey,
    #[error("Unsupported log format version {0} (supported: 1-{max}), or the file is not a DJI flight log.", max = crate::ingest::MAX_SUPPORTED_VERSION)]
    UnsupportedVersion(u8),
    #[error("The file is not a readable DJI flight log: {0}")]
    InvalidLog(String),
    #[error("The log could not be decrypted with the keys returned by DJI.")]
    DecryptionFailed,
    #[error("DJI API error: {0}")]
    DjiApi(String),
    #[error("Could not reach the DJI API: {0}")]
    Network(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotAvailable(String),
    #[error("{0}")]
    Upstream(String),
    #[error("The file is too large (limit: {0} MB). Set MAX_UPLOAD_MB to raise the limit.")]
    TooLarge(usize),
    #[error("Not found")]
    NotFound,
    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::MissingApiKey(_) => "missing_api_key",
            AppError::InvalidApiKey => "invalid_api_key",
            AppError::UnsupportedVersion(_) => "unsupported_version",
            AppError::InvalidLog(_) => "invalid_log",
            AppError::DecryptionFailed => "decryption_failed",
            AppError::DjiApi(_) => "dji_api_error",
            AppError::Network(_) => "network_error",
            AppError::BadRequest(_) => "bad_request",
            AppError::NotAvailable(_) => "not_available",
            AppError::Upstream(_) => "upstream_error",
            AppError::TooLarge(_) => "too_large",
            AppError::NotFound => "not_found",
            AppError::Internal(_) => "internal_error",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            AppError::MissingApiKey(_)
            | AppError::InvalidApiKey
            | AppError::UnsupportedVersion(_)
            | AppError::InvalidLog(_)
            | AppError::DecryptionFailed => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::DjiApi(_) | AppError::Network(_) | AppError::Upstream(_) => {
                StatusCode::BAD_GATEWAY
            }
            AppError::NotAvailable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound,
            other => AppError::Internal(format!("database: {other}")),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(code = self.code(), "{self}");
        } else {
            tracing::info!(code = self.code(), "{self}");
        }
        (
            status,
            Json(json!({ "error": { "code": self.code(), "message": self.to_string() } })),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
