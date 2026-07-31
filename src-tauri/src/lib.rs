//! The Tauri shell. Owns the window, the managed state, and the command
//! registration — nothing else.

mod commands;
mod error;
mod redact;
mod state;

use tauri::{Emitter, Manager, WindowEvent};

use state::AppState;

/// Sent instead of closing when the share is away. The frontend turns this
/// into the "you would lose the edits you are holding" prompt.
const QUIT_BLOCKED: &str = "share:quit-blocked";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The launcher picks project folders with `@tauri-apps/plugin-dialog`;
        // `capabilities/default.json` already grants it, but the plugin still
        // has to be initialised on this side or every pick fails.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .on_window_event(|window, event| {
            // Quitting while the share is away would take any edit the user is
            // still holding with it, silently — the autosave has nowhere to put
            // it and the index is not canonical. So the first close request is
            // refused and handed to the UI to explain; `force_quit` is how the
            // user says they meant it.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.state::<AppState>().is_offline()
            {
                api.prevent_close();
                let _ = window.emit(QUIT_BLOCKED, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::kind_registry,
            commands::project_create,
            commands::project_open,
            commands::project_close,
            commands::project_current,
            commands::project_recent,
            commands::share_offline,
            commands::force_quit,
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
