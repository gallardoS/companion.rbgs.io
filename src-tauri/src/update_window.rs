use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const HELPER_NAME: &str = "rbgs-companion-update-window.exe";
const PROGRESS_FILE: &str = "progress.json";
const POPUP_READY: &str = "popup-ready";
const COMPANION_OPENED: &str = "companion-opened";
const POPUP_CLOSED: &str = "popup-closed";
const POLL_INTERVAL: Duration = Duration::from_millis(150);
const STALL_TIMEOUT: Duration = Duration::from_secs(150);

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Progress {
    pub version: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub installing: bool,
    pub failed: bool,
}

pub(crate) struct UpdateWindow {
    directory: PathBuf,
    child: Child,
}

fn session_root(app: &tauri::AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().app_cache_dir()?.join("update-windows"))
}

impl UpdateWindow {
    pub async fn open(app: &tauri::AppHandle, progress: &Progress) -> io::Result<Self> {
        let root = session_root(app).map_err(io::Error::other)?;
        fs::create_dir_all(&root)?;
        let directory = tempfile::Builder::new()
            .prefix("window-")
            .tempdir_in(root)?;
        let helper = directory.path().join(HELPER_NAME);
        // Windows must be free to replace the installed executable while this copy runs.
        fs::copy(std::env::current_exe()?, &helper)?;
        write_progress(directory.path(), progress)?;
        let mut child = Command::new(helper)
            .arg("--update-window")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if !matches!(child.try_wait(), Ok(None)) || Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::other("Update window did not become visible"));
            }
            if directory.path().join(POPUP_READY).is_file() {
                return Ok(Self {
                    directory: directory.keep(),
                    child,
                });
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    pub fn report(&self, progress: &Progress) -> io::Result<()> {
        write_progress(&self.directory, progress)
    }

    pub fn prepare_install(&mut self, progress: &Progress) -> Result<(), String> {
        if !matches!(self.child.try_wait(), Ok(None)) {
            return Err("The update window has closed.".into());
        }
        self.report(progress)
            .map_err(|_| "Could not prepare the update window.".into())
    }

    pub fn fail(&self, progress: &mut Progress) {
        progress.failed = true;
        let _ = self.report(progress);
    }
}

fn write_progress(directory: &Path, progress: &Progress) -> io::Result<()> {
    let temporary = directory.join("progress.tmp");
    fs::write(&temporary, serde_json::to_vec(progress)?)?;
    fs::rename(temporary, directory.join(PROGRESS_FILE))
}

fn read_progress(directory: &Path) -> Option<Progress> {
    let data = fs::read(directory.join(PROGRESS_FILE)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn sessions(root: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_string_lossy().starts_with("window-")
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .map(|entry| entry.path())
}

fn signal_opened(root: &Path, version: &str) {
    for directory in sessions(root) {
        if read_progress(&directory).is_some_and(|progress| {
            progress.installing && !progress.failed && progress.version == version
        }) {
            let _ = fs::write(directory.join(COMPANION_OPENED), []);
        }
    }
}

#[tauri::command]
pub(crate) async fn companion_window_ready(window: tauri::WebviewWindow) {
    if window.label() != "main"
        || !window.is_visible().unwrap_or(false)
        || window.is_minimized().unwrap_or(true)
    {
        return;
    }
    let app = window.app_handle();
    let Ok(root) = session_root(app) else { return };
    signal_opened(&root, &app.package_info().version.to_string());
    // The helper executable can only be removed once its process has exited.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        for directory in sessions(&root) {
            if directory.join(POPUP_CLOSED).is_file() {
                let _ = fs::remove_dir_all(directory);
            }
        }
    });
}

struct HelperDirectory(PathBuf);

fn helper_progress(directory: &Path) -> Option<Progress> {
    let mut progress = read_progress(directory)?;
    let stalled = fs::metadata(directory.join(PROGRESS_FILE))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed >= STALL_TIMEOUT);
    progress.failed |= stalled;
    Some(progress)
}

#[tauri::command]
fn update_window_progress(directory: tauri::State<'_, HelperDirectory>) -> Option<Progress> {
    helper_progress(&directory.0)
}

#[tauri::command]
fn update_window_ready(window: tauri::WebviewWindow, directory: tauri::State<'_, HelperDirectory>) {
    if window.is_visible().unwrap_or(false) {
        let _ = fs::write(directory.0.join(POPUP_READY), []);
    }
}

#[tauri::command]
fn close_update_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|error| error.to_string())
}

pub(crate) fn run(mut context: tauri::Context<tauri::Wry>) {
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .setup(|app| {
            let executable = std::env::current_exe()?;
            let directory = executable
                .parent()
                .ok_or_else(|| io::Error::other("Missing update window directory"))?
                .to_owned();
            let root = session_root(app.handle())?.canonicalize()?;
            if executable
                .file_name()
                .is_none_or(|name| name != HELPER_NAME)
                || directory.canonicalize()?.parent() != Some(root.as_path())
                || read_progress(&directory).is_none()
            {
                return Err(io::Error::other("Invalid update window session").into());
            }
            app.manage(HelperDirectory(directory.clone()));
            let window = WebviewWindowBuilder::new(
                app,
                "update-window",
                WebviewUrl::App("index.html?update-window".into()),
            )
            .title("Updating rbgs.io companion")
            .inner_size(440.0, 190.0)
            .center()
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .data_directory(directory.join("webview"))
            .build()?;
            let close_directory = directory.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if !helper_progress(&close_directory).is_some_and(|progress| progress.failed) {
                        api.prevent_close();
                    }
                }
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if directory.join(COMPANION_OPENED).is_file() {
                        handle.exit(0);
                        break;
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            update_window_progress,
            update_window_ready,
            close_update_window
        ])
        .build(context);
    if let Ok(app) = app {
        app.run(|handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(directory) = handle.try_state::<HelperDirectory>() {
                    let _ = fs::write(directory.0.join(POPUP_CLOSED), []);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_waits_for_the_target_version_after_installation_starts() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("window-test");
        fs::create_dir(&directory).unwrap();
        let mut progress = Progress {
            version: "0.0.39".into(),
            downloaded: 0,
            total: None,
            installing: false,
            failed: false,
        };
        write_progress(&directory, &progress).unwrap();
        signal_opened(root.path(), "0.0.39");
        assert!(!directory.join(COMPANION_OPENED).exists());

        progress.installing = true;
        write_progress(&directory, &progress).unwrap();
        signal_opened(root.path(), "0.0.38");
        assert!(!directory.join(COMPANION_OPENED).exists());

        progress.failed = true;
        write_progress(&directory, &progress).unwrap();
        signal_opened(root.path(), "0.0.39");
        assert!(!directory.join(COMPANION_OPENED).exists());

        progress.failed = false;
        write_progress(&directory, &progress).unwrap();
        signal_opened(root.path(), "0.0.39");
        assert!(directory.join(COMPANION_OPENED).exists());
    }
}
