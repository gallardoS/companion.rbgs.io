use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;
use tauri_plugin_updater::UpdaterExt;

static RUNNING: AtomicBool = AtomicBool::new(false);
const ENDPOINT: &str =
    "https://github.com/gallardoS/companion.rbgs.io/releases/latest/download/latest.json";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    version: String,
    downloaded: u64,
    total: Option<u64>,
    installing: bool,
}

#[tauri::command]
pub(crate) async fn update_on_startup(
    app: tauri::AppHandle,
    progress: Channel<Progress>,
) -> Result<(), String> {
    // Development and unsigned local builds must never replace themselves.
    let Some(key) = option_env!("RBGS_UPDATER_PUBLIC_KEY").filter(|key| !key.trim().is_empty())
    else {
        return Ok(());
    };
    if cfg!(debug_assertions) || RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = run(&app, key, progress).await;
    RUNNING.store(false, Ordering::SeqCst);
    result
}

async fn run(app: &tauri::AppHandle, key: &str, progress: Channel<Progress>) -> Result<(), String> {
    let updater = app
        .updater_builder()
        .pubkey(key.trim())
        .endpoints(vec![ENDPOINT.parse().map_err(|_| "Invalid update URL")?])
        .map_err(|_| "Could not configure updates")?
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|_| "Could not configure updates")?;
    // Offline startup and releases predating latest.json should not block use.
    let Some(update) = updater.check().await.ok().flatten() else {
        return Ok(());
    };
    let mut state = Progress {
        version: update.version.clone(),
        downloaded: 0,
        total: None,
        installing: false,
    };
    progress
        .send(state.clone())
        .map_err(|_| "Update window unavailable")?;
    let bytes = update
        .download(
            |length, total| {
                state.downloaded += length as u64;
                state.total = total;
                let _ = progress.send(state.clone());
            },
            || {},
        )
        .await
        .map_err(|_| "The update could not be downloaded or verified. Please try again later.")?;
    // download() verifies the signature before handing bytes to the installer.
    state.installing = true;
    let _ = progress.send(state);
    update
        .install(bytes)
        .map_err(|_| "The update could not be installed. Please try again later.")?;
    // On Windows the updater exits this process and NSIS relaunches the app.
    Ok(())
}
