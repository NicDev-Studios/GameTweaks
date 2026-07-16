use serde::Serialize;
use tauri::{AppHandle, State};

use crate::config::model::{
    developer_mode_enabled, developer_mode_forced, LanguagePreference, ThemePreference,
    UpdateChannel,
};
use crate::config::store::save_config;
use crate::core::error::AppResult;
use crate::core::state::AppState;
use crate::updater::{
    check_for_update as check_for_update_impl,
    download_and_install_update as download_and_install_update_impl, UpdateInfo,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppOverview {
    pub name: &'static str,
    pub version: &'static str,
    pub config_version: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperModeState {
    pub enabled: bool,
    pub forced: bool,
}

#[tauri::command]
pub async fn get_app_overview(state: State<'_, AppState>) -> AppResult<AppOverview> {
    let config = state.config.read().await;

    Ok(AppOverview {
        name: "GameTweaks",
        version: crate::version::current(),
        config_version: config.version,
    })
}

#[tauri::command]
pub async fn get_theme_preference(state: State<'_, AppState>) -> AppResult<ThemePreference> {
    Ok(state.config.read().await.theme.clone())
}

#[tauri::command]
pub async fn set_theme_preference(
    app: AppHandle,
    state: State<'_, AppState>,
    theme: ThemePreference,
) -> AppResult<ThemePreference> {
    let mut config = state.config.write().await;
    let mut updated = config.clone();
    updated.theme = theme;
    save_config(&app, &updated)?;
    *config = updated;
    Ok(config.theme.clone())
}

#[tauri::command]
pub async fn get_language_preference(state: State<'_, AppState>) -> AppResult<LanguagePreference> {
    Ok(state.config.read().await.language.clone())
}

#[tauri::command]
pub async fn set_language_preference(
    app: AppHandle,
    state: State<'_, AppState>,
    language: LanguagePreference,
) -> AppResult<LanguagePreference> {
    let mut config = state.config.write().await;
    let mut updated = config.clone();
    updated.language = language;
    save_config(&app, &updated)?;
    *config = updated;
    Ok(config.language.clone())
}

#[tauri::command]
pub async fn get_developer_mode(state: State<'_, AppState>) -> AppResult<DeveloperModeState> {
    let config = state.config.read().await;
    Ok(DeveloperModeState {
        enabled: developer_mode_enabled(&config),
        forced: developer_mode_forced(),
    })
}

#[tauri::command]
pub async fn set_developer_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<DeveloperModeState> {
    let mut config = state.config.write().await;
    if !developer_mode_forced() {
        let mut updated = config.clone();
        updated.developer_mode = enabled;
        save_config(&app, &updated)?;
        *config = updated;
    }
    Ok(DeveloperModeState {
        enabled: developer_mode_enabled(&config),
        forced: developer_mode_forced(),
    })
}

#[tauri::command]
pub async fn get_update_channel(state: State<'_, AppState>) -> AppResult<UpdateChannel> {
    Ok(state.config.read().await.update_channel)
}

#[tauri::command]
pub async fn set_update_channel(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: UpdateChannel,
) -> AppResult<UpdateChannel> {
    let _operation = state.update_operation.lock().await;
    let mut config = state.config.write().await;
    let mut updated = config.clone();
    updated.update_channel = channel;
    save_config(&app, &updated)?;
    *config = updated;
    *state.pending_update.lock().await = None;
    Ok(config.update_channel)
}

#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<UpdateInfo>> {
    check_for_update_impl(&app, &state).await
}

#[tauri::command]
pub async fn download_and_install_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    download_and_install_update_impl(&app, &state).await
}
