//! GUI 可稳定匹配的错误类别。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiErrorCode {
    InvalidInput,
    Busy,
    NotFound,
    PermissionDenied,
    UntrustedServer,
    Network,
    Transfer,
    Storage,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct GuiError {
    pub code: GuiErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl From<GuiError> for String {
    fn from(error: GuiError) -> Self {
        error.message
    }
}

impl GuiError {
    pub fn from_message(message: String) -> Self {
        let lower = message.to_ascii_lowercase();
        let (code, retryable) = if lower.contains("already") || lower.contains("in progress") {
            (GuiErrorCode::Busy, true)
        } else if lower.contains("untrusted") || lower.contains("fingerprint") {
            (GuiErrorCode::UntrustedServer, true)
        } else if lower.contains("permission") || lower.contains("denied") {
            (GuiErrorCode::PermissionDenied, true)
        } else if lower.contains("invalid") || lower.contains("choose") || lower.contains("must") {
            (GuiErrorCode::InvalidInput, false)
        } else if lower.contains("server")
            || lower.contains("network")
            || lower.contains("timed out")
        {
            (GuiErrorCode::Network, true)
        } else if lower.contains("file") || lower.contains("directory") || lower.contains("export")
        {
            (GuiErrorCode::Storage, true)
        } else {
            (GuiErrorCode::Transfer, true)
        };
        Self {
            code,
            message,
            retryable,
        }
    }
}

pub(crate) trait GuiResultExt<T> {
    fn gui(self) -> Result<T, GuiError>;
}

impl<T> GuiResultExt<T> for Result<T, String> {
    fn gui(self) -> Result<T, GuiError> {
        self.map_err(GuiError::from_message)
    }
}
