use std::collections::HashMap;

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::core::error::AppResult;
use crate::core::state::AppState;
use crate::game_mods::{self, GameSupport, ModActionPlan};

#[tauri::command]
pub async fn get_game_support(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
) -> AppResult<GameSupport> {
    game_mods::get_support(&app, &state, app_id).await
}

#[tauri::command]
pub async fn install_development_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
) -> AppResult<GameSupport> {
    crate::agent::install_development_agent(&app, &state, app_id).await?;
    game_mods::get_support(&app, &state, app_id).await
}

#[tauri::command]
pub async fn prepare_mod_install(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
    mod_ids: Vec<String>,
) -> AppResult<ModActionPlan> {
    game_mods::prepare_install(&app, &state, app_id, mod_ids, false).await
}

#[tauri::command]
pub async fn install_mods(
    app: AppHandle,
    state: State<'_, AppState>,
    plan_id: String,
) -> AppResult<GameSupport> {
    game_mods::execute_install(&app, &state, plan_id).await
}

#[tauri::command]
pub async fn prepare_mod_update(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
    mod_id: String,
) -> AppResult<ModActionPlan> {
    game_mods::prepare_install(&app, &state, app_id, vec![mod_id], true).await
}

#[tauri::command]
pub async fn update_mod(
    app: AppHandle,
    state: State<'_, AppState>,
    plan_id: String,
) -> AppResult<GameSupport> {
    game_mods::execute_install(&app, &state, plan_id).await
}

#[tauri::command]
pub async fn prepare_mod_uninstall(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
    mod_id: String,
    remove_config: bool,
) -> AppResult<ModActionPlan> {
    game_mods::prepare_mod_uninstall(&app, &state, app_id, mod_id, remove_config).await
}

#[tauri::command]
pub async fn uninstall_mod(
    app: AppHandle,
    state: State<'_, AppState>,
    plan_id: String,
) -> AppResult<GameSupport> {
    game_mods::uninstall_mod(&app, &state, plan_id).await
}

#[tauri::command]
pub async fn set_mod_config(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
    mod_id: String,
    changes: HashMap<String, Value>,
) -> AppResult<GameSupport> {
    game_mods::set_config(&app, &state, app_id, mod_id, changes).await
}
