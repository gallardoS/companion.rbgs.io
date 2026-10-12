//! Bounded product reader and history projection, integrated with the local inbox.
use super::*;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    schema: u8,
    captures: Vec<Record>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    capture_id: String,
    capture_revision: u32,
    assignment_code: String,
    client: Client,
    observed_character: Identity,
    capture: Capture,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Client {
    addon_version: String,
    adapter_version: String,
    product: String,
    game_version: String,
    game_build: String,
    interface: u64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    name: String,
    realm: Option<String>,
    guid: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Map {
    profile: String,
    observed_name: String,
    game_map_id: Option<u64>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Row {
    character: Identity,
    observed_team_key: Option<String>,
    killing_blows: Option<u64>,
    honorable_kills: Option<u64>,
    deaths: Option<u64>,
    damage: Option<u64>,
    healing: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Capture {
    state: String,
    stage: String,
    continuity: String,
    incomplete_reason: Option<String>,
    map: Option<Map>,
    started_at: Option<String>,
    ended_at: Option<String>,
    roster: Vec<Row>,
    outcome: Outcome,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Outcome {
    kind: String,
    winning_observed_team_key: Option<String>,
    raw_winner: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub(super) capture_id: String,
    revision: u32,
    character: Identity,
    map: Map,
    started_at: String,
    pub(super) ended_at: String,
    winner: String,
    roster: Vec<Row>,
    build: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    pub(super) status: &'static str,
    pub(super) message: &'static str,
    pub(super) matches: Vec<Match>,
    pub(super) retained_attempts: usize,
    pub(super) queue: Option<super::inbox::QueueView>,
}
impl HistoryView {
    fn error(status: &'static str, message: &'static str) -> Self {
        Self {
            status,
            message,
            matches: vec![],
            retained_attempts: 0,
            queue: None,
        }
    }
}
#[derive(Default)]
pub(super) struct HistoryCache {
    key: Option<(PathBuf, String)>,
    bytes: Vec<u8>,
    view: Option<HistoryView>,
    pub(super) records: Vec<serde_json::Value>,
}
fn text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.chars().count() <= 128
        && !value.chars().any(char::is_control)
}
fn optional_text(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|value| text(value))
}
fn identity(value: &Identity) -> bool {
    text(&value.name) && optional_text(&value.realm) && optional_text(&value.guid)
}
pub(super) fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, byte)| {
            if [8, 13, 18, 23].contains(&i) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
pub(super) fn timestamp(value: &str) -> bool {
    if value.len() != 20 || !value.is_ascii() {
        return false;
    }
    let bytes = value.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        let special = match i {
            4 | 7 => Some(b'-'),
            10 => Some(b'T'),
            13 | 16 => Some(b':'),
            19 => Some(b'Z'),
            _ => None,
        };
        if special.map_or(!byte.is_ascii_digit(), |expected| *byte != expected) {
            return false;
        }
    }
    let year = value[0..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..10].parse::<u32>().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    year > 0
        && day > 0
        && day <= days
        && value[11..13].parse::<u32>().unwrap_or(99) <= 23
        && value[14..16].parse::<u32>().unwrap_or(99) <= 59
        && value[17..19].parse::<u32>().unwrap_or(99) <= 59
}
fn valid(record: &Record) -> bool {
    let c = &record.capture;
    let client = &record.client;
    let supported = client.product == "wow_classic_beta"
        && client.game_version == "1.60.1"
        && client.game_build == "70338"
        && client.interface == 16001
        && text(&client.addon_version)
        && client.adapter_version == "darkspear-70338-test-1";
    if !supported
        || !uuid(&record.capture_id)
        || !(1..=2147483647).contains(&record.capture_revision)
        || record.assignment_code.len() != 16
        || !record
            .assignment_code
            .bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
        || !identity(&record.observed_character)
        || c.roster.len() > 40
        || !["CONTINUOUS", "UNKNOWN"].contains(&c.continuity.as_str())
    {
        return false;
    }
    if c.started_at.as_ref().is_some_and(|v| !timestamp(v))
        || c.ended_at.as_ref().is_some_and(|v| !timestamp(v))
    {
        return false;
    }
    if c.map.as_ref().is_some_and(|m| {
        m.profile != "DARKSPEAR_DIAGNOSTIC"
            || !text(&m.observed_name)
            || m.game_map_id.is_some_and(|id| id > 9007199254740991)
    }) {
        return false;
    }
    if c.roster.iter().any(|row| {
        !identity(&row.character)
            || row
                .observed_team_key
                .as_ref()
                .is_some_and(|v| v != "0" && v != "1")
            || [
                row.killing_blows,
                row.honorable_kills,
                row.deaths,
                row.damage,
                row.healing,
            ]
            .iter()
            .flatten()
            .any(|v| *v > 9007199254740991)
    }) {
        return false;
    }
    if !optional_text(&c.outcome.raw_winner) {
        return false;
    }
    let win = c.outcome.kind == "WIN"
        && c.outcome
            .winning_observed_team_key
            .as_deref()
            .is_some_and(|v| v == "0" || v == "1");
    if !win && !(c.outcome.kind == "UNKNOWN" && c.outcome.winning_observed_team_key.is_none()) {
        return false;
    }
    if c.state == "COMPLETE" {
        c.stage == "COMPLETE"
            && c.continuity == "CONTINUOUS"
            && c.incomplete_reason.is_none()
            && c.map.as_ref().is_some_and(|m| m.game_map_id == Some(2997))
            && c.started_at.is_some()
            && c.ended_at.is_some()
            && !c.roster.is_empty()
            && win
    } else {
        c.state == "INCOMPLETE"
            && if ["ARMED", "RECORDING", "FINAL_PENDING"].contains(&c.stage.as_str()) {
                c.incomplete_reason.as_deref() == Some("CAPTURE_PENDING")
            } else {
                c.stage == "INCOMPLETE"
                    && c.incomplete_reason.as_deref().is_some_and(|v| {
                        [
                            "MAP_MISMATCH",
                            "CHARACTER_CHANGED",
                            "EARLY_EXIT",
                            "SESSION_BOUNDARY_UNCERTAIN",
                            "MISSING_FINAL_OUTCOME",
                            "MISSING_REQUIRED_DATA",
                            "UNSUPPORTED_ADAPTER",
                            "STORAGE_FULL",
                            "CANCELLED",
                        ]
                        .contains(&v)
                    })
            }
    }
}
pub(super) fn snapshots(bytes: &[u8]) -> Result<Vec<serde_json::Value>, Status> {
    let json = parser::product_json(bytes).map_err(|e| {
        if e == ParseError::TooLarge {
            Status::TooLarge
        } else {
            Status::InvalidData
        }
    })?;
    if json.get("schema").and_then(serde_json::Value::as_u64) != Some(2) {
        return Err(Status::UnsupportedSchema);
    }
    let export: Export = serde_json::from_value(json.clone()).map_err(|_| Status::InvalidData)?;
    if export.schema != 2 || export.captures.len() > 8 {
        return Err(Status::InvalidData);
    }
    let mut ids = HashSet::new();
    let mut active = 0;
    for record in export.captures {
        if !valid(&record) || !ids.insert(record.capture_id.to_ascii_lowercase()) {
            return Err(Status::InvalidData);
        }
        let c = record.capture;
        if ["ARMED", "RECORDING", "FINAL_PENDING"].contains(&c.stage.as_str()) {
            active += 1;
        }
    }
    if active > 1 {
        return Err(Status::InvalidData);
    }
    let mut records = json["captures"]
        .as_array()
        .ok_or(Status::InvalidData)?
        .clone();
    // Required nullable fields have an explicit null at this boundary. Optional
    // stats/GUID/rawWinner retain their original null-versus-absence distinction.
    for item in &mut records {
        item["observedCharacter"]
            .as_object_mut()
            .unwrap()
            .entry("realm")
            .or_insert(serde_json::Value::Null);
        let c = item["capture"].as_object_mut().unwrap();
        for name in ["map", "startedAt", "endedAt", "incompleteReason"] {
            c.entry(name).or_insert(serde_json::Value::Null);
        }
        if let Some(map) = c.get_mut("map").and_then(serde_json::Value::as_object_mut) {
            map.entry("gameMapId").or_insert(serde_json::Value::Null);
        }
        c.get_mut("outcome")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .entry("winningObservedTeamKey")
            .or_insert(serde_json::Value::Null);
        for row in c.get_mut("roster").unwrap().as_array_mut().unwrap() {
            row.as_object_mut()
                .unwrap()
                .entry("observedTeamKey")
                .or_insert(serde_json::Value::Null);
            row["character"]
                .as_object_mut()
                .unwrap()
                .entry("realm")
                .or_insert(serde_json::Value::Null);
        }
    }
    Ok(records)
}
pub(super) fn project_snapshot(value: serde_json::Value) -> Result<Option<Match>, Status> {
    let record: Record = serde_json::from_value(value).map_err(|_| Status::InvalidData)?;
    if !valid(&record) {
        return Err(Status::InvalidData);
    }
    let c = record.capture;
    if c.state != "COMPLETE" {
        return Ok(None);
    }
    // Projection deliberately excludes assignment codes and GUIDs from the UI.
    let mut character = record.observed_character;
    character.guid = None;
    let mut roster = c.roster;
    for row in &mut roster {
        row.character.guid = None;
    }
    Ok(Some(Match {
        capture_id: record.capture_id,
        revision: record.capture_revision,
        character,
        map: c.map.unwrap(),
        started_at: c.started_at.unwrap(),
        ended_at: c.ended_at.unwrap(),
        winner: c.outcome.winning_observed_team_key.unwrap(),
        roster,
        build: record.client.game_build,
    }))
}
fn parse_history(bytes: &[u8]) -> Result<HistoryView, Status> {
    let records = snapshots(bytes)?;
    let count = records.len();
    let mut matches = Vec::new();
    for record in records {
        if let Some(item) = project_snapshot(record)? {
            matches.push(item);
        }
    }
    matches.sort_by(|a, b| {
        b.ended_at
            .cmp(&a.ended_at)
            .then(a.capture_id.cmp(&b.capture_id))
    });
    Ok(HistoryView {
        status: "READY",
        message: "Local completed captures. Not verified or uploaded.",
        matches,
        retained_attempts: count,
        queue: None,
    })
}
impl HistoryCache {
    pub(super) fn read(&mut self, config: Option<&Config>) -> HistoryView {
        let result = (|| {
            let config = config.ok_or(Status::NotConfigured)?;
            let account = config.account.as_ref().ok_or(Status::SelectSource)?;
            let root = validate_installation(&config.installation)?;
            if !sources(&root)?.iter().any(|s| &s.id == account) {
                return Err(Status::SourceUnavailable);
            }
            let toc = child(&root, Path::new("Interface/AddOns/RBGS/RBGS.toc"), false)
                .map_err(|_| Status::MissingAddon)?;
            let (metadata, _) = read_bounded(&toc, MAX_TOC_BYTES)?;
            let toc = std::str::from_utf8(&metadata).map_err(|_| Status::UnsupportedAddon)?;
            if !toc.lines().any(|l| l.trim() == "## Interface: 16001")
                || !toc
                    .lines()
                    .any(|l| l.trim() == "## SavedVariables: RBGSMatchData")
            {
                return Err(Status::UnsupportedAddon);
            }
            let relative = Path::new("WTF/Account")
                .join(account)
                .join("SavedVariables/RBGS.lua");
            if !root.join(&relative).exists() {
                return Err(Status::NoSavedData);
            }
            let path = child(&root, &relative, false)?;
            let (bytes, before) = read_bounded(&path, MAX_BYTES)?;
            let key = (root.clone(), account.clone());
            if self.key.as_ref() == Some(&key) && self.bytes == bytes {
                if let Some(view) = &self.view {
                    return Ok(view.clone());
                }
            }
            std::thread::sleep(Duration::from_millis(250));
            let after_path = child(&root, &relative, false).map_err(|_| Status::UnstableFile)?;
            let (after, stat) = read_bounded(&after_path, MAX_BYTES)?;
            if path != after_path
                || bytes != after
                || before.modified().ok() != stat.modified().ok()
            {
                return Err(Status::UnstableFile);
            }
            let view = match parse_history(&bytes) {
                Ok(v) => v,
                Err(e) => history_error(e),
            };
            self.records = if view.status == "READY" {
                snapshots(&bytes)?
            } else {
                vec![]
            };
            self.key = Some(key);
            self.bytes = bytes;
            self.view = Some(view.clone());
            Ok(view)
        })();
        match result {
            Ok(view) => view,
            Err(status) => {
                *self = Self::default();
                history_error(status)
            }
        }
    }
}
fn history_error(status: Status) -> HistoryView {
    let (status, message) = match status {
        Status::NotConfigured | Status::SelectSource => (
            "SELECT_SOURCE",
            "Select your Forever installation and SavedVariables account above.",
        ),
        Status::MissingAddon | Status::UnsupportedAddon => (
            "MISSING_ADDON",
            "Install and enable rbgs.io (RBGS), then reload or log out.",
        ),
        Status::NoSavedData => (
            "NO_SAVED_DATA",
            "No product captures saved yet. Enable rbgs.io and use /reload or log out.",
        ),
        Status::UnstableFile => (
            "UNSTABLE_FILE",
            "WoW is saving the file. Waiting for a stable copy…",
        ),
        Status::TooLarge => (
            "INVALID_DATA",
            "The product file exceeds the 2 MB limit. Keep a private backup.",
        ),
        Status::InvalidData | Status::UnsupportedSchema => (
            "INVALID_DATA",
            "Product data is invalid or unsupported. Check addon/client versions; data was not changed.",
        ),
        _ => (
            "SOURCE_UNAVAILABLE",
            "The selected product source is unavailable. Check the folder/account above.",
        ),
    };
    HistoryView::error(status, message)
}
#[tauri::command]
pub async fn match_history(
    app: tauri::AppHandle,
    state: tauri::State<'_, SavedVariablesState>,
) -> Result<HistoryView, String> {
    let path = settings_path(&app)?;
    let inbox_path = super::inbox::path(&app)?;
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut manager = state
            .lock()
            .map_err(|_| "Local source state is unavailable")?;
        manager.load(&path)?;
        let config = manager.config.clone();
        let view = manager.history.read(config.as_ref());
        let records = manager.history.records.clone();
        Ok(super::inbox::refresh(
            &inbox_path,
            config.as_ref(),
            records,
            view,
        ))
    })
    .await
    .map_err(|_| "Local history reader could not finish")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};
    fn fixture() -> Json {
        serde_json::from_str(include_str!(
            "../../../tools/fixtures/product-70338-final.sanitized.json"
        ))
        .unwrap()
    }
    fn lua(value: &Json) -> String {
        match value {
            Json::Null => "nil".into(),
            Json::Number(n) => n.to_string(),
            Json::String(s) => serde_json::to_string(s).unwrap(),
            Json::Array(rows) => {
                format!("{{{}}}", rows.iter().map(lua).collect::<Vec<_>>().join(","))
            }
            Json::Object(object) => format!(
                "{{{}}}",
                object
                    .iter()
                    .map(|(k, v)| format!("[{}]={}", lua(&Json::String(k.clone())), lua(v)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            _ => panic!("not a product fixture value"),
        }
    }
    fn bytes(value: &Json) -> Vec<u8> {
        format!("RBGSMatchData={}", lua(value)).into_bytes()
    }
    #[test]
    fn reads_real_sanitized_final_and_never_exports_codes_or_guids() {
        let history = parse_history(&bytes(&fixture())).unwrap();
        assert_eq!(history.matches.len(), 1);
        assert_eq!(history.matches[0].revision, 158);
        assert_eq!(history.matches[0].roster.len(), 30);
        assert_eq!(history.matches[0].winner, "1");
        let ui = serde_json::to_string(&history).unwrap();
        assert!(!ui.contains("0123456789ABCDEF"));
        assert!(!ui.contains("fixture-player-guid"));
        assert!(!ui.contains("assignmentCode"));
    }
    #[test]
    fn filters_pending_and_incomplete_orders_completed_and_deduplicates_ids() {
        let mut value = fixture();
        let mut newer = value["captures"][0].clone();
        newer["captureId"] = json!("00000000-0000-4000-8000-000000000004");
        newer["capture"]["endedAt"] = json!("2026-10-11T17:00:00Z");
        let mut pending = newer.clone();
        pending["captureId"] = json!("00000000-0000-4000-8000-000000000005");
        pending["capture"]["state"] = json!("INCOMPLETE");
        pending["capture"]["stage"] = json!("ARMED");
        pending["capture"]["incompleteReason"] = json!("CAPTURE_PENDING");
        value["captures"]
            .as_array_mut()
            .unwrap()
            .extend([newer.clone(), pending]);
        let history = parse_history(&bytes(&value)).unwrap();
        assert_eq!(history.retained_attempts, 3);
        assert_eq!(history.matches.len(), 2);
        assert_eq!(
            history.matches[0].capture_id,
            newer["captureId"].as_str().unwrap()
        );
        value["captures"].as_array_mut().unwrap().push(newer);
        assert!(parse_history(&bytes(&value)).is_err());
        assert!(
            parse_history(b"RBGSMatchData={schema=2,captures={}}")
                .unwrap()
                .matches
                .is_empty()
        );
    }
    #[test]
    fn rejects_wrong_roots_schemas_execution_duplicate_keys_and_invalid_fields() {
        for source in [
            "RBGSCompatibilityData={schema=2,captures={}}",
            "RBGSMatchData={schema=1,captures={}}",
            "RBGSMatchData={schema=2,captures={}}; print('bad')",
            "RBGSMatchData={schema=2,schema=2,captures={}}",
            "RBGSMatchData={schema=2,captures={[2]={}}}",
        ] {
            assert!(parse_history(source.as_bytes()).is_err());
        }
        for (pointer, bad) in [
            ("/captures/0/capture/continuity", json!("UNKNOWN")),
            ("/captures/0/capture/endedAt", json!("2026-02-30T17:00:00Z")),
            (
                "/captures/0/capture/outcome/winningObservedTeamKey",
                json!("2"),
            ),
            ("/captures/0/client/gameBuild", json!("99999")),
            (
                "/captures/0/capture/roster/0/damage",
                json!(9007199254740992u64),
            ),
        ] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(parse_history(&bytes(&value)).is_err(), "{pointer}");
        }
        let mut value = fixture();
        value["captures"][0]["capture"]["extra"] = json!("unsupported");
        assert!(parse_history(&bytes(&value)).is_err());
    }
    #[test]
    fn enforces_retention_rows_text_limits_and_preserves_unknown_stats() {
        let mut value = fixture();
        value["captures"][0]["capture"]["roster"][0]
            .as_object_mut()
            .unwrap()
            .remove("damage");
        let history = parse_history(&bytes(&value)).unwrap();
        assert!(history.matches[0].roster[0].damage.is_none());
        let row = value["captures"][0]["capture"]["roster"][0].clone();
        value["captures"][0]["capture"]["roster"] = json!(vec![row; 41]);
        assert!(parse_history(&bytes(&value)).is_err());
        let mut value = fixture();
        value["captures"][0]["observedCharacter"]["name"] = json!("x".repeat(129));
        assert!(parse_history(&bytes(&value)).is_err());
        let record = fixture()["captures"][0].clone();
        assert!(parse_history(&bytes(&json!({"schema":2,"captures":vec![record;9]}))).is_err());
    }
    fn installation() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".flavor.info"), "wow_classic_beta").unwrap();
        fs::write(dir.path().join("WowB.exe"), b"fixture").unwrap();
        fs::create_dir_all(dir.path().join("Interface/AddOns/RBGS")).unwrap();
        fs::write(
            dir.path().join("Interface/AddOns/RBGS/RBGS.toc"),
            "## Interface: 16001\n## SavedVariables: RBGSMatchData",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("WTF/Account/one/SavedVariables")).unwrap();
        fs::create_dir_all(dir.path().join("WTF/Account/two/SavedVariables")).unwrap();
        dir
    }
    #[test]
    fn isolates_accounts_clears_missing_invalid_data_and_reads_after_restart() {
        let dir = installation();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("WTF/Account/one/SavedVariables/RBGS.lua");
        fs::write(&path, bytes(&fixture())).unwrap();
        let mut cache = HistoryCache::default();
        let mut config = Config {
            version: 1,
            installation: root,
            account: Some("one".into()),
        };
        assert_eq!(cache.read(Some(&config)).matches.len(), 1);
        assert_eq!(cache.read(Some(&config)).matches.len(), 1);
        config.account = Some("two".into());
        assert!(cache.read(Some(&config)).matches.is_empty());
        config.account = Some("one".into());
        assert_eq!(HistoryCache::default().read(Some(&config)).matches.len(), 1);
        fs::write(&path, "RBGSMatchData={schema=999}").unwrap();
        assert_eq!(cache.read(Some(&config)).status, "INVALID_DATA");
        fs::remove_file(&path).unwrap();
        assert_eq!(cache.read(Some(&config)).status, "NO_SAVED_DATA");
        fs::write(&path, bytes(&fixture())).unwrap();
        assert_eq!(cache.read(Some(&config)).matches.len(), 1);
    }
    #[test]
    #[ignore = "Optional read-only inspection of the selected real game source"]
    fn inspect_selected_product_source() {
        let path = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap())
            .join("io.rbgs.companion/saved-variables-source.json");
        let mut manager = SourceManager::default();
        manager.load(&path).unwrap();
        let config = manager.config.clone();
        let view = manager.history.read(config.as_ref());
        assert_eq!(view.status, "READY");
        assert!(!view.matches.is_empty());
        println!(
            "Product source: {} completed captures, {} roster rows in latest",
            view.matches.len(),
            view.matches[0].roster.len()
        );
    }
}
