use std::sync::Arc;

use tauri_plugin_updater::Update;
use tokio::sync::{Mutex, RwLock};

use crate::bepinex::BepInExInstallState;
use crate::config::model::AppConfig;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub bepinex: Arc<Mutex<BepInExInstallState>>,
    pub pending_update: Arc<Mutex<Option<Update>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Arc::new(RwLock::new(AppConfig::default())),
            bepinex: Arc::new(Mutex::new(BepInExInstallState::default())),
            pending_update: Arc::new(Mutex::new(None)),
        }
    }
}
