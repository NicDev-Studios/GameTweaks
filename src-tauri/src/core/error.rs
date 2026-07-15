use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("monitoring error: {0}")]
    Monitoring(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("steam discovery error: {0}")]
    SteamDiscovery(String),
    #[error("BepInEx error: {message}")]
    BepInEx { code: &'static str, message: String },
    #[error("game mod error: {message}")]
    GameMods { code: &'static str, message: String },
    #[error("updater error: {0}")]
    Updater(String),
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: &'static str,
    pub message: String,
}

impl From<AppError> for ErrorResponse {
    fn from(error: AppError) -> Self {
        let code = match &error {
            AppError::Config(_) => "config_error",
            AppError::Process(_) => "process_error",
            AppError::Monitoring(_) => "monitoring_error",
            AppError::Network(_) => "network_error",
            AppError::SteamDiscovery(_) => "steam_discovery_error",
            AppError::BepInEx { code, .. } => code,
            AppError::GameMods { code, .. } => code,
            AppError::Updater(_) => "updater_error",
        };

        Self {
            code,
            message: error.to_string(),
        }
    }
}

pub type AppResult<T> = Result<T, ErrorResponse>;
