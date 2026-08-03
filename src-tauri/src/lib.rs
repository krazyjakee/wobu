//! The Tauri shell. Owns the window, the managed state, and the command
//! registration — nothing else.

mod commands;
mod diag;
mod enhance;
mod error;
mod generate;
mod keys;
mod lora;
mod machine;
mod mcp;
mod redact;
mod shutdown;
mod state;
mod sync;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use state::AppState;

/// Sent instead of closing when the share is away. The frontend turns this
/// into the "you would lose the edits you are holding" prompt.
const QUIT_BLOCKED: &str = "share:quit-blocked";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before the builder, so that a failure to start is itself recorded.
    diag::init(diag::dir());
    diag::info(format!("wobu {} starting", env!("CARGO_PKG_VERSION")));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The launcher picks project folders with `@tauri-apps/plugin-dialog`;
        // `capabilities/default.json` already grants it, but the plugin still
        // has to be initialised on this side or every pick fails.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        // Raw pasted-image chunks are staged outside the project until their
        // declared length arrives. Separate managed state lets Cancel and
        // project close tear down those files without involving the watcher.
        .manage(commands::AssetTransfers::default())
        // Beside `AppState` rather than inside it, and that is the point: keys
        // belong to the installation, not to the open project. A type that could
        // reach a project folder is a type that could one day read a key out of
        // one, and project folders live on shares.
        .manage(keys::Keys::default())
        // A ComfyUI route is per installation for the same reason a key is:
        // collaborators share provider choices, not each other's machines.
        .manage(machine::MachineSettings::default())
        // Descriptions that have come back from a provider and not yet been
        // accepted, edited or rejected. Beside the project rather than inside
        // it for the same reason the queue is: the call that produced one
        // outlives the project it was started for, and each entry records which
        // project it belongs to so an accept can never be answered against a
        // different world.
        .manage(enhance::Pending::default())
        // Beside `AppState` rather than inside it, and that is #82's whole
        // point: that slot holds exactly one project and only while somebody has
        // it open, and syncing worlds nobody is looking at is the feature.
        .manage(sync::SyncState::default())
        // Constructing this reads `mcp.json` and nothing else — no socket is
        // bound and no process is spawned here, because at this point nothing
        // has consulted a setting. `mcp::init` in `setup` is the only thing that
        // can start either, and only when the stored settings say a person
        // turned it on. Beside `AppState` for the same reason a ComfyUI endpoint
        // is: a port and a command line belong to this machine, not to a world
        // that lives on a share.
        .manage(mcp::McpState::default())
        .setup(|app| {
            // Not another `.manage(Default::default())`: the queue reports
            // itself by emitting, and there is no `AppHandle` to emit through
            // until here. `state.rs` says why it sits beside `AppState` rather
            // than inside it.
            app.manage(state::Jobs::start(app.handle()));
            // Loads the peer identity and names this installation *before*
            // returning — a conflict sibling written before the alias is in
            // place carries `wobu-store`'s per-process fallback name, and
            // `peer::install` refuses a second value rather than replacing the
            // first. Binding the endpoint is the slow half and happens on a
            // task; every sync command answers "still starting" until it lands.
            let state = app.state::<AppState>().handle();
            app.state::<sync::SyncState>().start(app.handle(), state);
            // Hands the MCP module the project slot and the handle it emits
            // through. Starts a loopback listener only if the settings file
            // already said so — which it does not until somebody has ticked the
            // box in Settings → Agent access. See `docs/16-mcp.md`.
            mcp::init(app.handle(), app.state::<AppState>().handle());
            // A logout or a `systemctl stop` never reaches the window event
            // below, so without this the whole exit policy would apply to
            // exactly one of the ways this process can be asked to stop.
            // `docs/15-exit-policy.md` enumerates the rest.
            shutdown::install_signal_handlers(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Quitting while the share is away would take any edit the user is
            // still holding with it, silently — the autosave has nowhere to put
            // it and the index is not canonical. So the first close request is
            // refused and handed to the UI to explain; `force_quit` is how the
            // user says they meant it.
            //
            // This is a backstop rather than the gate. The renderer refuses the
            // same request in `useSafeWindowClose`, settles every editor write
            // and warns about unfinished jobs before it lets the window go —
            // and it is the only half that can, because the pending keystroke
            // lives over there. What this covers is a webview that is not
            // listening: one that crashed, or reloaded mid-close.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.state::<AppState>().is_offline()
            {
                api.prevent_close();
                let _ = window.emit(QUIT_BLOCKED, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::kind_registry,
            commands::preset_list,
            commands::project_create,
            commands::project_open,
            commands::project_open_cancel,
            commands::project_close,
            commands::project_current,
            commands::project_recent,
            commands::project_recent_forget,
            commands::style_transfer_preview,
            commands::style_transfer_apply,
            commands::share_offline,
            commands::force_quit,
            commands::node_list,
            commands::node_links,
            commands::corrupt_files,
            commands::project_reload,
            commands::project_export_wiki,
            commands::node_search,
            commands::node_get,
            commands::node_create,
            commands::node_upsert,
            commands::node_seed_lock_set,
            commands::node_delete,
            commands::node_move,
            commands::node_link_add,
            commands::node_link_remove,
            commands::node_link_update,
            commands::node_backlinks,
            commands::asset_import,
            commands::asset_import_transfer_begin,
            commands::asset_import_transfer_chunk,
            commands::asset_import_transfer_finish,
            commands::asset_import_transfer_cancel,
            commands::asset_list,
            commands::asset_usage_list,
            commands::asset_delete,
            commands::generation_list,
            commands::mesh_concepts,
            commands::mesh::turnaround_sheet,
            commands::mesh::mesh_options,
            commands::mesh::mesh_start,
            commands::mesh_asset_path,
            commands::mesh_source_path,
            commands::mesh_export,
            commands::generation_get,
            commands::generation_delete,
            commands::asset_thumb,
            commands::asset_thumb_batch,
            commands::node_thumb_batch,
            commands::asset_original,
            commands::asset_thumbs_ensure,
            commands::asset_thumbs_cancel,
            commands::asset_link,
            commands::asset_unlink,
            commands::asset_link_update,
            commands::asset_set_cover,
            commands::influence_resolve,
            commands::prompt_compile,
            generate::generate_start,
            generate::scene_generate_start,
            generate::generation_replay,
            generate::image_reference_report,
            generate::image_generation_capabilities,
            generate::spend_status,
            generate::spend_ceiling_set,
            generate::spend_recovery_reset,
            lora::lora_status,
            lora::lora_train_start,
            enhance::enhance_start,
            enhance::enhance_accept,
            enhance::enhance_discard,
            enhance::enhance_pending,
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
            commands::provider_key_status,
            commands::provider_key_set,
            commands::provider_key_delete,
            commands::provider_probe,
            machine::machine_settings,
            machine::comfyui_endpoint_set,
            machine::comfyui_endpoint_probe,
            machine::onboarding_state,
            machine::onboarding_accept_legal,
            machine::onboarding_finish,
            commands::project_providers,
            commands::project_provider_select,
            commands::status_bar_backend,
            commands::job_cancel,
            commands::job_list,
            sync::sync_status,
            sync::sync_share,
            sync::sync_accept,
            sync::sync_unshare,
            mcp::mcp_settings,
            mcp::mcp_server_set,
            mcp::mcp_server_token,
            mcp::mcp_server_token_rotate,
            mcp::mcp_activity,
            mcp::mcp_client_set,
            mcp::mcp_client_server_upsert,
            mcp::mcp_client_server_remove,
            mcp::mcp_client_server_probe,
            mcp::mcp_client_call,
        ])
        // `build` + `run` rather than `run` alone, and the difference is one
        // event. `SyncEndpoint` holds iroh's `Router`, which is `#[must_use]`
        // and aborts its accept loop when dropped — so letting the process fall
        // over the end of `main` would sever every inbound connection silently,
        // at whatever point in a transfer it happened to be. `RunEvent::Exit` is
        // the last moment there is still a runtime to wind any of it down on,
        // and the job queue is in the same position: a cancelled provider call
        // reports what it was billed, a killed one does not.
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, RunEvent::Exit) {
                // Before `wind_down`, and cheap: closing the MCP port and
                // killing the stdio servers the user configured takes no
                // network round trip and no lock this process holds. A Wobu
                // that exited leaving four of somebody's language servers
                // running would be a Wobu people stop enabling this for.
                mcp::shut_down(app);
                // On the main thread, with no locks held. Both halves matter: a
                // hang here is a window that will not close, and a shutdown that
                // ran while holding the project mutex would wait for a round
                // that is waiting for the mutex. Every stage in `shutdown.rs`
                // carries its own deadline for the case that reasoning is
                // wrong.
                shutdown::wind_down(app);
            }
        });
}
