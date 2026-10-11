//! The error shape every command returns to the UI: `{ code, message }` (SPEC §4.5).

use serde::Serialize;
use ts_rs::TS;

use crate::config::ConfigError;

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CmdError {
    /// Stable, machine-readable error kind, e.g. `invalid` or `io`.
    pub code: String,
    pub message: String,
}

impl CmdError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl From<ConfigError> for CmdError {
    fn from(e: ConfigError) -> Self {
        let code = match e {
            ConfigError::Io(_) => "io",
            ConfigError::Json(_) => "json",
            ConfigError::Invalid(_) => "invalid",
        };
        Self::new(code, e.to_string())
    }
}

pub type CmdResult<T> = Result<T, CmdError>;
