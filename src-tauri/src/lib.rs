//! The Tauri shell. Owns the window, the managed state, and the command
//! registration — nothing else.

mod commands;
mod error;
mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The launcher picks project folders with `@tauri-apps/plugin-dialog`;
        // `capabilities/default.json` already grants it, but the plugin still
        // has to be initialised on this side or every pick fails.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::kind_registry,
            commands::project_create,
            commands::project_open,
            commands::project_close,
            commands::project_current,
            commands::project_recent,
            commands::node_list,
            commands::node_get,
            commands::node_create,
            commands::node_upsert,
            commands::node_delete,
            commands::node_move,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
