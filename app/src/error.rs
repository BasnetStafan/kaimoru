//! The shared service-layer error type.
//!
//! Before this existed, services invented their own failure shapes
//! and a database failure could surface as a form-field error.
//! Services now return [`AppError`]; controllers decide how each
//! variant is presented, and generic user-facing text lives in
//! [`AppError::user_message`].

use crate::messages::GENERIC_FAILURE_MESSAGE;
use crate::validation::FieldError;

/// A failure of a service operation.
#[derive(Debug)]
pub enum AppError {
    /// The requested record does not exist (or is not visible to
    /// the caller, where ownership is part of the lookup).
    NotFound,
    /// Input failed validation: one error per failing field,
    /// rendered under its field by the form, never as a banner.
    Validation(Vec<FieldError>),
    /// A business rule refused the operation; the string is the
    /// user-safe explanation (e.g. attempts remaining), presented
    /// as the form-level message.
    Conflict(String),
    /// A database failure. Details belong in the log — logged by
    /// the layer that creates this variant — never on screen.
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        // The promise on [`AppError::db`] — a `Db` reaching a
        // controller has always been logged once — must hold
        // for plain `?` conversions too, so the conversion logs.
        tracing::error!("database error: {e}");
        Self::Db(e)
    }
}

impl std::fmt::Display for AppError {
    /// The log-shaped rendering. Customer-facing wording is
    /// [`AppError::user_message`], never this.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::Validation(fields) => write!(f, "validation failed ({} fields)", fields.len()),
            Self::Conflict(message) => write!(f, "{message}"),
            Self::Db(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl AppError {
    /// The user-facing message for failures that are not field
    /// errors; validation failures are rendered per field instead.
    pub fn user_message(&self) -> String {
        match self {
            Self::NotFound => "お探しの情報が見つかりませんでした。".to_string(),
            Self::Validation(_) => "入力内容をご確認ください。".to_string(),
            Self::Conflict(message) => message.clone(),
            Self::Db(_) => GENERIC_FAILURE_MESSAGE.to_string(),
        }
    }

    /// Logs a database failure with context and wraps it, so a
    /// `Db` reaching a controller has always been logged once.
    pub fn db(context: &str, e: sqlx::Error) -> Self {
        tracing::error!("{context}: {e}");
        Self::Db(e)
    }
}
