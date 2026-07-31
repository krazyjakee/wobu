//! The Tauri shell. Owns the window, the managed state, and the command
//! registration — nothing else.

mod commands;
mod diag;
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
    // Before the builder, so that a failure to start is itself recorded.
    diag::init(diag::dir());
    diag::info(&format!("wobu {} starting", env!("CARGO_PKG_VERSION")));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The launcher picks project folders with `@tauri-apps/plugin-dialog`;
        // `capabilities/default.json` already grants it, but the plugin still
        // has to be initialised on this side or every pick fails.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // Not another `.manage(Default::default())`: the queue reports
            // itself by emitting, and there is no `AppHandle` to emit through
            // until here. `state.rs` says why it sits beside `AppState` rather
            // than inside it.
            app.manage(state::Jobs::start(app.handle()));
            Ok(())
        })
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
            commands::project_open_cancel,
            commands::project_close,
            commands::project_current,
            commands::project_recent,
            commands::share_offline,
            commands::force_quit,
            commands::node_list,
            commands::corrupt_files,
            commands::project_reload,
            commands::node_search,
            commands::node_get,
            commands::node_create,
            commands::node_upsert,
            commands::node_delete,
            commands::node_move,
            commands::asset_import,
            commands::asset_import_bytes,
            commands::asset_list,
            commands::asset_link,
            commands::asset_unlink,
            commands::asset_link_update,
            commands::asset_set_cover,
            commands::influence_resolve,
            commands::prompt_compile,
            commands::conflicts,
            commands::conflict_resolve,
            commands::presence_peers,
            commands::presence_editing,
            commands::index_info,
            commands::index_rebuild,
            commands::about_info,
            commands::log_info,
            commands::log_set_level,
            commands::log_tail,
            commands::log_reveal,
            commands::job_cancel,
            commands::job_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
