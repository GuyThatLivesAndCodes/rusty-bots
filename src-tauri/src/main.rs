#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod ai;
mod bot;
mod commands;
mod automod;

use std::sync::Arc;
use tauri::Manager;
use crate::store::StoreHandle;
use crate::bot::BotRegistry;
use crate::commands::AppState;

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&data_dir).ok();
            let store_path = data_dir.join("bots.json");
            let store = Arc::new(StoreHandle::load(store_path).expect("load store"));
            let registry = Arc::new(BotRegistry::new());
            app.manage(AppState { store, registry });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_bots,
            commands::save_bot,
            commands::delete_bot,
            commands::start_bot_cmd,
            commands::stop_bot_cmd,
            commands::list_guilds,
            commands::list_guild_members,
            commands::list_guild_roles,
            commands::kick,
            commands::ban,
            commands::timeout,
            commands::add_role,
            commands::remove_role,
            commands::get_automod_config,
            commands::update_automod_config,
            commands::toggle_automod,
            commands::add_automod_rule,
            commands::remove_automod_rule,
            commands::set_automod_ignored_channels,
            commands::set_automod_ignored_roles,
            commands::set_automod_whitelist,
            commands::set_automod_log_channel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
