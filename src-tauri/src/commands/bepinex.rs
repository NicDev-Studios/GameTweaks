use tauri::{AppHandle, State};

use crate::bepinex::{install, prepare_install, BepInExInstallPlan, BepInExInstallResult};
use crate::core::error::AppResult;
use crate::core::state::AppState;

#[tauri::command]
pub async fn prepare_bepinex_install(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: u32,
) -> AppResult<BepInExInstallPlan> {
    prepare_install(&app, &state, app_id).await
}

#[tauri::command]
pub async fn install_bepinex(
    app: AppHandle,
    state: State<'_, AppState>,
    plan_id: String,
) -> AppResult<BepInExInstallResult> {
    install(&app, &state, plan_id).await
}
