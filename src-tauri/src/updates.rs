use crate::update_window::{Progress, UpdateWindow};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use tokio::sync::Mutex;

#[derive(Default)]
pub(crate) struct UpdateState {
    operation: Mutex<()>,
    available: Mutex<Option<String>>,
}

const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
const AVAILABLE_EVENT: &str = "companion-update-available";
const ENDPOINT: &str =
    "https://github.com/gallardoS/companion.rbgs.io/releases/latest/download/latest.json";

pub(crate) fn start_checks(app: tauri::AppHandle) {
    if signing_key().is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(CHECK_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let state = app.state::<UpdateState>();
            if let Ok(_operation) = state.operation.try_lock() {
                let _ = check(&app, &state).await;
            }
        }
    });
}

fn signing_key() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        return None;
    }
    option_env!("RBGS_UPDATER_PUBLIC_KEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

fn updater(app: &tauri::AppHandle) -> Result<Updater, String> {
    let key = signing_key().ok_or("Updates are unavailable in this build.")?;
    app.updater_builder()
        .pubkey(key)
        .endpoints(vec![ENDPOINT.parse().map_err(|_| "Invalid update URL")?])
        .map_err(|_| "Could not configure updates")?
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| "Could not configure updates".into())
}

async fn check(app: &tauri::AppHandle, state: &UpdateState) -> Result<Option<Update>, String> {
    let update = updater(app)?
        .check()
        .await
        .map_err(|_| "Could not check for updates. Check your connection and try again.")?;
    let version = update.as_ref().map(|update| update.version.clone());
    let mut available = state.available.lock().await;
    if *available != version {
        let _ = app.emit(AVAILABLE_EVENT, &version);
        *available = version;
    }
    Ok(update)
}

#[tauri::command]
pub(crate) async fn available_update(
    state: tauri::State<'_, UpdateState>,
) -> Result<Option<String>, String> {
    Ok(state.available.lock().await.clone())
}

#[tauri::command]
pub(crate) async fn check_for_updates(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Result<Option<String>, String> {
    let _operation = state.operation.lock().await;
    Ok(check(&app, &state).await?.map(|update| update.version))
}

#[tauri::command]
pub(crate) async fn install_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Result<bool, String> {
    let _operation = state.operation.lock().await;
    let Some(update) = check(&app, &state).await? else {
        return Ok(false);
    };
    install(&app, update).await?;
    Ok(true)
}

async fn install(app: &tauri::AppHandle, update: Update) -> Result<(), String> {
    let mut state = Progress {
        version: update.version.clone(),
        downloaded: 0,
        total: None,
        installing: false,
        failed: false,
    };
    let mut window = UpdateWindow::open(app, &state)
        .await
        .map_err(|_| "The update window could not be opened.")?;
    let mut last_report = std::time::Instant::now();
    let bytes = match update
        .download(
            |length, total| {
                state.downloaded += length as u64;
                state.total = total;
                if last_report.elapsed() >= std::time::Duration::from_millis(150) {
                    let _ = window.report(&state);
                    last_report = std::time::Instant::now();
                }
            },
            || {},
        )
        .await
    {
        Ok(bytes) => bytes,
        Err(_) => {
            window.fail(&mut state);
            return Err("The update could not be downloaded or verified.".into());
        }
    };
    // download() verifies the signature before handing bytes to the installer.
    state.installing = true;
    if let Err(error) = window.prepare_install(&state) {
        window.fail(&mut state);
        return Err(error);
    }
    if update.install(bytes).is_err() {
        window.fail(&mut state);
        return Err("The update could not be installed.".into());
    }
    // On Windows the updater exits this process and NSIS relaunches the app.
    Ok(())
}
