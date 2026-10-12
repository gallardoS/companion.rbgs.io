#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(pairing::PairingState::default())
        .manage(saved_variables::SavedVariablesState::default())
        .manage(saved_variables::transport::TransportState::default())
        .invoke_handler(tauri::generate_handler![
            pairing::start_pairing,
            pairing::poll_pairing,
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
mod saved_variables;
