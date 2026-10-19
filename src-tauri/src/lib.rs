#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            presence::show(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(pairing::PairingState::default())
        .manage(web_connect::WebConnectState::default())
        .manage(saved_variables::SavedVariablesState::default())
        .manage(saved_variables::transport::TransportState::default())
        .manage(assignments::AssignmentState::default())
        .setup(presence::setup)
        .invoke_handler(tauri::generate_handler![
            pairing::start_pairing,
            pairing::poll_pairing,
            pairing::open_pairing_browser,
            pairing::connection_status,
            web_connect::web_connection_state,
            saved_variables::saved_variables_status,
            saved_variables::saved_variables_update,
            saved_variables::history::match_history,
            saved_variables::transport::list_capture_assignments,
            assignments::assignment_activity,
            assignments::open_test_match,
            saved_variables::transport::issue_capture_assignment,
            saved_variables::transport::sync_captures,
            saved_variables::choose_forever_installation,
            saved_variables::scan_forever_installations,
            saved_variables::select_saved_variables_source
        ])
        .run(tauri::generate_context!())
        .expect("failed to start rbgs.io companion");
}
mod assignments;
mod pairing;
mod presence;
mod saved_variables;
#[cfg(desktop)]
mod tray;
mod web_connect;
