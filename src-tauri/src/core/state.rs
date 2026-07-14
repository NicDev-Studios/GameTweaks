use std::sync::Arc;

use tauri_plugin_updater::Update;
use tokio::sync::{Mutex, RwLock};

use crate::config::model::AppConfig;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub pending_update: Arc<Mutex<Option<Update>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Arc::new(RwLock::new(AppConfig::default())),
            pending_update: Arc::new(Mutex::new(None)),
        }
    }
}
