use std::sync::Arc;

use tauri_plugin_updater::Update;
use tokio::sync::{Mutex, RwLock};

use crate::agent::AgentState;
use crate::bepinex::BepInExInstallState;
use crate::config::model::AppConfig;
use crate::game_mods::GameModsState;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub bepinex: Arc<Mutex<BepInExInstallState>>,
    pub game_mods: Arc<Mutex<GameModsState>>,
    pub agent: Arc<Mutex<AgentState>>,
    pub pending_update: Arc<Mutex<Option<Update>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Arc::new(RwLock::new(AppConfig::default())),
            bepinex: Arc::new(Mutex::new(BepInExInstallState::default())),
            game_mods: Arc::new(Mutex::new(GameModsState::default())),
            agent: Arc::new(Mutex::new(AgentState::default())),
            pending_update: Arc::new(Mutex::new(None)),
        }
    }
}
