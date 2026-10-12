#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            use tauri::Manager;
            if args.iter().any(|arg| presence::allowed_link(arg)) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(pairing::PairingState::default())
        .manage(saved_variables::SavedVariablesState::default())
        .manage(saved_variables::transport::TransportState::default())
        .setup(presence::setup)
        .invoke_handler(tauri::generate_handler![
            pairing::start_pairing,
            pairing::poll_pairing,
            pairing::open_pairing_browser,
            pairing::connection_status,
            saved_variables::saved_variables_status,
            saved_variables::saved_variables_update,
            saved_variables::history::match_history,
            saved_variables::transport::list_capture_assignments,
            saved_variables::transport::issue_capture_assignment,
            saved_variables::transport::sync_captures,
            saved_variables::choose_forever_installation,
            saved_variables::scan_forever_installations,
            saved_variables::select_saved_variables_source
        ])
        .run(tauri::generate_context!())
        .expect("failed to start rbgs.io companion");
}
mod pairing;
mod presence;
mod saved_variables;
