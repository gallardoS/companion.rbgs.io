use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const SERVICE: &str = "io.rbgs.companion.device";
const SCOPE: &str = "companion:status";
pub(crate) const CAPTURE_SCOPE: &str = "companion:status companion:assignments evidence:write";
pub(crate) static CONNECTION_LOCK: Mutex<()> = Mutex::const_new(());

pub struct PairingState(Mutex<Option<Pending>>);
impl Default for PairingState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}
struct Pending {
    secret: String,
    deadline: Instant,
    next_poll: Instant,
    issued_token: Option<String>,
    requested_scope: String,
    approval_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    code: String,
    expires_at: String,
    approval_url: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairingResponse {
    code: String,
    polling_secret: String,
    expires_at: String,
    interval_seconds: u64,
    approval_secret: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenResponse {
    status: String,
    token: Option<String>,
    scope: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    device_id: String,
    pub(crate) account_id: String,
    #[serde(default)]
    pub(crate) account_name: Option<String>,
    scope: String,
    local_storage_warning: Option<String>,
    #[serde(default)]
    pub capture_permissions: bool,
    #[serde(skip)]
    pub(crate) credential_fingerprint: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectionStatusError {
    kind: ConnectionErrorKind,
    message: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ConnectionErrorKind {
    NotPaired,
    AuthRequired,
    Unavailable,
    StorageUnavailable,
}

impl From<String> for ConnectionStatusError {
    fn from(message: String) -> Self {
        let kind = match message.as_str() {
            "No paired device. Generate a pairing code." => ConnectionErrorKind::NotPaired,
            "Device access expired, was revoked, or the account is unavailable. Pair again." => {
                ConnectionErrorKind::AuthRequired
            }
            "Windows credential storage is unavailable"
            | "Windows credential storage could not be read" => {
                ConnectionErrorKind::StorageUnavailable
            }
            _ => ConnectionErrorKind::Unavailable,
        };
        Self { kind, message }
    }
}

fn configured_origin(debug: bool, value: Option<&str>) -> String {
    if debug {
        value.unwrap_or("https://api.rbgs.io").into()
    } else {
        "https://api.rbgs.io".into()
    }
}
fn allowed_origin(value: &str, debug: bool) -> bool {
    value == "https://api.rbgs.io" || (debug && value == "http://localhost:8080")
}
pub(crate) fn origin() -> &'static str {
    // Native launch configuration is frozen for this process; web content cannot change it.
    static ORIGIN: OnceLock<String> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        let value = if cfg!(debug_assertions) {
            match std::env::var("RBGS_COMPANION_API_ORIGIN") {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(std::env::VarError::NotUnicode(_)) => Some(String::new()),
            }
        } else {
            None
        };
        configured_origin(cfg!(debug_assertions), value.as_deref())
    })
}
pub(crate) fn client() -> Result<reqwest::Client, String> {
    if !allowed_origin(origin(), cfg!(debug_assertions)) {
        return Err("Unsupported API origin".into());
    }
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not initialize the connection".into())
}
pub(crate) fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, origin())
        .map_err(|_| "Windows credential storage is unavailable".into())
}
pub(crate) fn stored_token() -> Result<String, String> {
    entry()?.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => "No paired device. Generate a pairing code.".into(),
        _ => "Windows credential storage could not be read".into(),
    })
}
pub(crate) fn credential_fingerprint() -> Option<String> {
    use sha2::{Digest, Sha256};
    stored_token()
        .ok()
        .map(|token| format!("{:x}", Sha256::digest(token.as_bytes())))
}
pub(crate) fn valid_secret(secret: &str) -> bool {
    secret.len() == 43
        && secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn api_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 => {
            "Device access expired, was revoked, or the account is unavailable. Pair again.".into()
        }
        410 => "Pairing expired or was already used. Generate a new code.".into(),
        429 => "Too many attempts. Wait one minute before retrying.".into(),
        409 => "Device limit reached. Revoke an existing device in web Settings.".into(),
        _ => "The server could not confirm this operation. Try again later.".into(),
    }
}

#[tauri::command]
pub async fn start_pairing(
    state: tauri::State<'_, PairingState>,
    web: tauri::State<'_, crate::web_connect::WebConnectState>,
    capture_permissions: Option<bool>,
) -> Result<PairingView, String> {
    let _connection = CONNECTION_LOCK.lock().await;
    if web.is_pending().await {
        return Err("Finish the connection opened from the website first".into());
    }
    let mut pending = state.0.lock().await;
    if pending.as_ref().is_some_and(|p| p.issued_token.is_some()) {
        return Err("Finish saving the issued credential before generating another code".into());
    }
    let requested_scope = if capture_permissions.unwrap_or(true) {
        CAPTURE_SCOPE
    } else {
        SCOPE
    };
    let response = client()?
        .post(format!("{}/api/v1/device-pairings", origin()))
        .json(&serde_json::json!({ "scope": requested_scope }))
        .send()
        .await
        .map_err(|_| "Connection unavailable")?;
    if !response.status().is_success() {
        return Err(api_error(response.status()));
    }
    let result: PairingResponse = response
        .json()
        .await
        .map_err(|_| "Invalid pairing response")?;
    if !valid_secret(&result.polling_secret)
        || !valid_secret(&result.approval_secret)
        || result.code.len() != 10
        || result.interval_seconds != 5
    {
        return Err("Invalid pairing response".into());
    }
    let approval_url = format!(
        "{}/settings#companion-link={}",
        web_origin(),
        result.approval_secret
    );
    *pending = Some(Pending {
        secret: result.polling_secret,
        deadline: Instant::now() + Duration::from_secs(300),
        next_poll: Instant::now(),
        issued_token: None,
        requested_scope: requested_scope.into(),
        approval_url: approval_url.clone(),
    });
    Ok(PairingView {
        code: result.code,
        expires_at: result.expires_at,
        approval_url,
    })
}

#[tauri::command]
pub async fn poll_pairing(state: tauri::State<'_, PairingState>) -> Result<String, String> {
    advance_pairing(&state).await
}
pub(crate) async fn advance_pairing(state: &PairingState) -> Result<String, String> {
    let _connection = CONNECTION_LOCK.lock().await;
    let mut slot = state.0.lock().await;
    let Some(pending) = slot.as_mut() else {
        return Ok(if stored_token().is_ok() {
            "PAIRED"
        } else {
            "IDLE"
        }
        .into());
    };
    if pending.issued_token.is_none() {
        if Instant::now() >= pending.deadline {
            return Err("Pairing expired. Generate a new code.".into());
        }
        if Instant::now() < pending.next_poll {
            return Ok("PENDING".into());
        }
        pending.next_poll = Instant::now() + Duration::from_secs(5);
        let response = client()?.post(format!("{}/api/v1/device-pairings/token", origin()))
            .json(&serde_json::json!({ "pollingSecret": pending.secret })).send().await
            .map_err(|_| "Connection interrupted. If approval was already consumed, generate a new code.")?;
        if !response.status().is_success() {
            return Err(api_error(response.status()));
        }
        let result: TokenResponse = response
            .json()
            .await
            .map_err(|_| "Invalid token response")?;
        if result.status == "PENDING" {
            return Ok("PENDING".into());
        }
        let token = result
            .token
            .filter(|t| valid_secret(t))
            .ok_or("Invalid token response")?;
        if result.status != "PAIRED"
            || result.scope.as_deref() != Some(pending.requested_scope.as_str())
        {
            return Err("Invalid token scope".into());
        }
        pending.issued_token = Some(token);
    }
    // On storage failure retain the token only in Rust memory so the user can retry without a second exchange.
    entry()?
        .set_password(pending.issued_token.as_ref().unwrap())
        .map_err(|_| "Could not save the credential in Windows. Retry finishing the connection.")?;
    *slot = None;
    Ok("PAIRED".into())
}

pub(crate) async fn cancel_unissued(state: &PairingState) -> Result<(), String> {
    let mut pending = state.0.lock().await;
    if pending.as_ref().is_some_and(|p| p.issued_token.is_some()) {
        return Err("Finish saving the existing connection before opening another".into());
    }
    *pending = None;
    Ok(())
}

fn web_origin() -> &'static str {
    if origin() == "http://localhost:8080" {
        "http://localhost:5173"
    } else {
        "https://rbgs.io"
    }
}
#[tauri::command]
pub async fn open_pairing_browser(
    app: tauri::AppHandle,
    state: tauri::State<'_, PairingState>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let slot = state.0.lock().await;
    let pending = slot
        .as_ref()
        .filter(|p| Instant::now() < p.deadline)
        .ok_or("Generate a connection request first")?;
    app.opener()
        .open_url(&pending.approval_url, None::<&str>)
        .map_err(|_| {
            "Could not open the browser. Use the manual code option in web Settings.".into()
        })
}
pub(crate) async fn send_heartbeat() -> Result<(), String> {
    let token = stored_token()?;
    let response = client()?
        .post(format!("{}/api/v1/companion/heartbeat", origin()))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|_| "Heartbeat unavailable")?;
    if !response.status().is_success() {
        return Err(api_error(response.status()));
    }
    Ok(())
}

#[tauri::command]
pub async fn connection_status(app: tauri::AppHandle) -> Result<Connection, ConnectionStatusError> {
    let mut result = verify_connection().await?;
    let path = crate::saved_variables::inbox::path(&app)?;
    let fingerprint = result.credential_fingerprint.clone();
    let account = result.account_id.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        crate::saved_variables::inbox::confirm_account(&path, &fingerprint, &account)
    })
    .await
    .map_err(|_| "Local account context could not be saved")
    .and_then(|r| r.map_err(|_| "Local account context could not be saved"));
    if let Err(message) = saved {
        result.local_storage_warning = Some(message.into());
    }
    Ok(result)
}
pub(crate) async fn verify_connection() -> Result<Connection, String> {
    let token = stored_token()?;
    verify_token(&token).await
}
pub(crate) async fn verify_token(token: &str) -> Result<Connection, String> {
    let response = client()?
        .get(format!("{}/api/v1/companion/status", origin()))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|_| "Connection unavailable. Your credential remains stored.")?;
    if !response.status().is_success() {
        return Err(api_error(response.status()));
    }
    let mut result: Connection = response
        .json()
        .await
        .map_err(|_| "Invalid connection response")?;
    if ![SCOPE, CAPTURE_SCOPE].contains(&result.scope.as_str())
        || result.account_id.len() != 36
        || !result.account_id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err("Invalid connection scope".into());
    }
    use sha2::{Digest, Sha256};
    result.credential_fingerprint = format!("{:x}", Sha256::digest(token.as_bytes()));
    result.local_storage_warning = None;
    result.capture_permissions = result.scope == CAPTURE_SCOPE;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_name_is_optional_for_older_servers_and_reaches_the_ui() {
        let mut payload = serde_json::json!({
            "deviceId": "device", "accountId": "account", "scope": SCOPE,
            "localStorageWarning": null
        });
        let old: Connection = serde_json::from_value(payload.clone()).unwrap();
        assert!(old.account_name.is_none());
        payload["accountName"] = serde_json::json!("BattleTag#1234");
        let current: Connection = serde_json::from_value(payload).unwrap();
        assert_eq!(current.account_name.as_deref(), Some("BattleTag#1234"));
        let ui = serde_json::to_value(current).unwrap();
        assert_eq!(ui["accountName"], "BattleTag#1234");
        assert!(ui.get("credentialFingerprint").is_none());
    }

    #[test]
    fn connection_errors_distinguish_repairing_from_retrying_without_exposing_credentials() {
        for (message, expected) in [
            ("No paired device. Generate a pairing code.", "NOT_PAIRED"),
            (
                "Device access expired, was revoked, or the account is unavailable. Pair again.",
                "AUTH_REQUIRED",
            ),
            (
                "Windows credential storage is unavailable",
                "STORAGE_UNAVAILABLE",
            ),
            (
                "Windows credential storage could not be read",
                "STORAGE_UNAVAILABLE",
            ),
            (
                "Connection unavailable. Your credential remains stored.",
                "UNAVAILABLE",
            ),
            (
                "The server could not confirm this operation. Try again later.",
                "UNAVAILABLE",
            ),
            ("Invalid connection response", "UNAVAILABLE"),
            ("Local account context could not be saved", "UNAVAILABLE"),
        ] {
            let error = ConnectionStatusError::from(message.to_owned());
            let value = serde_json::to_value(error).unwrap();
            assert_eq!(value["kind"], expected);
            assert_eq!(value["message"], message);
            assert_eq!(value.as_object().unwrap().len(), 2);
        }
    }
    #[test]
    fn environment_defaults_release_policy_and_destination_allowlist() {
        assert_eq!(configured_origin(true, None), "https://api.rbgs.io");
        assert_eq!(
            configured_origin(false, Some("http://localhost:8080")),
            "https://api.rbgs.io"
        );
        assert_eq!(
            configured_origin(false, Some("https://untrusted.invalid")),
            "https://api.rbgs.io"
        );
        assert!(allowed_origin("https://api.rbgs.io", false));
        assert!(allowed_origin("http://localhost:8080", true));
        assert!(!allowed_origin("http://localhost:8080", false));
        for value in [
            "",
            "http://localhost:5173",
            "https://untrusted.invalid",
            "https://api.rbgs.io/",
            "https://api.rbgs.io@untrusted.invalid",
        ] {
            assert!(!allowed_origin(value, true));
        }
    }
    #[test]
    fn same_binary_uses_the_environment_of_each_launch() {
        const CHILD: &str = "RBGS_TEST_ENVIRONMENT_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let value = std::env::var("RBGS_COMPANION_API_ORIGIN").unwrap();
            let expected = configured_origin(cfg!(debug_assertions), Some(&value));
            assert_eq!(origin(), expected);
            assert_eq!(
                web_origin(),
                if expected == "http://localhost:8080" {
                    "http://localhost:5173"
                } else {
                    "https://rbgs.io"
                }
            );
            return;
        }
        for value in ["http://localhost:8080", "https://api.rbgs.io"] {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "pairing::tests::same_binary_uses_the_environment_of_each_launch",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("RBGS_COMPANION_API_ORIGIN", value)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "Environment launch regression failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
    #[test]
    fn rejects_invalid_secrets() {
        assert!(valid_secret(&"a".repeat(43)));
        assert!(!valid_secret(&"a".repeat(42)));
        assert!(!valid_secret(&format!("{}\n", "a".repeat(42))));
    }
    #[test]
    fn expired_upgrade_stays_expired_instead_of_reporting_the_old_credential_as_paired() {
        tauri::async_runtime::block_on(async {
            let state = PairingState(Mutex::new(Some(Pending {
                secret: "a".repeat(43),
                deadline: Instant::now() - Duration::from_secs(1),
                next_poll: Instant::now(),
                issued_token: None,
                requested_scope: CAPTURE_SCOPE.into(),
                approval_url: "https://rbgs.io/settings#companion-link=fixture".into(),
            })));
            assert!(
                advance_pairing(&state)
                    .await
                    .unwrap_err()
                    .contains("expired")
            );
            assert!(state.0.lock().await.is_some());
        });
    }
    #[test]
    fn credential_store_survives_a_new_entry() {
        // A unique synthetic credential is removed even when verification fails.
        let name = format!(
            "fixture-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        );
        let first = keyring::Entry::new("io.rbgs.companion.test", &name).unwrap();
        first.set_password("synthetic-credential").unwrap();
        let restored = keyring::Entry::new("io.rbgs.companion.test", &name)
            .unwrap()
            .get_password();
        let removed = first.delete_credential();
        assert_eq!(restored.unwrap(), "synthetic-credential");
        removed.unwrap();
    }
}
