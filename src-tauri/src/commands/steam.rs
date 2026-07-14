use tauri::AppHandle;

#[cfg(not(windows))]
use tauri::Manager;

use crate::core::error::{AppError, AppResult, ErrorResponse};
use crate::steam::{discover_installed_games, SteamGame};

#[tauri::command]
pub async fn list_steam_games(_app: AppHandle) -> AppResult<Vec<SteamGame>> {
    #[cfg(windows)]
    let home_dir: Option<std::path::PathBuf> = None;

    #[cfg(target_os = "linux")]
    let home_dir = match _app.path().home_dir() {
        Ok(home_dir) => Some(home_dir),
        Err(error) => {
            tracing::warn!(%error, "failed to resolve the home directory for Steam discovery");
            None
        }
    };

    #[cfg(all(not(windows), not(target_os = "linux")))]
    let home_dir = Some(_app.path().home_dir().map_err(|error| {
        tracing::warn!(%error, "failed to resolve the home directory for Steam discovery");
        ErrorResponse::from(AppError::SteamDiscovery(
            "Steam library locations could not be resolved".into(),
        ))
    })?);

    let games =
        tauri::async_runtime::spawn_blocking(move || discover_installed_games(home_dir.as_deref()))
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Steam discovery task failed");
                ErrorResponse::from(AppError::SteamDiscovery(
                    "Steam games could not be read".into(),
                ))
            })?;

    games.map_err(|_| {
        ErrorResponse::from(AppError::SteamDiscovery(
            "Steam games could not be read".into(),
        ))
    })
}
