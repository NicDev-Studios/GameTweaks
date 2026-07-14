pub mod commands;
pub mod config;
pub mod core;
pub mod updater;

use crate::commands::app::{
    check_for_update, download_and_install_update, get_app_overview, get_language_preference,
    get_theme_preference, get_update_channel, set_language_preference, set_theme_preference,
    set_update_channel,
};
use crate::config::store::load_config;
use crate::core::state::AppState;
use tauri::Manager;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gametweaks=info,tauri=info".into()),
        )
        .init();

    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_opener::init())
                    .expect("failed to initialize opener plugin");
                app.handle()
                    .plugin(tauri_plugin_process::init())
                    .expect("failed to initialize process plugin");
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())
                    .expect("failed to initialize updater plugin");
            }

            let state = app.state::<AppState>();
            match load_config(app.handle()) {
                Ok(config) => tauri::async_runtime::block_on(async {
                    *state.config.write().await = config;
                }),
                Err(error) => tracing::warn!(message = %error.message, "failed to load config"),
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_for_update,
            download_and_install_update,
            get_app_overview,
            get_language_preference,
            get_theme_preference,
            get_update_channel,
            set_language_preference,
            set_theme_preference,
            set_update_channel,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run GameTweaks");
}
