pub mod agent;
pub mod bepinex;
pub mod commands;
pub mod config;
pub mod core;
pub mod game_mods;
pub mod steam;
pub mod updater;
pub mod version;

use crate::commands::app::{
    check_for_update, download_and_install_update, get_app_overview, get_language_preference,
    get_theme_preference, get_update_channel, set_language_preference, set_theme_preference,
    set_update_channel,
};
use crate::commands::bepinex::{
    install_bepinex, prepare_bepinex_install, prepare_bepinex_uninstall, uninstall_bepinex,
};
use crate::commands::game_mods::{
    get_game_support, install_mods, prepare_mod_install, prepare_mod_uninstall, prepare_mod_update,
    set_mod_config, uninstall_mod, update_mod,
};
use crate::commands::steam::list_steam_games;
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

            crate::agent::start_server(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_for_update,
            download_and_install_update,
            get_app_overview,
            get_language_preference,
            install_bepinex,
            install_mods,
            get_game_support,
            list_steam_games,
            prepare_bepinex_install,
            prepare_bepinex_uninstall,
            prepare_mod_install,
            prepare_mod_uninstall,
            prepare_mod_update,
            set_mod_config,
            get_theme_preference,
            get_update_channel,
            set_language_preference,
            set_theme_preference,
            set_update_channel,
            uninstall_bepinex,
            uninstall_mod,
            update_mod,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run GameTweaks");
}
