pub mod history;
pub(crate) mod inbox;
mod parser;

use parser::{Compatibility, MAX_BYTES, ParseError, Summary};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

const ADDON: &str = "RBGSCompatibility";
const DATA_FILE: &str = "RBGSCompatibility.lua";
const MAX_ACCOUNTS: usize = 64;
const MAX_CONFIG_BYTES: usize = 8192;
const MAX_TOC_BYTES: usize = 16_384;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    NotConfigured,
    WrongClient,
    MissingAddon,
    UnsupportedAddon,
    NoSavedData,
    SelectSource,
    SourceUnavailable,
    UnstableFile,
    TooLarge,
    InvalidData,
    UnsupportedSchema,
    UnsupportedClient,
    UnknownClient,
    Ready,
}

impl Status {
    fn message(self) -> &'static str {
        match self {
            Self::NotConfigured => {
                "Choose your Forever beta client folder or its World of Warcraft parent folder."
            }
            Self::WrongClient => {
                "This is not the supported Forever beta installation. Choose the folder containing the beta client."
            }
            Self::MissingAddon => {
                "RBGSCompatibility is not installed. Install the diagnostic addon, enable it in-game, then reload or log out."
            }
            Self::UnsupportedAddon => {
                "The installed addon metadata is unsupported. Use the diagnostic addon for interface 16001 with RBGSCompatibilityData."
            }
            Self::NoSavedData => {
                "No diagnostic SavedVariables source is available yet. Enable RBGSCompatibility, log into a character, then use /reload or log out and refresh."
            }
            Self::SelectSource => {
                "Several account sources are available. Choose the account used by your Forever character."
            }
            Self::SourceUnavailable => {
                "The chosen source is unavailable or unsafe to read. Check the installation, refresh the sources, or choose another folder."
            }
            Self::UnstableFile => {
                "The file is being written or replaced. Wait for /reload or logout to finish, then refresh."
            }
            Self::TooLarge => {
                "This diagnostic file exceeds the 2 MB reading limit. Keep a private backup and review addon retention before retrying; no data was changed."
            }
            Self::InvalidData => {
                "The saved file is incomplete, malformed or outside the bounded data-only Lua format. Finish /reload or logout and refresh; no Lua was executed."
            }
            Self::UnsupportedSchema => {
                "This SavedVariables schema is unsupported. Check the addon/companion versions."
            }
            Self::UnsupportedClient => {
                "The saved client build has not been checked with this diagnostic reader. Data is shown for inspection only."
            }
            Self::UnknownClient => {
                "No usable client build was saved. Reload or log out with the diagnostic addon enabled, then refresh."
            }
            Self::Ready => {
                "Saved diagnostic data read successfully. These are persisted observations, not verified match evidence."
            }
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceOption {
    id: String,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    installation: Option<String>,
    sources: Vec<SourceOption>,
    selected_source: Option<String>,
    status: Status,
    message: &'static str,
    data: Option<Summary>,
    file_modified_at: Option<u64>,
    read_at: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    version: u8,
    installation: PathBuf,
    account: Option<String>,
}

#[derive(Default)]
struct SourceManager {
    loaded: bool,
    config: Option<Config>,
    watch: WatchCursor,
    history: history::HistoryCache,
}

#[derive(Debug, PartialEq, Eq)]
struct WatchSample {
    source: Option<(PathBuf, Option<String>)>,
    content: Result<Vec<u8>, Status>,
    modified: Option<SystemTime>,
}

#[derive(Default)]
struct WatchCursor {
    accepted: Option<WatchSample>,
    pending: Option<WatchSample>,
}

impl WatchCursor {
    fn changed_and_settled(&mut self, sample: WatchSample) -> bool {
        if self.accepted.as_ref() == Some(&sample) {
            self.pending = None;
            return false;
        }
        if self.pending.as_ref() == Some(&sample) {
            return true;
        }
        self.pending = Some(sample);
        false
    }
    fn accept(&mut self) {
        self.accepted = self.pending.take();
    }
}

#[derive(Clone, Default)]
pub struct SavedVariablesState(Arc<Mutex<SourceManager>>);

fn regular(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        } // FILE_ATTRIBUTE_REPARSE_POINT
    }
    true
}

fn child(root: &Path, relative: &Path, directory: bool) -> Result<PathBuf, Status> {
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Status::SourceUnavailable);
    }
    let mut path = root.to_owned();
    for component in relative.components() {
        path.push(component);
        let metadata = fs::symlink_metadata(&path).map_err(|_| Status::SourceUnavailable)?;
        if !regular(&metadata) {
            return Err(Status::SourceUnavailable);
        }
    }
    let resolved = fs::canonicalize(&path).map_err(|_| Status::SourceUnavailable)?;
    if !resolved.starts_with(root) {
        return Err(Status::SourceUnavailable);
    }
    let metadata = fs::metadata(&resolved).map_err(|_| Status::SourceUnavailable)?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(Status::SourceUnavailable);
    }
    Ok(resolved)
}

fn read_bounded(path: &Path, maximum: usize) -> Result<(Vec<u8>, Metadata), Status> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Status::SourceUnavailable)?;
    if !regular(&metadata) || !metadata.is_file() {
        return Err(Status::SourceUnavailable);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|_| Status::SourceUnavailable)?;
    let metadata = file.metadata().map_err(|_| Status::SourceUnavailable)?;
    if !regular(&metadata) || !metadata.is_file() {
        return Err(Status::SourceUnavailable);
    }
    if metadata.len() > maximum as u64 {
        return Err(Status::TooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Status::SourceUnavailable)?;
    if bytes.len() > maximum {
        return Err(Status::TooLarge);
    }
    if bytes.len() as u64 != metadata.len() {
        return Err(Status::UnstableFile);
    }
    Ok((bytes, metadata))
}

fn validate_installation(chosen: &Path) -> Result<PathBuf, Status> {
    let candidate = if chosen.join(".flavor.info").exists() {
        chosen.to_owned()
    } else {
        chosen.join("_classic_beta_")
    };
    let metadata = fs::symlink_metadata(&candidate).map_err(|_| Status::WrongClient)?;
    if !regular(&metadata) || !metadata.is_dir() {
        return Err(Status::WrongClient);
    }
    let root = fs::canonicalize(candidate).map_err(|_| Status::WrongClient)?;
    let flavor = child(&root, Path::new(".flavor.info"), false).map_err(|_| Status::WrongClient)?;
    let flavor = read_bounded(&flavor, 4096)
        .map_err(|_| Status::WrongClient)?
        .0;
    let flavor = std::str::from_utf8(&flavor)
        .map_err(|_| Status::WrongClient)?
        .trim_start_matches('\u{feff}')
        .trim();
    let lines: Vec<_> = flavor.lines().map(str::trim).collect();
    if lines != ["wow_classic_beta"] && lines != ["Product Flavor!STRING:0", "wow_classic_beta"] {
        return Err(Status::WrongClient);
    }
    child(&root, Path::new("WowB.exe"), false).map_err(|_| Status::WrongClient)?;
    Ok(root)
}

fn check_addon(root: &Path) -> Result<(), Status> {
    let relative = Path::new("Interface").join("AddOns").join(ADDON);
    child(root, &relative.join("RBGSCompatibility.lua"), false)
        .map_err(|_| Status::MissingAddon)?;
    let toc = child(root, &relative.join("RBGSCompatibility.toc"), false)
        .map_err(|_| Status::MissingAddon)?;
    let bytes = read_bounded(&toc, MAX_TOC_BYTES)
        .map_err(|_| Status::UnsupportedAddon)?
        .0;
    let text = std::str::from_utf8(&bytes).map_err(|_| Status::UnsupportedAddon)?;
    let interface = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("## Interface:").map(str::trim));
    let variables = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("## SavedVariables:")
            .map(str::trim)
    });
    if interface != Some("16001") || variables != Some("RBGSCompatibilityData") {
        return Err(Status::UnsupportedAddon);
    }
    Ok(())
}

fn default_installations() -> Vec<PathBuf> {
    // Ordered fallbacks also work when a 32-bit process expands ProgramFiles differently.
    let mut roots = Vec::new();
    for base in [
        std::env::var_os("ProgramFiles(x86)"),
        Some(r"C:\Program Files (x86)".into()),
        std::env::var_os("ProgramW6432"),
        std::env::var_os("ProgramFiles"),
        Some(r"C:\Program Files".into()),
    ]
    .into_iter()
    .flatten()
    {
        let path = PathBuf::from(base).join("World of Warcraft");
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
    roots
}

fn discover_default(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find_map(|path| validate_installation(path).ok())
}

fn scan_installations(folder: &Path) -> Result<Vec<PathBuf>, String> {
    const MAX_DIRECTORIES: usize = 10_000;
    const MAX_ENTRIES: usize = 100_000;
    const MAX_DEPTH: usize = 6;
    let metadata = fs::symlink_metadata(folder).map_err(|_| "The scan folder is unavailable")?;
    if !regular(&metadata) || !metadata.is_dir() {
        return Err("Choose a regular local folder to scan".into());
    }
    let root = fs::canonicalize(folder).map_err(|_| "The scan folder is unavailable")?;
    let mut pending = vec![(root, 0)];
    let mut found = Vec::new();
    let mut visited = 0;
    let mut entries_seen = 0;
    let started = std::time::Instant::now();
    while let Some((directory, depth)) = pending.pop() {
        visited += 1;
        if visited > MAX_DIRECTORIES || started.elapsed() > Duration::from_secs(10) {
            return Err(
                "Scan limit reached. Choose a smaller folder or select the installation directly."
                    .into(),
            );
        }
        if let Ok(client) = validate_installation(&directory) {
            if !found.contains(&client) {
                found.push(client);
            }
            // Do not inspect account files or addon trees during discovery.
            continue;
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            entries_seen += 1;
            if entries_seen > MAX_ENTRIES || started.elapsed() > Duration::from_secs(10) {
                return Err("Scan limit reached. Choose a smaller folder or select the installation directly.".into());
            }
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if regular(&metadata) && metadata.is_dir() {
                if pending.len() >= MAX_DIRECTORIES {
                    return Err("Scan limit reached. Choose a smaller folder or select the installation directly.".into());
                }
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn sources(root: &Path) -> Result<Vec<SourceOption>, Status> {
    let directory = match child(root, Path::new("WTF/Account"), true) {
        Ok(path) => path,
        Err(_) => return Ok(Vec::new()),
    };
    let mut sources = Vec::new();
    let entries = fs::read_dir(directory).map_err(|_| Status::SourceUnavailable)?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ACCOUNTS {
            return Err(Status::SourceUnavailable);
        }
        let entry = entry.map_err(|_| Status::SourceUnavailable)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.len() > 128 || name.chars().any(char::is_control) {
            continue;
        }
        let relative = Path::new("WTF/Account").join(&name).join("SavedVariables");
        if child(root, &relative, true).is_ok() {
            sources.push(SourceOption {
                id: name.clone(),
                label: name,
            });
        }
    }
    sources.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(sources)
}

fn stable_read(root: &Path, account: &str) -> Result<(Summary, Metadata), Status> {
    read_with_pause(root, account, || {
        std::thread::sleep(Duration::from_millis(250))
    })
}

fn read_with_pause(
    root: &Path,
    account: &str,
    pause: impl FnOnce(),
) -> Result<(Summary, Metadata), Status> {
    let relative = Path::new("WTF/Account")
        .join(account)
        .join("SavedVariables")
        .join(DATA_FILE);
    // Validate containment again for each sample, including after a file replacement.
    let path = child(root, &relative, false)?;
    let (first, metadata) = read_bounded(&path, MAX_BYTES)?;
    pause();
    let path_after = child(root, &relative, false).map_err(|_| Status::UnstableFile)?;
    let (second, after) = read_bounded(&path_after, MAX_BYTES).map_err(|status| {
        if status == Status::SourceUnavailable {
            Status::UnstableFile
        } else {
            status
        }
    })?;
    if path != path_after || first != second || metadata.modified().ok() != after.modified().ok() {
        return Err(Status::UnstableFile);
    }
    let data = parser::parse(&second).map_err(|error| match error {
        ParseError::TooLarge => Status::TooLarge,
        ParseError::UnsupportedSchema => Status::UnsupportedSchema,
        _ => Status::InvalidData,
    })?;
    Ok((data, after))
}

fn seconds(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

impl SourceManager {
    fn load(&mut self, config_path: &Path) -> Result<(), String> {
        if self.loaded {
            return Ok(());
        }
        if config_path.exists() {
            let bytes = read_bounded(config_path, MAX_CONFIG_BYTES)
                .map_err(
                    |_| "Local source settings could not be read. Choose the installation again.",
                )?
                .0;
            let config: Config = serde_json::from_slice(&bytes)
                .map_err(|_| "Local source settings are invalid. Choose the installation again.")?;
            if config.version != 1 {
                return Err(
                    "Local source settings version is unsupported. Choose the installation again."
                        .into(),
                );
            }
            self.config = Some(config);
        }
        self.loaded = true;
        Ok(())
    }
    fn save_config(config: &Config, config_path: &Path) -> Result<(), String> {
        let parent = config_path
            .parent()
            .ok_or("Local settings directory is unavailable")?;
        fs::create_dir_all(parent).map_err(|_| "Local settings directory could not be created")?;
        let bytes =
            serde_json::to_vec(config).map_err(|_| "Local source settings could not be encoded")?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err("The selected source path is too long to save".into());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Local clock unavailable")?
            .as_nanos();
        let temporary = parent.join(format!("source-{}-{nonce}.tmp", std::process::id()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, config_path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|_| "Local source settings could not be saved".into())
    }
    fn choose(&mut self, folder: &Path, config_path: &Path) -> Result<View, String> {
        let root = validate_installation(folder).map_err(|status| status.message())?;
        let available = sources(&root).map_err(|status| status.message())?;
        let account = if available.len() == 1 {
            Some(available[0].id.clone())
        } else {
            None
        };
        let config = Config {
            version: 1,
            installation: root,
            account,
        };
        Self::save_config(&config, config_path)?;
        self.config = Some(config);
        self.loaded = true;
        self.watch = WatchCursor::default();
        Ok(self.view())
    }
    fn select(&mut self, id: &str, config_path: &Path) -> Result<View, String> {
        let mut config = self.config.clone().ok_or("Choose the installation first")?;
        let root =
            validate_installation(&config.installation).map_err(|status| status.message())?;
        if !sources(&root)
            .map_err(|status| status.message())?
            .iter()
            .any(|source| source.id == id)
        {
            return Err(
                "The selected source is no longer available. Refresh the source list.".into(),
            );
        }
        config.account = Some(id.into());
        Self::save_config(&config, config_path)?;
        self.config = Some(config);
        self.watch = WatchCursor::default();
        Ok(self.view())
    }
    fn watch_sample(&self) -> WatchSample {
        let mut sample = WatchSample {
            source: self
                .config
                .as_ref()
                .map(|config| (config.installation.clone(), config.account.clone())),
            content: Err(Status::NotConfigured),
            modified: None,
        };
        sample.content = (|| {
            let config = self.config.as_ref().ok_or(Status::NotConfigured)?;
            let root = validate_installation(&config.installation)?;
            check_addon(&root)?;
            let available = sources(&root)?;
            if available.is_empty() {
                return Err(Status::NoSavedData);
            }
            let account = config.account.as_ref().ok_or(Status::SelectSource)?;
            if !available.iter().any(|source| &source.id == account) {
                return Err(Status::SourceUnavailable);
            }
            let relative = Path::new("WTF/Account")
                .join(account)
                .join("SavedVariables")
                .join(DATA_FILE);
            if !root.join(&relative).exists() {
                return Err(Status::NoSavedData);
            }
            let path = child(&root, &relative, false)?;
            let (content, metadata) = read_bounded(&path, MAX_BYTES)?;
            sample.modified = metadata.modified().ok();
            Ok(content)
        })();
        sample
    }
    fn update(&mut self) -> Option<View> {
        let sample = self.watch_sample();
        if !self.watch.changed_and_settled(sample) {
            return None;
        }
        let view = self.view();
        // Retry a changing/partial file even without another notification.
        // Invalid stable content is reported once and retried when its bytes change.
        if view.status != Status::UnstableFile {
            self.watch.accept();
        }
        Some(view)
    }
    fn view(&self) -> View {
        let mut view = View {
            installation: self
                .config
                .as_ref()
                .map(|c| c.installation.to_string_lossy().into()),
            sources: Vec::new(),
            selected_source: self.config.as_ref().and_then(|c| c.account.clone()),
            status: Status::NotConfigured,
            message: Status::NotConfigured.message(),
            data: None,
            file_modified_at: None,
            read_at: None,
        };
        let result = (|| {
            let Some(config) = &self.config else {
                return Err(Status::NotConfigured);
            };
            let root = validate_installation(&config.installation)?;
            check_addon(&root)?;
            view.sources = sources(&root)?;
            if view.sources.is_empty() {
                return Err(Status::NoSavedData);
            }
            let Some(account) = &config.account else {
                return Err(Status::SelectSource);
            };
            if !view.sources.iter().any(|source| &source.id == account) {
                return Err(Status::SourceUnavailable);
            }
            let relative = Path::new("WTF/Account")
                .join(account)
                .join("SavedVariables")
                .join(DATA_FILE);
            if !root.join(&relative).exists() {
                return Err(Status::NoSavedData);
            }
            let (summary, metadata) = stable_read(&root, account)?;
            let status = match summary.compatibility {
                Compatibility::ObservedDiagnosticBuild => Status::Ready,
                Compatibility::UnsupportedClient => Status::UnsupportedClient,
                Compatibility::UnknownClient => Status::UnknownClient,
            };
            view.file_modified_at = metadata.modified().ok().and_then(seconds);
            view.read_at = seconds(SystemTime::now());
            view.data = Some(summary);
            Ok(status)
        })();
        view.status = result.unwrap_or_else(|status| status);
        view.message = view.status.message();
        view
    }
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("saved-variables-source.json"))
        .map_err(|_| "Local settings directory is unavailable".into())
}

#[tauri::command]
pub async fn saved_variables_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, SavedVariablesState>,
) -> Result<View, String> {
    let path = settings_path(&app)?;
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut manager = state
            .lock()
            .map_err(|_| "Local source state is unavailable")?;
        manager.load(&path)?;
        if manager.config.is_none() {
            if let Some(folder) = discover_default(&default_installations()) {
                return manager.choose(&folder, &path);
            }
        }
        Ok(manager.view())
    })
    .await
    .map_err(|_| "Local source reader could not finish")?
}

#[tauri::command]
pub async fn saved_variables_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, SavedVariablesState>,
) -> Result<Option<View>, String> {
    let path = settings_path(&app)?;
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut manager = state
            .lock()
            .map_err(|_| "Local source state is unavailable")?;
        manager.load(&path)?;
        Ok(manager.update())
    })
    .await
    .map_err(|_| "Automatic saved-data reader could not finish")?
}

#[tauri::command]
pub async fn scan_forever_installations(
    app: tauri::AppHandle,
    state: tauri::State<'_, SavedVariablesState>,
) -> Result<Option<View>, String> {
    let path = settings_path(&app)?;
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(folder) = app.dialog().file()
            .set_title("Scan folders for Forever — choose a drive or games folder")
            .blocking_pick_folder() else { return Ok(None); };
        let folder = folder.into_path().map_err(|_| "Only a local scan folder is supported")?;
        let found = scan_installations(&folder)?;
        let installation = match found.as_slice() {
            [installation] => installation,
            [] => return Err("No Forever beta installation found within six folder levels. Choose another scan folder or select the installation directly.".into()),
            _ => return Err("Several Forever installations found. Choose a smaller scan folder or select the installation directly.".into()),
        };
        let mut manager = state.lock().map_err(|_| "Local source state is unavailable")?;
        manager.choose(installation, &path).map(Some)
    }).await.map_err(|_| "Installation scan could not finish")?
}

#[tauri::command]
pub async fn choose_forever_installation(
    app: tauri::AppHandle,
    state: tauri::State<'_, SavedVariablesState>,
) -> Result<Option<View>, String> {
    let path = settings_path(&app)?;
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let folder = app
            .dialog()
            .file()
            .set_title("Choose the Forever beta client folder or World of Warcraft folder")
            .blocking_pick_folder();
        let Some(folder) = folder else {
            return Ok(None);
        };
        let folder = folder
            .into_path()
            .map_err(|_| "Only a local installation folder is supported")?;
        let mut manager = state
            .lock()
            .map_err(|_| "Local source state is unavailable")?;
        manager.choose(&folder, &path).map(Some)
    })
    .await
    .map_err(|_| "Installation selection could not finish")?
}

#[tauri::command]
pub async fn select_saved_variables_source(
    app: tauri::AppHandle,
    state: tauri::State<'_, SavedVariablesState>,
    source_id: String,
) -> Result<View, String> {
    let path = settings_path(&app)?;
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut manager = state
            .lock()
            .map_err(|_| "Local source state is unavailable")?;
        manager.load(&path)?;
        manager.select(&source_id, &path)
    })
    .await
    .map_err(|_| "Source selection could not finish")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    const EMPTY: &[u8] = b"RBGSCompatibilityData={schema=1,records={}}";
    #[test]
    fn discovery_validates_candidates_and_preserves_the_configured_source() {
        let invalid = tempfile::tempdir().unwrap();
        let first = installation();
        let second = installation();
        let candidates = vec![
            invalid.path().to_owned(),
            first.path().to_owned(),
            second.path().to_owned(),
        ];
        assert_eq!(
            discover_default(&candidates),
            Some(fs::canonicalize(first.path()).unwrap())
        );
        assert!(discover_default(&candidates[..1]).is_none());
        let settings = tempfile::tempdir().unwrap();
        let path = settings.path().join("source.json");
        let mut manager = SourceManager::default();
        manager.choose(second.path(), &path).unwrap();
        manager.load(&path).unwrap();
        assert_eq!(
            manager.config.as_ref().unwrap().installation,
            fs::canonicalize(second.path()).unwrap()
        );
    }
    #[test]
    fn scan_finds_nested_clients_deduplicates_parent_and_limits_depth() {
        let container = tempfile::tempdir().unwrap();
        let fixture = installation();
        let client = container
            .path()
            .join("Games/World of Warcraft/_classic_beta_");
        fs::create_dir_all(&client).unwrap();
        fs::copy(
            fixture.path().join(".flavor.info"),
            client.join(".flavor.info"),
        )
        .unwrap();
        fs::copy(fixture.path().join("WowB.exe"), client.join("WowB.exe")).unwrap();
        assert_eq!(
            scan_installations(container.path()).unwrap(),
            vec![fs::canonicalize(&client).unwrap()]
        );
        let too_deep = container.path().join("a/b/c/d/e/f/g/client");
        fs::create_dir_all(&too_deep).unwrap();
        fs::copy(
            fixture.path().join(".flavor.info"),
            too_deep.join(".flavor.info"),
        )
        .unwrap();
        fs::copy(fixture.path().join("WowB.exe"), too_deep.join("WowB.exe")).unwrap();
        assert_eq!(scan_installations(container.path()).unwrap().len(), 1);
        assert!(scan_installations(&container.path().join("missing")).is_err());
        let other = container.path().join("OtherClient");
        fs::create_dir_all(&other).unwrap();
        fs::copy(
            fixture.path().join(".flavor.info"),
            other.join(".flavor.info"),
        )
        .unwrap();
        fs::copy(fixture.path().join("WowB.exe"), other.join("WowB.exe")).unwrap();
        assert_eq!(scan_installations(container.path()).unwrap().len(), 2);
    }
    fn installation() -> TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(".flavor.info"),
            "Product Flavor!STRING:0\nwow_classic_beta\n",
        )
        .unwrap();
        fs::write(temp.path().join("WowB.exe"), []).unwrap();
        let addon = temp.path().join("Interface/AddOns/RBGSCompatibility");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("RBGSCompatibility.lua"), "-- synthetic addon").unwrap();
        fs::write(
            addon.join("RBGSCompatibility.toc"),
            "## Interface: 16001\n## SavedVariables: RBGSCompatibilityData\n",
        )
        .unwrap();
        temp
    }
    fn source(root: &Path, account: &str, content: &[u8]) -> PathBuf {
        let path = root
            .join("WTF/Account")
            .join(account)
            .join("SavedVariables")
            .join(DATA_FILE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }
    #[test]
    fn automatic_updates_debounce_replace_delete_recover_and_ignore_other_files() {
        let root = installation();
        let file = source(root.path(), "Fixture", EMPTY);
        let settings = tempfile::tempdir().unwrap();
        let mut manager = SourceManager::default();
        manager
            .choose(root.path(), &settings.path().join("source.json"))
            .unwrap();
        assert!(manager.update().is_none());
        assert_eq!(manager.update().unwrap().data.unwrap().record_count, 0);
        assert!(manager.update().is_none());
        fs::write(file.parent().unwrap().join("OtherAddon.lua"), "anything").unwrap();
        assert!(manager.update().is_none());
        fs::write(&file, "RBGSCompatibilityData={").unwrap();
        assert!(manager.update().is_none());
        let replacement = file.with_extension("replacement");
        fs::write(&replacement, "RBGSCompatibilityData={schema=1,records={{kind=\"event\",payload={name=\"synthetic\"}}}}").unwrap();
        fs::rename(replacement, &file).unwrap();
        assert!(manager.update().is_none());
        assert_eq!(manager.update().unwrap().data.unwrap().record_count, 1);
        assert!(manager.update().is_none());
        fs::write(&file, "RBGSCompatibilityData={").unwrap();
        assert!(manager.update().is_none());
        let invalid = manager.update().unwrap();
        assert_eq!(invalid.status, Status::InvalidData);
        assert!(invalid.data.is_none());
        assert!(manager.update().is_none());
        fs::remove_file(&file).unwrap();
        assert!(manager.update().is_none());
        assert_eq!(manager.update().unwrap().status, Status::NoSavedData);
        fs::write(&file, EMPTY).unwrap();
        assert!(manager.update().is_none());
        assert_eq!(manager.update().unwrap().data.unwrap().record_count, 0);
        let mut restored = SourceManager::default();
        restored.load(&settings.path().join("source.json")).unwrap();
        assert!(restored.update().is_none());
        assert_eq!(restored.update().unwrap().data.unwrap().record_count, 0);
    }
    #[test]
    fn automatic_updates_switch_source_and_do_not_reuse_the_old_accounts_data() {
        let root = installation();
        let first = source(root.path(), "First", EMPTY);
        source(
            root.path(),
            "Second",
            b"RBGSCompatibilityData={schema=2,records={}}",
        );
        let settings = tempfile::tempdir().unwrap();
        let path = settings.path().join("source.json");
        let mut manager = SourceManager::default();
        manager.choose(root.path(), &path).unwrap();
        manager.select("First", &path).unwrap();
        manager.update();
        assert!(manager.update().unwrap().data.is_some());
        manager.select("Second", &path).unwrap();
        assert!(manager.update().is_none());
        let current = manager.update().unwrap();
        assert_eq!(current.selected_source.as_deref(), Some("Second"));
        assert_eq!(current.status, Status::UnsupportedSchema);
        assert!(current.data.is_none());
        fs::write(first, "RBGSCompatibilityData={").unwrap();
        assert!(manager.update().is_none());
    }
    #[test]
    fn persists_the_choice_and_requires_explicit_selection_for_multiple_accounts() {
        let installation = installation();
        source(installation.path(), "FixtureA", EMPTY);
        source(installation.path(), "FixtureB", EMPTY);
        let settings = tempfile::tempdir().unwrap();
        let path = settings.path().join("source.json");
        let mut manager = SourceManager::default();
        assert_eq!(
            manager.choose(installation.path(), &path).unwrap().status,
            Status::SelectSource
        );
        assert!(manager.select("../other", &path).is_err());
        assert_eq!(
            manager.select("FixtureB", &path).unwrap().status,
            Status::UnknownClient
        );
        let mut restored = SourceManager::default();
        restored.load(&path).unwrap();
        assert_eq!(restored.view().selected_source.as_deref(), Some("FixtureB"));
    }
    #[test]
    fn distinguishes_wrong_client_missing_addon_missing_data_and_bad_schema() {
        let root = installation();
        let settings = tempfile::tempdir().unwrap();
        let path = settings.path().join("source.json");
        let mut manager = SourceManager::default();
        assert_eq!(
            manager.choose(root.path(), &path).unwrap().status,
            Status::NoSavedData
        );
        let data = source(
            root.path(),
            "Fixture",
            b"RBGSCompatibilityData={schema=2,records={}}",
        );
        assert_eq!(
            manager.choose(root.path(), &path).unwrap().status,
            Status::UnsupportedSchema
        );
        fs::write(&data, EMPTY).unwrap();
        assert_eq!(manager.view().status, Status::UnknownClient);
        fs::remove_file(
            root.path()
                .join("Interface/AddOns/RBGSCompatibility/RBGSCompatibility.toc"),
        )
        .unwrap();
        assert_eq!(manager.view().status, Status::MissingAddon);
        fs::write(root.path().join(".flavor.info"), "wow_retail").unwrap();
        assert_eq!(manager.view().status, Status::WrongClient);
    }
    #[test]
    fn file_replacement_refreshes_and_invalid_data_never_reuses_a_previous_summary() {
        let root = installation();
        let path = source(root.path(), "Fixture", EMPTY);
        let settings = tempfile::tempdir().unwrap();
        let mut manager = SourceManager::default();
        assert_eq!(
            manager
                .choose(root.path(), &settings.path().join("source.json"))
                .unwrap()
                .data
                .unwrap()
                .record_count,
            0
        );
        let replacement = path.with_extension("replacement");
        fs::write(&replacement, "RBGSCompatibilityData={schema=1,records={{kind=\"event\",at=123,payload={name=\"synthetic\"}}}}").unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_eq!(manager.view().data.unwrap().record_count, 1);
        fs::write(&path, "RBGSCompatibilityData={schema=").unwrap();
        let invalid = manager.view();
        assert_eq!(invalid.status, Status::InvalidData);
        assert!(invalid.data.is_none());
        fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert_eq!(manager.view().status, Status::TooLarge);
    }
    #[test]
    fn contains_reads_and_never_inspects_unrelated_addon_content() {
        let root = installation();
        let path = source(root.path(), "Fixture", EMPTY);
        fs::write(
            path.parent().unwrap().join("OtherAddon.lua"),
            "dofile(\"private\")",
        )
        .unwrap();
        assert!(stable_read(&fs::canonicalize(root.path()).unwrap(), "Fixture").is_ok());
        assert!(
            child(
                &fs::canonicalize(root.path()).unwrap(),
                Path::new("../private"),
                false
            )
            .is_err()
        );
        assert!(child(&fs::canonicalize(root.path()).unwrap(), &path, false).is_err());
    }
    #[test]
    fn rejects_corrupt_settings_and_can_recover_by_choosing_a_folder() {
        let root = installation();
        let settings = tempfile::tempdir().unwrap();
        let path = settings.path().join("source.json");
        fs::write(&path, "{bad").unwrap();
        let mut manager = SourceManager::default();
        assert!(manager.load(&path).is_err());
        assert!(manager.choose(root.path(), &path).is_ok());
        let mut restored = SourceManager::default();
        assert!(restored.load(&path).is_ok());
    }
    #[test]
    fn detects_a_write_or_removal_between_samples() {
        let root = installation();
        let path = source(root.path(), "Fixture", EMPTY);
        let canonical = fs::canonicalize(root.path()).unwrap();
        assert_eq!(
            read_with_pause(&canonical, "Fixture", || fs::write(
                &path,
                "RBGSCompatibilityData={"
            )
            .unwrap())
            .unwrap_err(),
            Status::UnstableFile
        );
        fs::write(&path, EMPTY).unwrap();
        assert_eq!(
            read_with_pause(&canonical, "Fixture", || fs::remove_file(&path).unwrap()).unwrap_err(),
            Status::UnstableFile
        );
    }
    #[cfg(windows)]
    #[test]
    fn rejects_directory_junctions_leading_outside_the_installation() {
        let root = installation();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("WTF/Account")).unwrap();
        let target = root.path().join("WTF/Account/Linked");
        let result = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(target.to_string_lossy().replace('/', "\\"))
            .arg(outside.path().to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let outcome = child(
            &fs::canonicalize(root.path()).unwrap(),
            Path::new("WTF/Account/Linked"),
            true,
        );
        assert!(scan_installations(&target).is_err());
        assert!(
            scan_installations(root.path())
                .unwrap()
                .iter()
                .all(|path| path.starts_with(fs::canonicalize(root.path()).unwrap()))
        );
        // Removing this junction itself does not traverse or remove its target.
        fs::remove_dir(&target).unwrap();
        assert_eq!(outcome.unwrap_err(), Status::SourceUnavailable);
        assert!(outside.path().is_dir());
    }

    #[test]
    #[ignore = "requires an explicitly selected local installation; reads diagnostic data only"]
    fn inspect_local_saved_data_without_identifiers() {
        let root = std::env::var_os("RBGS_TEST_INSTALLATION")
            .expect("Set RBGS_TEST_INSTALLATION to the chosen client folder");
        let root = validate_installation(Path::new(&root)).unwrap();
        check_addon(&root).unwrap();
        let available = sources(&root).unwrap();
        let account = match std::env::var("RBGS_TEST_SOURCE") {
            Ok(account) => {
                assert!(available.iter().any(|source| source.id == account));
                account
            }
            Err(_) if available.len() == 1 => available[0].id.clone(),
            Err(_) => {
                println!(
                    "{} account sources found; choose one explicitly before reading data",
                    available.len()
                );
                return;
            }
        };
        let (summary, _) = stable_read(&root, &account).unwrap();
        println!(
            "Read-only diagnostic check: records={}, build={}, compatibility={:?}, scoreboard_rows={:?}",
            summary.record_count,
            summary.client_build.as_deref().unwrap_or("unknown"),
            summary.compatibility,
            summary.latest_scoreboard_rows
        );
    }
}
