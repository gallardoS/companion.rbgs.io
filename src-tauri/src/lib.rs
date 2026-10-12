#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(pairing::PairingState::default())
        .invoke_handler(tauri::generate_handler![
            pairing::start_pairing,
            pairing::poll_pairing,
            pairing::connection_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to start rbgs.io companion");
}
mod pairing;
