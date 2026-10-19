use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, UserAttentionType};
use tauri_plugin_notification::NotificationExt;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestMatch {
    capture_id: String,
    arm_until: String,
    cancelled: bool,
    character: Character,
}
#[derive(Clone, Deserialize)]
struct Character {
    name: String,
}
impl TestMatch {
    fn available(&self, now: i64) -> bool {
        !self.cancelled
            && chrono::DateTime::parse_from_rfc3339(&self.arm_until)
                .is_ok_and(|t| t.timestamp() > now)
    }
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    account_id: Option<String>,
    version: u64,
    capture_ids: Vec<String>,
    open_capture_id: Option<String>,
    open_sequence: u64,
}
#[derive(Default)]
struct Monitor {
    scope: Option<(String, String)>,
    signature: String,
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    rows: Vec<TestMatch>,
    activity: Activity,
    updated_at: i64,
}
impl Monitor {
    fn update(
        &mut self,
        account: &str,
        fingerprint: &str,
        list: &Value,
        now: i64,
    ) -> Result<Vec<TestMatch>, String> {
        let rows: Vec<TestMatch> =
            serde_json::from_value(list.clone()).map_err(|_| "Cannot read the test match list")?;
        let mut ids = HashSet::new();
        if rows.len() > 50
            || !rows.iter().all(|r| {
                crate::saved_variables::history::uuid(&r.capture_id)
                    && ids.insert(r.capture_id.clone())
                    && !r.character.name.is_empty()
                    && r.character.name.len() <= 512
                    && !r.character.name.chars().any(char::is_control)
                    && chrono::DateTime::parse_from_rfc3339(&r.arm_until).is_ok()
            })
        {
            return Err("Cannot read the test match list".into());
        }
        let scope = (account.to_owned(), fingerprint.to_owned());
        let baseline = self.scope.as_ref() != Some(&scope);
        if baseline {
            self.scope = Some(scope);
            self.seen.clear();
            self.seen_order.clear();
            self.activity.open_capture_id = None;
        }
        let signature = list.to_string();
        if baseline || signature != self.signature {
            self.activity.version += 1;
            self.signature = signature;
        }
        let mut fresh = Vec::new();
        for row in &rows {
            if self.seen.insert(row.capture_id.clone()) {
                self.seen_order.push_back(row.capture_id.clone());
                if !baseline && row.available(now) {
                    fresh.push(row.clone());
                }
            }
        }
        while self.seen_order.len() > 1024 {
            if let Some(id) = self.seen_order.pop_front() {
                self.seen.remove(&id);
            }
        }
        self.rows = rows;
        self.updated_at = now;
        self.activity.account_id = Some(account.to_owned());
        self.activity.capture_ids = self
            .rows
            .iter()
            .filter(|r| r.available(now))
            .map(|r| r.capture_id.clone())
            .collect();
        if self
            .activity
            .open_capture_id
            .as_ref()
            .is_some_and(|id| !self.activity.capture_ids.contains(id))
        {
            self.activity.open_capture_id = None;
        }
        Ok(fresh)
    }
    fn snapshot(&self, fingerprint: Option<&str>, now: i64) -> Activity {
        if self
            .scope
            .as_ref()
            .is_none_or(|s| Some(s.1.as_str()) != fingerprint)
            || now - self.updated_at > 60
        {
            return Activity::default();
        }
        let mut result = self.activity.clone();
        result.capture_ids = self
            .rows
            .iter()
            .filter(|r| r.available(now))
            .map(|r| r.capture_id.clone())
            .collect();
        if result
            .open_capture_id
            .as_ref()
            .is_some_and(|id| !result.capture_ids.contains(id))
        {
            result.open_capture_id = None;
        }
        result
    }
}
#[derive(Default)]
pub struct AssignmentState {
    monitor: Mutex<Monitor>,
    polling: tokio::sync::Mutex<()>,
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(crate) fn observe(
    app: &tauri::AppHandle,
    account: &str,
    fingerprint: &str,
    list: &Value,
) -> Result<(), String> {
    if crate::pairing::credential_fingerprint().as_deref() != Some(fingerprint) {
        return Err("The connected account changed. Try again.".into());
    }
    let fresh = app
        .state::<AssignmentState>()
        .monitor
        .lock()
        .map_err(|_| "Test updates unavailable")?
        .update(account, fingerprint, list, now())?;
    if !fresh.is_empty()
        && app
            .get_webview_window("main")
            .is_some_and(|w| !w.is_focused().unwrap_or(false))
    {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.request_user_attention(Some(UserAttentionType::Informational));
        }
        let row = &fresh[0];
        let _ = app.notification().builder().title("Test match ready")
            .body("Open rbgs.io companion to get your match command before entering the battleground.")
            .extra("captureId", &row.capture_id).extra("accountId", account).show();
    }
    Ok(())
}
async fn refresh(app: &tauri::AppHandle) -> Result<(), String> {
    refresh_list(app).await.map(|_| ())
}
pub(crate) async fn refresh_list(app: &tauri::AppHandle) -> Result<Value, String> {
    let state = app.state::<AssignmentState>();
    let _guard = state.polling.lock().await;
    let (account, fingerprint, list) =
        crate::saved_variables::transport::fetch_assignments(app).await?;
    observe(app, &account, &fingerprint, &list)?;
    Ok(list)
}
#[tauri::command]
pub fn assignment_activity(app: tauri::AppHandle) -> Activity {
    let fingerprint = crate::pairing::credential_fingerprint();
    app.state::<AssignmentState>()
        .monitor
        .lock()
        .map(|m| m.snapshot(fingerprint.as_deref(), now()))
        .unwrap_or_default()
}
async fn open(
    app: &tauri::AppHandle,
    id: Option<String>,
    expected_account: Option<String>,
) -> Result<(), String> {
    refresh(app).await?;
    let fingerprint = crate::pairing::credential_fingerprint();
    {
        let state = app.state::<AssignmentState>();
        let mut monitor = state
            .monitor
            .lock()
            .map_err(|_| "Test updates unavailable")?;
        if expected_account.as_ref().is_some_and(|account| {
            monitor
                .scope
                .as_ref()
                .is_none_or(|scope| &scope.0 != account)
        }) {
            return Err("The connected account changed.".into());
        }
        let activity = monitor.snapshot(fingerprint.as_deref(), now());
        let selected = id
            .or_else(|| activity.capture_ids.first().cloned())
            .filter(|id| activity.capture_ids.contains(id))
            .ok_or("This test match is no longer available.")?;
        monitor.activity.open_capture_id = Some(selected);
        monitor.activity.open_sequence += 1;
    }
    crate::presence::show(app);
    Ok(())
}
#[tauri::command]
pub async fn open_test_match(app: tauri::AppHandle, capture_id: String) -> Result<(), String> {
    open(&app, Some(capture_id), None).await
}
pub(crate) fn open_latest(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = open(&app, None, None).await;
    });
}
pub(crate) fn setup(app: &tauri::AppHandle) {
    let handle = app.clone();
    let listener = app.notification().on_action(move |event| {
        if event.action_id() != "tap" {
            return;
        }
        let Some(extra) = event.notification().map(|n| n.extra()) else {
            return;
        };
        let Some(id) = extra
            .get("captureId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };
        let Some(account) = extra
            .get("accountId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let _ = open(&handle, Some(id), Some(account)).await;
        });
    });
    if listener.is_err() {
        eprintln!("Test notification clicks are unavailable. Open the companion to view tests.");
    }
    if let Some(window) = app.get_webview_window("main") {
        let window_handle = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Focused(true)) {
                let _ = window_handle.request_user_attention(None);
            }
        });
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let delay = if refresh(&handle).await.is_ok() {
                5
            } else {
                15
            };
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn rows(id: u8, cancelled: bool, deadline: &str) -> Value {
        json!([{"captureId":format!("00000000-0000-4000-8000-{id:012}"),"armUntil":deadline,"cancelled":cancelled,"character":{"name":"Synthetic Player"}}])
    }
    #[test]
    fn baseline_and_account_switch_are_silent_new_assignments_notify_once() {
        let mut m = Monitor::default();
        let future = "2030-01-01T00:00:00Z";
        assert!(
            m.update("a", "f1", &rows(1, false, future), 100)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            m.update("a", "f1", &rows(2, false, future), 101)
                .unwrap()
                .len(),
            1
        );
        assert!(
            m.update("a", "f1", &rows(2, false, future), 102)
                .unwrap()
                .is_empty()
        );
        assert!(
            m.update("b", "f2", &rows(3, false, future), 103)
                .unwrap()
                .is_empty()
        );
        assert!(m.snapshot(Some("f1"), 104).account_id.is_none());
        assert_eq!(m.snapshot(Some("f2"), 104).account_id.as_deref(), Some("b"));
        assert!(m.snapshot(Some("f2"), 200).capture_ids.is_empty());
    }
    #[test]
    fn expired_cancelled_and_malformed_rows_do_not_alert_or_replace_a_valid_snapshot() {
        let mut m = Monitor::default();
        m.update("a", "f", &json!([]), 100).unwrap();
        assert!(
            m.update("a", "f", &rows(1, true, "2030-01-01T00:00:00Z"), 101)
                .unwrap()
                .is_empty()
        );
        assert!(
            m.update("a", "f", &rows(2, false, "1970-01-01T00:00:01Z"), 102)
                .unwrap()
                .is_empty()
        );
        let version = m.activity.version;
        assert!(m.update("a", "f", &rows(3, false, "bad"), 103).is_err());
        assert_eq!(m.activity.version, version);
        assert!(m.snapshot(Some("f"), 104).capture_ids.is_empty());
    }
    #[test]
    fn activity_contains_no_command_codes_or_credential_fingerprints() {
        let mut m = Monitor::default();
        let mut list = rows(1, false, "2030-01-01T00:00:00Z");
        list[0]["command"] = json!("private-command");
        m.update("a", "private-fingerprint", &list, 100).unwrap();
        let wire = serde_json::to_string(&m.snapshot(Some("private-fingerprint"), 101)).unwrap();
        assert!(!wire.contains("private-command"));
        assert!(!wire.contains("private-fingerprint"));
    }
}
