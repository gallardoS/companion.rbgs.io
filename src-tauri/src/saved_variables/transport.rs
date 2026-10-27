//! Controlled-test transport. Payloads and receipts are committed before UI claims.
use super::*;
use inbox::{Inbox, canonical, digest};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Default)]
pub struct TransportState {
    sync: AsyncMutex<()>,
    issue: AsyncMutex<Option<PendingIssue>>,
}
struct PendingIssue {
    account: String,
    capture: String,
    code: String,
    descriptor: Vec<u8>,
}
struct Session {
    client: reqwest::Client,
    token: String,
    account: String,
    endpoint: String,
    fingerprint: String,
}
#[derive(Debug)]
struct Failure {
    status: u16,
    retry_after: u64,
}
impl Session {
    async fn new() -> Result<Self, String> {
        let token = crate::pairing::stored_token()?;
        let connection = crate::pairing::verify_token(&token).await?;
        if !connection.capture_permissions {
            return Err(
                "Open Settings, choose Allow match uploads and confirm in your browser.".into(),
            );
        }
        Ok(Self {
            client: crate::pairing::client()?,
            token,
            account: connection.account_id,
            fingerprint: connection.credential_fingerprint,
            endpoint: crate::pairing::origin().into(),
        })
    }
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<(Value, Vec<u8>), Failure> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.endpoint))
            .bearer_auth(&self.token)
            .header("X-RBGS-Capture-Test-Kinds", "CAPTURE_ONLY")
            .header("X-RBGS-Capture-Test-Profile", "TBC_ANNIVERSARY_WSG_TEST")
            .header("Accept", "application/json");
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_vec());
        }
        let mut response = request.send().await.map_err(|_| Failure {
            status: 0,
            retry_after: 0,
        })?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .filter(|v| *v <= 86400)
            .unwrap_or(0);
        if !(200..300).contains(&status) {
            return Err(Failure {
                status,
                retry_after,
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Failure {
            status: 0,
            retry_after: 0,
        })? {
            if bytes.len() + chunk.len() > 262144 {
                return Err(Failure {
                    status: 422,
                    retry_after: 0,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| Failure {
            status: 422,
            retry_after: 0,
        })?;
        Ok((value, bytes))
    }
}
fn local<T>(result: std::result::Result<T, &'static str>) -> Result<T, String> {
    result.map_err(|_| {
        "Cannot open match storage on this PC. Your saved data has not been changed.".into()
    })
}
fn http_message(f: &Failure) -> String {
    match f.status {401|403=>"This PC cannot access match uploads. Reconnect your account and allow match uploads in Settings.",409=>"The match command was already created, or the uploaded data conflicts with an earlier saved version. Your saved data is kept.",
        410=>"This test match is no longer available. Request a new one.",404=>"No assigned match was found for this saved data. Past games cannot be assigned to a new test.",
        503=>"Test matches or the server are currently unavailable. Try again later.",429=>"Too many requests. Wait a moment before trying again.",_=>"Cannot contact rbgs.io or read its reply. Your saved match data is kept."}.into()
}
fn utc_stamp(value: &Value) -> bool {
    let Some(s) = value
        .as_str()
        .filter(|s| s.is_ascii() && s.len() >= 20 && s.len() <= 30 && s.ends_with('Z'))
    else {
        return false;
    };
    if s.len() > 20
        && (s.as_bytes()[19] != b'.'
            || !s.as_bytes()[20..s.len() - 1].iter().all(u8::is_ascii_digit)
            || s.len() == 21)
    {
        return false;
    }
    history::timestamp(&format!("{}Z", &s[..19]))
}
fn descriptor(value: &Value, capture: &str) -> Result<(), String> {
    let capture_only = value.get("testKind").is_some_and(|v| v == "CAPTURE_ONLY");
    let kind_valid = value.get("testKind").is_none() || capture_only;
    let required = [
        "schemaVersion",
        "matchId",
        "assignmentId",
        "captureId",
        "assignmentCodeSha256",
        "mode",
        "issuedAt",
        "armUntil",
        "rosterVersion",
        "assignedCharacterId",
        "assignedTeamKey",
        "expectedMapProfile",
        "expectedRoster",
    ];
    let valid = value.as_object().is_some_and(|o| {
        o.len() == required.len() + usize::from(capture_only)
            && required.iter().all(|k| o.contains_key(*k))
    }) && kind_valid
        && value["schemaVersion"] == 2
        && value["mode"] == "CONTROLLED_TEST"
        && (value["expectedMapProfile"] == "DARKSPEAR_DIAGNOSTIC"
            || capture_only && value["expectedMapProfile"] == "TBC_ANNIVERSARY_WSG_TEST")
        && [
            "matchId",
            "assignmentId",
            "captureId",
            "assignedCharacterId",
        ]
        .iter()
        .all(|k| value[*k].as_str().is_some_and(history::uuid))
        && value["captureId"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(capture))
        && value["assignmentCodeSha256"]
            .as_str()
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        && value["rosterVersion"]
            .as_u64()
            .is_some_and(|n| n >= 1 && n <= 2147483647)
        && if capture_only {
            value["assignedTeamKey"].is_null()
        } else {
            value["assignedTeamKey"]
                .as_str()
                .is_some_and(|s| s == "A" || s == "B")
        }
        && ["issuedAt", "armUntil"]
            .iter()
            .all(|k| utc_stamp(&value[*k]))
        && value["expectedRoster"].as_array().is_some_and(|r| {
            (if capture_only {
                r.len() == 1
            } else {
                r.len() >= 2 && r.len() <= 40
            }) && r.iter().any(|row| {
                row["characterId"] == value["assignedCharacterId"]
                    && row["assignedTeamKey"] == value["assignedTeamKey"]
            })
        });
    if !valid {
        return Err(
            "Cannot read the assigned match details. Check that the companion is up to date."
                .into(),
        );
    }
    Ok(())
}
fn body(snapshot: &Value, assignment: &Value) -> Result<Vec<u8>, String> {
    descriptor(
        assignment,
        snapshot["captureId"].as_str().ok_or("Invalid capture ID")?,
    )?;
    let anniversary = assignment["expectedMapProfile"] == "TBC_ANNIVERSARY_WSG_TEST";
    if (snapshot["client"]["product"] == "wow_anniversary") != anniversary
        || !snapshot["capture"]["map"].is_null()
            && snapshot["capture"]["map"]["profile"] != assignment["expectedMapProfile"]
    {
        return Err("The saved client or map profile does not match this test.".into());
    }
    if assignment["assignmentCodeSha256"].as_str()
        != Some(
            digest(
                snapshot["assignmentCode"]
                    .as_str()
                    .ok_or("Missing assignment code")?
                    .as_bytes(),
            )
            .as_str(),
        )
    {
        return Err(
            "The command in the saved data does not match the assigned match for this account."
                .into(),
        );
    }
    let mut value = snapshot.clone();
    let object = value.as_object_mut().ok_or("Invalid snapshot")?;
    object.insert("schemaVersion".into(), Value::from(2));
    object.insert("matchId".into(), assignment["matchId"].clone());
    object.insert("assignmentId".into(), assignment["assignmentId"].clone());
    // Lua cannot persist null. Materialize required nullable wire members;
    // optional GUIDs, statistics and raw winner remain absent when unobserved.
    value["observedCharacter"]
        .as_object_mut()
        .ok_or("Invalid identity")?
        .entry("realm")
        .or_insert(Value::Null);
    let capture = value["capture"].as_object_mut().ok_or("Invalid capture")?;
    for key in ["map", "startedAt", "endedAt", "incompleteReason"] {
        capture.entry(key).or_insert(Value::Null);
    }
    if let Some(map) = capture.get_mut("map").and_then(Value::as_object_mut) {
        map.entry("gameMapId").or_insert(Value::Null);
    }
    capture
        .get_mut("outcome")
        .and_then(Value::as_object_mut)
        .ok_or("Invalid outcome")?
        .entry("winningObservedTeamKey")
        .or_insert(Value::Null);
    for row in capture
        .get_mut("roster")
        .and_then(Value::as_array_mut)
        .ok_or("Invalid roster")?
    {
        let row = row.as_object_mut().ok_or("Invalid row")?;
        row.entry("observedTeamKey").or_insert(Value::Null);
        row.get_mut("character")
            .and_then(Value::as_object_mut)
            .ok_or("Invalid score identity")?
            .entry("realm")
            .or_insert(Value::Null);
    }
    value["client"]
        .as_object_mut()
        .ok_or("Invalid client")?
        .insert(
            "companionVersion".into(),
            Value::from(env!("CARGO_PKG_VERSION")),
        );
    local(canonical(&value))
}
fn receipt(value: &Value, assignment: &str, capture: &str, revision: u32, sha: &str) -> bool {
    let required = [
        "schemaVersion",
        "evidenceId",
        "assignmentId",
        "captureId",
        "captureRevision",
        "receivedAt",
        "durablyStored",
        "processingStatus",
        "contentSha256",
    ];
    value
        .as_object()
        .is_some_and(|o| o.len() == required.len() && required.iter().all(|k| o.contains_key(*k)))
        && value["schemaVersion"] == 2
        && value["evidenceId"].as_str().is_some_and(history::uuid)
        && value["assignmentId"].as_str() == Some(assignment)
        && value["captureId"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(capture))
        && value["captureRevision"].as_u64() == Some(revision as u64)
        && value["contentSha256"].as_str() == Some(sha)
        && value["durablyStored"] == true
        && value["processingStatus"]
            .as_str()
            .is_some_and(|s| s == "PENDING" || s == "REVIEW")
        && utc_stamp(&value["receivedAt"])
}
fn save_issue(path: &Path, pending: &PendingIssue) -> Result<(), String> {
    let db = local(Inbox::open(path))?;
    local(db.reserve_space(pending.descriptor.len() + 1024))?;
    db.db.execute("INSERT INTO capture_issuances SELECT ?,?,?,?,? WHERE (SELECT COUNT(*) FROM capture_issuances)<100 ON CONFLICT(origin,account_id,capture_id) DO NOTHING",
        params![crate::pairing::origin(),pending.account,pending.capture,pending.code,pending.descriptor]).map_err(|_| "The code was issued but could not be saved. Keep the app open and retry saving.")?;
    let saved: String = db
        .db
        .query_row(
            "SELECT code FROM capture_issuances WHERE origin=? AND account_id=? AND capture_id=?",
            params![crate::pairing::origin(), pending.account, pending.capture],
            |r| r.get(0),
        )
        .map_err(
            |_| "Command storage is full. Keep the companion open to retry saving the command.",
        )?;
    if saved != pending.code {
        return Err("This command conflicts with a saved command. Your saved data is kept.".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn list_capture_assignments(app: tauri::AppHandle) -> Result<Value, String> {
    crate::assignments::refresh_list(&app).await
}
fn match_command(capture: &str, code: &str, assignment: &Value) -> String {
    let action = if assignment["expectedMapProfile"] == "TBC_ANNIVERSARY_WSG_TEST" {
        "test-wsg"
    } else {
        "match"
    };
    format!("/rbgs {action} RB2:{capture}:{code}")
}
fn saved_command(db: &Inbox, account: &str, capture: &str) -> Result<Option<String>, String> {
    let saved: Option<(String, Vec<u8>)> = db.db.query_row(
        "SELECT code,descriptor FROM capture_issuances WHERE origin=? AND account_id=? AND capture_id=?",
        params![crate::pairing::origin(), account, capture], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().map_err(|_| "Cannot read your saved match commands")?;
    saved
        .map(|(code, bytes)| {
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "Cannot read the saved assignment")?;
            descriptor(&value, capture)?;
            Ok(match_command(capture, &code, &value))
        })
        .transpose()
}
pub(crate) async fn fetch_assignments(
    app: &tauri::AppHandle,
) -> Result<(String, String, Value), String> {
    let session = Session::new().await?;
    let fingerprint = session.fingerprint.clone();
    let path = inbox::path(app)?;
    let (list, _) = session
        .request(reqwest::Method::GET, "/api/v1/companion/assignments", None)
        .await
        .map_err(|f| http_message(&f))?;
    let rows = list
        .as_array()
        .filter(|r| r.len() <= 50)
        .ok_or("Invalid assignment list")?;
    let db = local(Inbox::open(&path))?;
    let mut result = vec![];
    for row in rows {
        let capture = row["captureId"]
            .as_str()
            .filter(|v| history::uuid(v))
            .ok_or("Invalid assignment ID")?;
        let command = saved_command(&db, &session.account, capture)?;
        let mut view = row.clone();
        view["command"] = command.map(Value::from).unwrap_or(Value::Null);
        if let Some(character) = view["character"].as_object_mut() {
            character.remove("guid");
        }
        result.push(view);
    }
    Ok((session.account, fingerprint, Value::Array(result)))
}
#[tauri::command]
pub async fn issue_capture_assignment(
    app: tauri::AppHandle,
    state: tauri::State<'_, TransportState>,
    capture_id: String,
) -> Result<String, String> {
    if !history::uuid(&capture_id) {
        return Err("Invalid capture ID".into());
    }
    let session = Session::new().await?;
    let path = inbox::path(&app)?;
    let mut slot = state.issue.lock().await;
    if let Some(pending) = slot.as_ref() {
        if pending.account != session.account || pending.capture != capture_id {
            return Err("Save the pending match command before getting another one.".into());
        }
    } else {
        let db = local(Inbox::open(&path))?;
        if let Some(command) = saved_command(&db, &session.account, &capture_id)? {
            return Ok(command);
        }
        local(db.reserve_space(32768))?;
        let count: u32 = db
            .db
            .query_row("SELECT COUNT(*) FROM capture_issuances", [], |r| r.get(0))
            .map_err(|_| "Cannot open your saved match commands")?;
        if count >= 100 {
            return Err("Match command storage is full. No new command was requested.".into());
        }
        let (issued, _) = session
            .request(
                reqwest::Method::POST,
                &format!("/api/v1/companion/assignments/{capture_id}/issue"),
                None,
            )
            .await
            .map_err(|f| http_message(&f))?;
        let code = issued["assignmentCode"]
            .as_str()
            .filter(|s| {
                s.len() == 16
                    && s.bytes()
                        .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
            })
            .ok_or("Invalid issued code")?
            .to_owned();
        descriptor(&issued["assignment"], &capture_id)?;
        if issued["assignment"]["assignmentCodeSha256"].as_str()
            != Some(digest(code.as_bytes()).as_str())
        {
            return Err("Invalid issued code hash".into());
        }
        *slot = Some(PendingIssue {
            account: session.account,
            capture: capture_id.clone(),
            code,
            descriptor: local(canonical(&issued["assignment"]))?,
        });
    }
    let pending = slot.as_ref().unwrap();
    save_issue(&path, pending)?;
    let assignment: Value =
        serde_json::from_slice(&pending.descriptor).map_err(|_| "Invalid saved assignment")?;
    let command = match_command(&pending.capture, &pending.code, &assignment);
    *slot = None;
    Ok(command)
}

#[derive(Clone)]
struct Candidate {
    scope: String,
    id: String,
    revision: u32,
    snapshot_digest: String,
    snapshot: Value,
}
fn candidates(path: &Path, account: &str, source: &str) -> Result<Vec<Candidate>, String> {
    let db = local(Inbox::open(path))?;
    let origin = crate::pairing::origin();
    let scope = format!("{origin}|account:{account}");
    let unbound = format!("{origin}|unbound:{source}");
    let mut stmt=db.db.prepare("SELECT s.scope,s.capture_id,s.revision,s.digest,s.payload FROM snapshots s JOIN sources l ON s.scope=l.scope AND s.capture_id=l.capture_id AND s.revision=l.revision AND s.digest=l.digest
        WHERE l.source=? AND s.scope IN (?,?) AND s.stage IN ('COMPLETE','INCOMPLETE') AND s.state<>'CONFLICT' ORDER BY s.saved_at LIMIT 100").map_err(|_| "Cannot read your saved match data")?;
    let rows = stmt
        .query_map(params![source, scope, unbound], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(|_| "Cannot read your saved match data")?;
    let mut result = vec![];
    for row in rows {
        let (scope, id, revision, hash, payload) =
            row.map_err(|_| "Cannot read this saved match")?;
        if payload.len() > 262144 || digest(&payload) != hash {
            return Err("The saved match data appears damaged. It has not been changed.".into());
        }
        let snapshot: Value =
            serde_json::from_slice(&payload).map_err(|_| "Stored snapshot invalid")?;
        history::project_snapshot(snapshot.clone()).map_err(|_| "Stored capture invalid")?;
        result.push(Candidate {
            scope,
            id,
            revision,
            snapshot_digest: hash,
            snapshot,
        });
    }
    Ok(result)
}
fn reserve(
    path: &Path,
    account: &str,
    item: &Candidate,
) -> Result<Option<(Option<String>, Option<Vec<u8>>, u32)>, String> {
    let db = local(Inbox::open(path))?;
    let origin = crate::pairing::origin();
    db.db.execute("INSERT INTO capture_transports(origin,account_id,capture_id,revision,snapshot_scope,snapshot_digest,state) SELECT ?,?,?,?,?,?,'WAITING_ASSIGNMENT' WHERE (SELECT COUNT(*) FROM capture_transports)<100 ON CONFLICT DO NOTHING",
        params![origin,account,item.id,item.revision,item.scope,item.snapshot_digest]).map_err(|_| "Local request staging failed")?;
    let (hash,state,next,assignment,payload,attempts,stored_sha):(String,String,u64,Option<String>,Option<Vec<u8>>,u32,Option<String>)=db.db.query_row("SELECT snapshot_digest,state,next_attempt_at,assignment_id,payload,attempts,content_sha256 FROM capture_transports WHERE origin=? AND account_id=? AND capture_id=? AND revision=?",
        params![origin,account,item.id,item.revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|_| "Local request unavailable")?;
    if hash != item.snapshot_digest {
        return Err(
            "This saved version conflicts with earlier match data. Your saved data is kept.".into(),
        );
    }
    if payload.as_ref().is_some_and(|bytes| {
        bytes.len() > 262144 || stored_sha.as_deref() != Some(digest(bytes).as_str())
    }) {
        return Err("The saved upload data appears damaged. Nothing was sent.".into());
    }
    if ["ACKNOWLEDGED", "BLOCKED", "CONFLICT"].contains(&state.as_str())
        || next > seconds(SystemTime::now()).unwrap_or(0)
    {
        return Ok(None);
    }
    Ok(Some((assignment, payload, attempts)))
}
fn prepared(
    path: &Path,
    account: &str,
    item: &Candidate,
    assignment: &str,
    payload: &[u8],
) -> Result<(String, Vec<u8>), String> {
    let db = local(Inbox::open(path))?;
    local(db.reserve_space(payload.len()))?;
    db.db.execute("UPDATE capture_transports SET assignment_id=?,payload=?,content_sha256=?,state='PENDING' WHERE origin=? AND account_id=? AND capture_id=? AND revision=? AND payload IS NULL",
        params![assignment,payload,digest(payload),crate::pairing::origin(),account,item.id,item.revision]).map_err(|_| "Final request could not be saved. Nothing was sent.")?;
    let (id,bytes,sha):(String,Vec<u8>,String)=db.db.query_row("SELECT assignment_id,payload,content_sha256 FROM capture_transports WHERE origin=? AND account_id=? AND capture_id=? AND revision=?",
        params![crate::pairing::origin(),account,item.id,item.revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_| "Frozen request unavailable")?;
    if id != assignment || digest(&bytes) != sha {
        return Err("Frozen request binding mismatch".into());
    }
    Ok((id, bytes))
}
fn mark(
    path: &Path,
    account: &str,
    item: &Candidate,
    state: &str,
    attempts: u32,
    next: u64,
    error: Option<&str>,
    receipt: Option<&[u8]>,
) -> Result<(), String> {
    let db = local(Inbox::open(path))?;
    db.db.execute("UPDATE capture_transports SET state=?,attempts=?,next_attempt_at=?,last_error=?,receipt=COALESCE(?,receipt) WHERE origin=? AND account_id=? AND capture_id=? AND revision=?",
        params![state,attempts,next,error,receipt,crate::pairing::origin(),account,item.id,item.revision]).map_err(|_| "Could not save upload progress on this PC. Try checking uploads again.")?;
    Ok(())
}
fn delay(attempts: u32, retry_after: u64) -> u64 {
    let base = 2u64.saturating_mul(1u64 << attempts.min(8)).min(300);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % (base / 4 + 1))
        .unwrap_or(0);
    (base + jitter).min(300).max(retry_after)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    checked: usize,
    acknowledged: usize,
    waiting: usize,
    message: &'static str,
}
#[tauri::command]
pub async fn sync_captures(
    app: tauri::AppHandle,
    source: tauri::State<'_, SavedVariablesState>,
    state: tauri::State<'_, TransportState>,
) -> Result<SyncView, String> {
    let _guard = state
        .sync
        .try_lock()
        .map_err(|_| "Uploads are already being checked")?;
    let session = Session::new().await?;
    let path = inbox::path(&app)?;
    let settings = settings_path(&app)?;
    let source_id = {
        let mut manager = source.0.lock().map_err(|_| "Local source unavailable")?;
        manager.load(&settings)?;
        let config = manager
            .config
            .as_ref()
            .filter(|c| c.account.is_some())
            .ok_or("Choose your WoW account in Settings first")?;
        inbox::source_id(config)
    };
    inbox::confirm_account(&path, &digest(session.token.as_bytes()), &session.account)?;
    run_sync(&path, &session, &source_id).await
}
async fn run_sync(path: &Path, session: &Session, source_id: &str) -> Result<SyncView, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    let rows = candidates(path, &session.account, source_id)?;
    let mut view = SyncView {
        checked: 0,
        acknowledged: 0,
        waiting: 0,
        message: "Match data is saved on this PC. Uploaded means rbgs.io saved the data; results still need to be verified.",
    };
    for item in rows {
        if view.checked >= 3 || std::time::Instant::now() >= deadline {
            break;
        }
        let Some((mut assignment, mut payload, attempts)) =
            reserve(&path, &session.account, &item)?
        else {
            continue;
        };
        view.checked += 1;
        if payload.is_none() {
            match session
                .request(
                    reqwest::Method::GET,
                    &format!("/api/v1/companion/assignments/{}", item.id),
                    None,
                )
                .await
            {
                Ok((value, _)) => match body(&item.snapshot, &value) {
                    Ok(bytes) => {
                        let id = value["assignmentId"].as_str().unwrap().to_owned();
                        let (id, bytes) = prepared(path, &session.account, &item, &id, &bytes)?;
                        assignment = Some(id);
                        payload = Some(bytes);
                    }
                    Err(_) => {
                        mark(
                            &path,
                            &session.account,
                            &item,
                            "BLOCKED",
                            attempts,
                            0,
                            Some("Assignment binding or profile mismatch"),
                            None,
                        )?;
                        view.waiting += 1;
                        continue;
                    }
                },
                Err(f) => {
                    let state = if f.status == 401 || f.status == 403 {
                        "NEEDS_AUTH"
                    } else {
                        "WAITING_ASSIGNMENT"
                    };
                    mark(
                        &path,
                        &session.account,
                        &item,
                        state,
                        attempts.saturating_add(1),
                        seconds(SystemTime::now()).unwrap_or(0) + delay(attempts, f.retry_after),
                        Some("Assignment unavailable; local snapshot retained"),
                        None,
                    )?;
                    view.waiting += 1;
                    continue;
                }
            }
        }
        let assignment = assignment.ok_or("Stored assignment missing")?;
        let payload = payload.ok_or("Stored request missing")?;
        let sha = digest(&payload);
        // Recover a committed response first; this also handles a crash after POST.
        let recovered = session
            .request(
                reqwest::Method::GET,
                &format!(
                    "/api/v1/evidence/receipts/{assignment}/{}/{}",
                    item.id, item.revision
                ),
                None,
            )
            .await;
        let response = match recovered {
            Ok(value) => Ok(value),
            Err(f) if f.status == 404 => {
                mark(
                    &path,
                    &session.account,
                    &item,
                    "SENDING",
                    attempts,
                    0,
                    None,
                    None,
                )?;
                session
                    .request(reqwest::Method::POST, "/api/v1/evidence", Some(&payload))
                    .await
            }
            Err(f) => Err(f),
        };
        match response {
            Ok((value, bytes)) => {
                if !receipt(&value, &assignment, &item.id, item.revision, &sha) {
                    mark(
                        &path,
                        &session.account,
                        &item,
                        "CONFLICT",
                        attempts,
                        0,
                        Some("Receipt identity or digest mismatch"),
                        None,
                    )?;
                    view.waiting += 1;
                    continue;
                }
                mark(
                    &path,
                    &session.account,
                    &item,
                    "ACKNOWLEDGED",
                    attempts,
                    0,
                    None,
                    Some(&bytes),
                )?;
                view.acknowledged += 1;
            }
            Err(f) => {
                let status = match f.status {
                    401 | 403 => "NEEDS_AUTH",
                    409 => "CONFLICT",
                    400 | 404 | 410 | 413 | 415 | 422 => "BLOCKED",
                    _ => "PENDING",
                };
                mark(
                    &path,
                    &session.account,
                    &item,
                    status,
                    attempts.saturating_add(1),
                    seconds(SystemTime::now()).unwrap_or(0) + delay(attempts, f.retry_after),
                    Some("Delivery not acknowledged; exact request retained"),
                    None,
                )?;
                view.waiting += 1;
            }
        }
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    const ACCOUNT: &str = "00000000-0000-4000-8000-000000000009";
    const ASSIGNMENT: &str = "00000000-0000-4000-8000-000000000002";
    fn fixture() -> Value {
        let root: Value = serde_json::from_str(include_str!(
            "../../../tools/fixtures/product-70338-final.sanitized.json"
        ))
        .unwrap();
        root["captures"][0].clone()
    }
    fn assignment(snapshot: &Value) -> Value {
        json!({"schemaVersion":2,"matchId":"00000000-0000-4000-8000-000000000001", "assignmentId":ASSIGNMENT,
            "captureId":snapshot["captureId"],"assignmentCodeSha256":digest(snapshot["assignmentCode"].as_str().unwrap().as_bytes()),
            "mode":"CONTROLLED_TEST","issuedAt":"2026-10-11T15:00:00Z","armUntil":"2026-10-11T16:00:00.123456Z", "rosterVersion":1,
            "assignedCharacterId":ACCOUNT,"assignedTeamKey":"A","expectedMapProfile":"DARKSPEAR_DIAGNOSTIC",
            "expectedRoster":[{"characterId":ACCOUNT,"assignedTeamKey":"A"},{"characterId":ASSIGNMENT,"assignedTeamKey":"B"}]})
    }
    #[test]
    fn capture_only_descriptor_binds_one_declared_reporter_without_a_team() {
        let snapshot = fixture();
        let mut descriptor = assignment(&snapshot);
        descriptor["testKind"] = json!("CAPTURE_ONLY");
        descriptor["assignedTeamKey"] = Value::Null;
        descriptor["expectedRoster"] = json!([{"characterId":ACCOUNT,"assignedTeamKey":null}]);
        let bytes = body(&snapshot, &descriptor).unwrap();
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, body(&snapshot, &assignment(&snapshot)).unwrap());
        assert!(report.get("testKind").is_none()); // Purpose is immutable server assignment metadata.
        descriptor["assignedTeamKey"] = json!("A");
        assert!(body(&snapshot, &descriptor).is_err());
        descriptor["assignedTeamKey"] = Value::Null;
        descriptor["expectedRoster"][0]["characterId"] = json!(ASSIGNMENT);
        assert!(body(&snapshot, &descriptor).is_err());
        descriptor["testKind"] = json!("UNKNOWN");
        assert!(body(&snapshot, &descriptor).is_err());
        descriptor.as_object_mut().unwrap().remove("testKind");
        assert!(body(&snapshot, &descriptor).is_err()); // Single-row shape needs explicit purpose.
    }
    #[test]
    fn anniversary_command_and_profile_survive_saved_issuance_restart() {
        let mut snapshot = fixture();
        let mut value = assignment(&snapshot);
        value["testKind"] = json!("CAPTURE_ONLY");
        value["assignedTeamKey"] = Value::Null;
        value["expectedRoster"] = json!([{"characterId":ACCOUNT,"assignedTeamKey":null}]);
        value["expectedMapProfile"] = json!("TBC_ANNIVERSARY_WSG_TEST");
        assert!(body(&snapshot, &value).is_err());
        snapshot["client"]["product"] = json!("wow_anniversary");
        snapshot["capture"]["map"]["profile"] = json!("TBC_ANNIVERSARY_WSG_TEST");
        assert!(body(&snapshot, &value).is_ok());
        let capture = snapshot["captureId"].as_str().unwrap();
        let code = snapshot["assignmentCode"].as_str().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inbox.sqlite3");
        save_issue(
            &path,
            &PendingIssue {
                account: ACCOUNT.into(),
                capture: capture.into(),
                code: code.into(),
                descriptor: canonical(&value).unwrap(),
            },
        )
        .unwrap();
        let db = Inbox::open(&path).unwrap();
        assert_eq!(
            saved_command(&db, ACCOUNT, capture).unwrap().unwrap(),
            format!("/rbgs test-wsg RB2:{capture}:{code}")
        );
        value.as_object_mut().unwrap().remove("testKind");
        assert!(descriptor(&value, capture).is_err());
    }
    fn accepted(snapshot: &Value, bytes: &[u8]) -> Value {
        json!({"schemaVersion":2,"evidenceId":ACCOUNT,"assignmentId":ASSIGNMENT,"captureId":snapshot["captureId"],
            "captureRevision":snapshot["captureRevision"],"receivedAt":"2026-10-12T00:00:00.123456Z", "durablyStored":true,"processingStatus":"PENDING","contentSha256":digest(bytes)})
    }
    fn database() -> (tempfile::TempDir, PathBuf, Candidate) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inbox.sqlite3");
        let scope = format!("{}|account:{ACCOUNT}", crate::pairing::origin());
        Inbox::open(&path)
            .unwrap()
            .ingest(&scope, "source", true, &[fixture()], u64::MAX)
            .unwrap();
        let item = candidates(&path, ACCOUNT, "source").unwrap().pop().unwrap();
        (dir, path, item)
    }
    // Real loopback HTTP, including a deliberately lost POST response.
    fn server(
        replies: Vec<(u16, Value)>,
    ) -> (Session, std::thread::JoinHandle<Vec<(String, Vec<u8>)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, value) in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut wire = Vec::new();
                let mut buf = [0; 4096];
                let header_end = loop {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    wire.extend_from_slice(&buf[..n]);
                    if let Some(p) = wire.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                };
                let headers = String::from_utf8(wire[..header_end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while wire.len() < header_end + length {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    wire.extend_from_slice(&buf[..n]);
                }
                seen.push((
                    headers.lines().next().unwrap().to_owned(),
                    wire[header_end..].to_vec(),
                ));
                if status != 0 {
                    let bytes = serde_json::to_vec(&value).unwrap();
                    write!(socket,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
                    socket.write_all(&bytes).unwrap();
                }
            }
            seen
        });
        (
            Session {
                fingerprint: "fixture".into(),
                client: reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap(),
                token: "fixture-token".into(),
                account: ACCOUNT.into(),
                endpoint,
            },
            worker,
        )
    }
    #[test]
    fn binding_receipt_and_timestamp_are_strict() {
        let snapshot = fixture();
        let mut desc = assignment(&snapshot);
        let bytes = body(&snapshot, &desc).unwrap();
        let mut ack = accepted(&snapshot, &bytes);
        assert!(receipt(
            &ack,
            ASSIGNMENT,
            snapshot["captureId"].as_str().unwrap(),
            158,
            &digest(&bytes)
        ));
        ack["contentSha256"] = json!("0".repeat(64));
        assert!(!receipt(
            &ack,
            ASSIGNMENT,
            snapshot["captureId"].as_str().unwrap(),
            158,
            &digest(&bytes)
        ));
        desc["assignmentCodeSha256"] = json!("0".repeat(64));
        assert!(body(&snapshot, &desc).is_err());
        assert!(!utc_stamp(&json!("2026-13-12T00:00:00Z")));
        assert!(!utc_stamp(&json!("2026-10-12T00:00:00.Z")));
        assert_eq!(delay(100, 600), 600);
    }
    #[test]
    fn final_wire_matches_the_backend_fixture_including_lua_nulls() {
        let snapshot = fixture();
        let bytes = body(&snapshot, &assignment(&snapshot)).unwrap();
        assert_eq!(
            bytes,
            include_bytes!("../../../tools/fixtures/evidence-wire.sanitized.json")
        );
    }
    #[test]
    fn frozen_request_survives_restart_and_rejects_damage() {
        let (_dir, path, item) = database();
        reserve(&path, ACCOUNT, &item).unwrap();
        let bytes = body(&item.snapshot, &assignment(&item.snapshot)).unwrap();
        prepared(&path, ACCOUNT, &item, ASSIGNMENT, &bytes).unwrap();
        let mut newer = bytes.clone();
        newer.push(b' ');
        assert_eq!(
            prepared(&path, ACCOUNT, &item, ASSIGNMENT, &newer)
                .unwrap()
                .1,
            bytes
        );
        assert_eq!(
            reserve(&path, ACCOUNT, &item).unwrap().unwrap().1.unwrap(),
            bytes
        );
        Inbox::open(&path)
            .unwrap()
            .db
            .execute("UPDATE capture_transports SET payload=x'00'", [])
            .unwrap();
        assert!(reserve(&path, ACCOUNT, &item).is_err());
    }
    #[test]
    fn lost_post_reply_recovers_receipt_without_another_post() {
        tauri::async_runtime::block_on(async {
            let (_dir, path, item) = database();
            let desc = assignment(&item.snapshot);
            let bytes = body(&item.snapshot, &desc).unwrap();
            let ack = accepted(&item.snapshot, &bytes);
            let (session, worker) = server(vec![
                (200, desc),
                (404, json!({})),
                (0, json!({})),
                (200, ack),
            ]);
            assert_eq!(
                run_sync(&path, &session, "source").await.unwrap().waiting,
                1
            );
            Inbox::open(&path)
                .unwrap()
                .db
                .execute("UPDATE capture_transports SET next_attempt_at=0", [])
                .unwrap();
            assert_eq!(
                run_sync(&path, &session, "source")
                    .await
                    .unwrap()
                    .acknowledged,
                1
            );
            let requests = worker.join().unwrap();
            assert_eq!(
                requests.iter().filter(|r| r.0.starts_with("POST")).count(),
                1
            );
            assert_eq!(requests[2].1, bytes);
            let db = Inbox::open(&path).unwrap();
            let (state, stored): (String, Vec<u8>) = db
                .db
                .query_row("SELECT state,receipt FROM capture_transports", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(state, "ACKNOWLEDGED");
            assert!(receipt(
                &serde_json::from_slice(&stored).unwrap(),
                ASSIGNMENT,
                &item.id,
                item.revision,
                &digest(&bytes)
            ));
        });
    }
    #[test]
    fn mismatched_receipt_is_not_acknowledged() {
        tauri::async_runtime::block_on(async {
            let (_dir, path, item) = database();
            let desc = assignment(&item.snapshot);
            let bytes = body(&item.snapshot, &desc).unwrap();
            let mut ack = accepted(&item.snapshot, &bytes);
            ack["captureRevision"] = json!(1);
            let (session, worker) = server(vec![(200, desc), (200, ack)]);
            let view = run_sync(&path, &session, "source").await.unwrap();
            assert_eq!(view.acknowledged, 0);
            assert_eq!(worker.join().unwrap().len(), 2);
            let state: String = Inbox::open(&path)
                .unwrap()
                .db
                .query_row("SELECT state FROM capture_transports", [], |r| r.get(0))
                .unwrap();
            assert_eq!(state, "CONFLICT");
        });
    }
    #[test]
    fn revoked_device_keeps_frozen_request_for_repairing() {
        tauri::async_runtime::block_on(async {
            let (_dir, path, item) = database();
            reserve(&path, ACCOUNT, &item).unwrap();
            let bytes = body(&item.snapshot, &assignment(&item.snapshot)).unwrap();
            prepared(&path, ACCOUNT, &item, ASSIGNMENT, &bytes).unwrap();
            let (session, worker) = server(vec![(403, json!({}))]);
            assert_eq!(
                run_sync(&path, &session, "source")
                    .await
                    .unwrap()
                    .acknowledged,
                0
            );
            assert_eq!(worker.join().unwrap().len(), 1);
            let (state, payload): (String, Vec<u8>) = Inbox::open(&path)
                .unwrap()
                .db
                .query_row("SELECT state,payload FROM capture_transports", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(state, "NEEDS_AUTH");
            assert_eq!(payload, bytes);
        });
    }
}
