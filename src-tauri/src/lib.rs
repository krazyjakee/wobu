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
    // Immediately after the log has a home, so a panic anywhere past this line
    // lands in the file rather than a stderr nobody reads. See
    // `diag::install_panic_hook`.
    diag::install_panic_hook();
    diag::info(format!("wobu {} starting", env!("CARGO_PKG_VERSION")));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The launcher picks project folders with `@tauri-apps/plugin-dialog`;
        // `capabilities/default.json` already grants it, but the plugin still
        // has to be initialised on this side or every pick fails.
        .plugin(tauri_plugin_dialog::init())
        // Checking for an update is driven from Settings, never on startup: a
        // world builder opening Wobu to work is not asking to talk to GitHub.
        // The plugin verifies every payload against the public key in
        // `tauri.conf.json` before it writes anything, so an endpoint that has
        // been tampered with produces a refusal rather than an install.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Only so the user can take the freshly installed version now rather
        // than on their next launch.
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        // Raw pasted-image chunks are staged outside the project until their
        // declared length arrives. Separate managed state lets Cancel and
        // project close tear down those files without involving the watcher.
        .manage(commands::assets::AssetTransfers::default())
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
            commands::project::kind_registry,
            commands::project::preset_list,
            commands::project::project_create,
            commands::project::project_open,
            commands::project::project_open_cancel,
            commands::project::project_close,
            commands::project::project_current,
            commands::project::project_recent,
            commands::project::project_recent_forget,
            commands::style::style_transfer_preview,
            commands::style::style_transfer_apply,
            commands::project::share_offline,
            commands::project::force_quit,
            commands::nodes::node_list,
            commands::nodes::node_links,
            commands::project::corrupt_files,
            commands::project::project_reload,
            commands::project::project_export_wiki,
            commands::nodes::node_search,
            commands::nodes::node_get,
            commands::nodes::node_create,
            commands::nodes::node_upsert,
            commands::nodes::node_seed_lock_set,
            commands::nodes::node_delete,
            commands::nodes::node_move,
            commands::nodes::node_link_add,
            commands::nodes::node_link_remove,
            commands::nodes::node_link_update,
            commands::nodes::node_backlinks,
            commands::assets::asset_import,
            commands::assets::asset_import_transfer_begin,
            commands::assets::asset_import_transfer_chunk,
            commands::assets::asset_import_transfer_finish,
            commands::assets::asset_import_transfer_cancel,
            commands::assets::asset_list,
            commands::assets::asset_usage_list,
            commands::assets::asset_delete,
            commands::generations::generation_list,
            commands::generations::mesh_concepts,
            commands::mesh::turnaround::turnaround_sheet,
            commands::mesh::options::mesh_options,
            commands::mesh::mesh_start,
            commands::generations::mesh_asset_path,
            commands::generations::mesh_source_path,
            commands::generations::mesh_export,
            commands::generations::generation_get,
            commands::generations::generation_delete,
            commands::thumbs::asset_thumb,
            commands::thumbs::asset_thumb_batch,
            commands::thumbs::node_thumb_batch,
            commands::assets::asset_original,
            commands::thumbs::asset_thumbs_ensure,
            commands::thumbs::asset_thumbs_cancel,
            commands::assets::asset_link,
            commands::assets::asset_unlink,
            commands::assets::asset_link_update,
            commands::assets::asset_set_cover,
            commands::influence::influence_resolve,
            commands::influence::prompt_compile,
            generate::generate_start,
            generate::scene::scene_generate_start,
            generate::replay::generation_replay,
            generate::preview::image_reference_report,
            generate::preview::image_generation_capabilities,
            generate::spend::spend_status,
            generate::spend::spend_ceiling_set,
            generate::spend::spend_recovery_reset,
            lora::lora_status,
            lora::lora_train_start,
            enhance::enhance_start,
            enhance::enhance_accept,
            enhance::enhance_discard,
            enhance::enhance_pending,
            commands::diagnostics::conflicts,
            commands::diagnostics::conflict_resolve,
            commands::diagnostics::presence_peers,
            commands::diagnostics::presence_editing,
            commands::diagnostics::index_info,
            commands::diagnostics::index_rebuild,
            commands::diagnostics::about_info,
            commands::diagnostics::log_info,
            commands::diagnostics::log_set_level,
            commands::diagnostics::log_tail,
            commands::diagnostics::log_reveal,
            commands::credentials::provider_key_status,
            commands::credentials::provider_key_set,
            commands::credentials::provider_key_delete,
            commands::credentials::provider_probe,
            machine::machine_settings,
            machine::comfyui_endpoint_set,
            machine::comfyui_endpoint_probe,
            machine::onboarding_state,
            machine::onboarding_accept_legal,
            machine::onboarding_finish,
            commands::settings::project_providers,
            commands::settings::project_provider_select,
            commands::settings::status_bar_backend,
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
