//! Immutable local snapshots and durable account-scoped transport state.
use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: usize = 100;
const MAX_PAYLOAD: usize = 256 * 1024;
const MAX_TOTAL: usize = 25 * 1024 * 1024;
const DISK_RESERVE: u64 = 50 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

pub(crate) fn path(app: &tauri::AppHandle) -> std::result::Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|p| p.join("capture-inbox-v1.sqlite3"))
        .map_err(|_| "Local capture storage directory is unavailable".into())
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Explicit canonicalization, independent of serde map insertion order. Integers
// only, sorted keys, exact UTF-8 strings, ordered arrays and null/absence preserved.
pub(super) fn canonical(value: &Value) -> Result<Vec<u8>> {
    fn write(value: &Value, out: &mut Vec<u8>, depth: usize) -> Result<()> {
        if depth > 30 {
            return Err("INVALID_DATA");
        }
        match value {
            Value::Null => out.extend_from_slice(b"null"),
            Value::Bool(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
            Value::Number(n) => {
                let n = n
                    .as_u64()
                    .filter(|n| *n <= 9007199254740991)
                    .ok_or("INVALID_DATA")?;
                out.extend_from_slice(n.to_string().as_bytes());
            }
            Value::String(s) => out.extend_from_slice(
                serde_json::to_string(s)
                    .map_err(|_| "INVALID_DATA")?
                    .as_bytes(),
            ),
            Value::Array(rows) => {
                out.push(b'[');
                for (i, item) in rows.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    write(item, out, depth + 1)?;
                }
                out.push(b']');
            }
            Value::Object(object) => {
                out.push(b'{');
                let sorted: BTreeMap<_, _> = object.iter().collect();
                for (i, (key, item)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(
                        serde_json::to_string(key)
                            .map_err(|_| "INVALID_DATA")?
                            .as_bytes(),
                    );
                    out.push(b':');
                    write(item, out, depth + 1)?;
                }
                out.push(b'}');
            }
        }
        if out.len() > MAX_PAYLOAD {
            return Err("STORAGE_FULL");
        }
        Ok(())
    }
    let mut bytes = Vec::new();
    write(value, &mut bytes, 0)?;
    Ok(bytes)
}
pub(super) fn source_id(config: &Config) -> String {
    let value = serde_json::json!({"installation":config.installation.to_string_lossy(),"account":config.account});
    digest(&serde_json::to_vec(&value).unwrap())
}
#[cfg(windows)]
fn disk_available(parent: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let name: Vec<u16> = parent.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    // Buffers remain alive for the synchronous Windows call; only available is written.
    let success = unsafe {
        GetDiskFreeSpaceExW(
            name.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if success == 0 {
        Err("STORAGE_UNAVAILABLE")
    } else {
        Ok(available)
    }
}
#[cfg(not(windows))]
fn disk_available(_: &Path) -> Result<u64> {
    Ok(u64::MAX)
}

pub(super) struct Inbox {
    pub(super) db: Connection,
    parent: PathBuf,
}
impl Inbox {
    pub(super) fn reserve_space(&self, bytes: usize) -> Result<()> {
        if disk_available(&self.parent)? < DISK_RESERVE.saturating_add(bytes as u64 * 2) {
            return Err("STORAGE_FULL");
        }
        Ok(())
    }
    pub(super) fn open(path: &Path) -> Result<Self> {
        let parent = path.parent().ok_or("STORAGE_UNAVAILABLE")?;
        fs::create_dir_all(parent).map_err(|_| "STORAGE_UNAVAILABLE")?;
        for ancestor in parent.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).map_err(|_| "STORAGE_UNAVAILABLE")?;
            if !regular(&metadata) {
                return Err("STORAGE_UNAVAILABLE");
            }
        }
        for file in [
            path.to_owned(),
            path.with_file_name("capture-inbox-v1.sqlite3-journal"),
        ] {
            if let Ok(meta) = fs::symlink_metadata(&file) {
                if !regular(&meta) || !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
                    return Err("STORAGE_UNAVAILABLE");
                }
            }
        }
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| "STORAGE_UNAVAILABLE")?;
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        if version > 2 {
            return Err("UNSUPPORTED_STORAGE");
        }
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA max_page_count=16384;")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        if version == 0 {
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| "STORAGE_UNAVAILABLE")?;
            tx.execute_batch("CREATE TABLE bindings(origin TEXT NOT NULL, fingerprint TEXT NOT NULL, account TEXT NOT NULL, PRIMARY KEY(origin,fingerprint));
                CREATE TABLE snapshots(scope TEXT NOT NULL, capture_id TEXT NOT NULL, revision INTEGER NOT NULL, digest TEXT NOT NULL,
                    payload BLOB NOT NULL, stage TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('WAITING_ASSIGNMENT','NEEDS_AUTH','CONFLICT')),
                    saved_at INTEGER NOT NULL, PRIMARY KEY(scope,capture_id,revision,digest));
                CREATE TABLE sources(scope TEXT NOT NULL, capture_id TEXT NOT NULL, revision INTEGER NOT NULL, digest TEXT NOT NULL, source TEXT NOT NULL,
                    PRIMARY KEY(scope,capture_id,revision,digest,source),
                    FOREIGN KEY(scope,capture_id,revision,digest) REFERENCES snapshots(scope,capture_id,revision,digest));
                CREATE INDEX source_lookup ON sources(source,scope);
                PRAGMA user_version=1;") .map_err(|_| "STORAGE_UNAVAILABLE")?;
            tx.commit().map_err(|_| "STORAGE_UNAVAILABLE")?;
        }
        if version <= 1 {
            db.execute_batch("BEGIN IMMEDIATE;
              CREATE TABLE capture_transports(origin TEXT NOT NULL,account_id TEXT NOT NULL,capture_id TEXT NOT NULL,revision INTEGER NOT NULL,
                snapshot_scope TEXT NOT NULL,snapshot_digest TEXT NOT NULL,assignment_id TEXT,payload BLOB,content_sha256 TEXT,
                state TEXT NOT NULL CHECK(state IN ('WAITING_ASSIGNMENT','PENDING','SENDING','ACKNOWLEDGED','NEEDS_AUTH','BLOCKED','CONFLICT')),
                attempts INTEGER NOT NULL DEFAULT 0,next_attempt_at INTEGER NOT NULL DEFAULT 0,receipt BLOB,last_error TEXT,
                PRIMARY KEY(origin,account_id,capture_id,revision));
              CREATE TABLE capture_issuances(origin TEXT NOT NULL,account_id TEXT NOT NULL,capture_id TEXT NOT NULL,code TEXT NOT NULL,descriptor BLOB NOT NULL,
                PRIMARY KEY(origin,account_id,capture_id));
              PRAGMA user_version=2; COMMIT;") .map_err(|_| "STORAGE_UNAVAILABLE")?;
        }
        Ok(Self {
            db,
            parent: parent.to_owned(),
        })
    }
    fn account(&self, origin: &str, fingerprint: Option<&str>) -> Result<Option<String>> {
        let Some(fingerprint) = fingerprint else {
            return Ok(None);
        };
        self.db
            .query_row(
                "SELECT account FROM bindings WHERE origin=? AND fingerprint=?",
                params![origin, fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
    fn confirm(&mut self, origin: &str, fingerprint: &str, account: &str) -> Result<()> {
        if !history::uuid(account)
            || fingerprint.len() != 64
            || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("INVALID_DATA");
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let old: Option<String> = tx
            .query_row(
                "SELECT account FROM bindings WHERE origin=? AND fingerprint=?",
                params![origin, fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        if let Some(old) = old {
            return if old == account {
                Ok(())
            } else {
                Err("ACCOUNT_CONFLICT")
            };
        }
        let count: usize = tx
            .query_row("SELECT COUNT(*) FROM bindings", [], |r| r.get(0))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        if count >= 256 {
            return Err("STORAGE_FULL");
        }
        tx.execute(
            "INSERT INTO bindings VALUES(?,?,?)",
            params![origin, fingerprint, account],
        )
        .map_err(|_| "STORAGE_UNAVAILABLE")?;
        tx.commit().map_err(|_| "STORAGE_UNAVAILABLE")
    }
    pub(super) fn ingest(
        &mut self,
        scope: &str,
        source: &str,
        bound: bool,
        records: &[Value],
        available: u64,
    ) -> Result<()> {
        let mut remaining = available;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let (mut count, mut total): (usize, usize) = tx
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(length(payload)),0) FROM snapshots",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        for record in records {
            history::project_snapshot(record.clone()).map_err(|_| "INVALID_DATA")?;
            let id = record["captureId"]
                .as_str()
                .ok_or("INVALID_DATA")?
                .to_ascii_lowercase();
            let revision = record["captureRevision"].as_u64().ok_or("INVALID_DATA")?;
            let stage = record["capture"]["stage"].as_str().ok_or("INVALID_DATA")?;
            let payload = canonical(record)?;
            let hash = digest(&payload);
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM snapshots WHERE scope=? AND capture_id=? AND revision=? AND digest=?)",params![scope,id,revision,hash],|r|r.get(0)).map_err(|_| "STORAGE_UNAVAILABLE")?;
            if !exists {
                if count >= MAX_ROWS
                    || total + payload.len() > MAX_TOTAL
                    || remaining < DISK_RESERVE + payload.len() as u64 * 2 + 8192
                {
                    return Err("STORAGE_FULL");
                }
                let collision:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM snapshots WHERE scope=? AND capture_id=? AND (revision=? OR state='CONFLICT'))",params![scope,id,revision],|r|r.get(0)).map_err(|_| "STORAGE_UNAVAILABLE")?;
                let terminal:Option<Vec<u8>>=tx.query_row("SELECT payload FROM snapshots WHERE scope=? AND capture_id=? AND revision<? AND stage IN ('COMPLETE','INCOMPLETE') ORDER BY revision DESC LIMIT 1",
                    params![scope,id,revision],|r|r.get(0)).optional().map_err(|_| "STORAGE_UNAVAILABLE")?;
                let rewritten = if let Some(terminal) = terminal {
                    let mut old: Value =
                        serde_json::from_slice(&terminal).map_err(|_| "STORAGE_UNAVAILABLE")?;
                    let mut new = record.clone();
                    old.as_object_mut().unwrap().remove("captureRevision");
                    new.as_object_mut().unwrap().remove("captureRevision");
                    old != new
                } else {
                    false
                };
                let previous:Option<Vec<u8>>=tx.query_row("SELECT payload FROM snapshots WHERE scope=? AND capture_id=? ORDER BY revision DESC LIMIT 1",
                    params![scope,id],|r|r.get(0)).optional().map_err(|_| "STORAGE_UNAVAILABLE")?;
                let binding_changed = if let Some(previous) = previous {
                    let previous: Value =
                        serde_json::from_slice(&previous).map_err(|_| "STORAGE_UNAVAILABLE")?;
                    ["assignmentCode", "observedCharacter", "client"]
                        .iter()
                        .any(|key| previous[*key] != record[*key])
                } else {
                    false
                };
                let conflict = collision || rewritten || binding_changed;
                if conflict {
                    tx.execute(
                        "UPDATE snapshots SET state='CONFLICT' WHERE scope=? AND capture_id=?",
                        params![scope, id],
                    )
                    .map_err(|_| "STORAGE_UNAVAILABLE")?;
                }
                let state = if conflict {
                    "CONFLICT"
                } else if bound {
                    "WAITING_ASSIGNMENT"
                } else {
                    "NEEDS_AUTH"
                };
                let now = seconds(SystemTime::now()).ok_or("STORAGE_UNAVAILABLE")?;
                tx.execute(
                    "INSERT INTO snapshots VALUES(?,?,?,?,?,?,?,?)",
                    params![scope, id, revision, hash, payload, stage, state, now],
                )
                .map_err(|_| "STORAGE_UNAVAILABLE")?;
                count += 1;
                total += payload.len();
                remaining -= payload.len() as u64 * 2 + 8192;
            }
            let links: usize = tx
                .query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))
                .map_err(|_| "STORAGE_UNAVAILABLE")?;
            let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sources WHERE scope=? AND capture_id=? AND revision=? AND digest=? AND source=?)",params![scope,id,revision,hash,source],|r|r.get(0)).map_err(|_| "STORAGE_UNAVAILABLE")?;
            if !linked {
                if links >= 6400 || remaining < DISK_RESERVE + 4096 {
                    return Err("STORAGE_FULL");
                }
                remaining -= 4096;
                tx.execute(
                    "INSERT INTO sources VALUES(?,?,?,?,?)",
                    params![scope, id, revision, hash, source],
                )
                .map_err(|_| "STORAGE_UNAVAILABLE")?;
            }
        }
        tx.commit().map_err(|_| "STORAGE_UNAVAILABLE")
    }
    fn view(&self, scope: &str, source: &str) -> Result<(Vec<history::Match>, QueueView)> {
        let (origin, account) = scope.split_once("|account:").unwrap_or(("", ""));
        let mut statement=self.db.prepare("SELECT s.capture_id,s.revision,s.digest,CASE WHEN length(s.payload)<=262144 THEN s.payload ELSE NULL END,
            CASE WHEN s.state='CONFLICT' THEN 'CONFLICT' ELSE COALESCE(t.state,s.state) END,s.saved_at FROM snapshots s JOIN sources l
            ON s.scope=l.scope AND s.capture_id=l.capture_id AND s.revision=l.revision AND s.digest=l.digest
            LEFT JOIN capture_transports t ON t.origin=? AND t.account_id=? AND t.capture_id=s.capture_id AND t.revision=s.revision AND t.snapshot_digest=s.digest
            WHERE l.source=? AND (s.scope=? OR (t.assignment_id IS NOT NULL AND s.scope LIKE '%|unbound:%'))
            ORDER BY s.capture_id,s.revision DESC,s.digest,s.saved_at")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(params![origin, account, source, scope], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, u64>(5)?,
                ))
            })
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut matches = vec![];
        let mut items = vec![];
        let mut ids = BTreeSet::new();
        let mut total = 0;
        let mut conflicts = BTreeSet::new();
        let mut waiting = 0;
        let mut needs_auth = 0;
        let mut acknowledged = 0;
        let mut pending = 0;
        let mut blocked = 0;
        let mut seen = BTreeSet::new();
        for row in rows {
            let (id, revision, hash, payload, state, saved_at) =
                row.map_err(|_| "STORAGE_UNAVAILABLE")?;
            if payload.len() > MAX_PAYLOAD || digest(&payload) != hash {
                return Err("STORAGE_UNAVAILABLE");
            }
            if !seen.insert((id.clone(), revision, hash.clone())) {
                continue;
            }
            let value: Value =
                serde_json::from_slice(&payload).map_err(|_| "STORAGE_UNAVAILABLE")?;
            if canonical(&value)? != payload
                || value["captureId"]
                    .as_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                    != Some(&id)
                || value["captureRevision"].as_u64() != Some(revision as u64)
            {
                return Err("STORAGE_UNAVAILABLE");
            }
            let projected =
                history::project_snapshot(value.clone()).map_err(|_| "STORAGE_UNAVAILABLE")?;
            total += payload.len();
            match state.as_str() {
                "CONFLICT" => {
                    conflicts.insert(id.clone());
                }
                "WAITING_ASSIGNMENT" => waiting += 1,
                "NEEDS_AUTH" => needs_auth += 1,
                "ACKNOWLEDGED" => acknowledged += 1,
                "PENDING" | "SENDING" => pending += 1,
                "BLOCKED" => blocked += 1,
                _ => return Err("STORAGE_UNAVAILABLE"),
            };
            if ids.insert(id.clone()) && state != "CONFLICT" {
                if let Some(item) = projected {
                    matches.push(item);
                }
            }
            items.push(QueueItem {
                capture_id: id,
                revision,
                state,
                coverage: value["capture"]["state"].as_str().unwrap().to_owned(),
                saved_at,
            });
            if items.len() > MAX_ROWS || total > MAX_TOTAL {
                return Err("STORAGE_UNAVAILABLE");
            }
        }
        matches.retain(|item| !conflicts.contains(&item.capture_id.to_ascii_lowercase()));
        matches.sort_by(|a, b| {
            b.ended_at
                .cmp(&a.ended_at)
                .then(a.capture_id.cmp(&b.capture_id))
        });
        let queue = QueueView {
            status: "READY",
            message: "Saved versions include updates to the same match. Uploaded means rbgs.io saved that version; its result still needs to be verified.",
            snapshots: items.len(),
            captures: ids.len(),
            conflicts: conflicts.len(),
            waiting_assignment: waiting,
            needs_auth,
            acknowledged,
            pending,
            blocked,
            bytes: total,
            items,
            source_status: String::new(),
            source_message: String::new(),
            unbound_snapshots: 0,
        };
        Ok((matches, queue))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    capture_id: String,
    revision: u32,
    state: String,
    coverage: String,
    saved_at: u64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    status: &'static str,
    message: &'static str,
    snapshots: usize,
    captures: usize,
    conflicts: usize,
    waiting_assignment: usize,
    needs_auth: usize,
    acknowledged: usize,
    pending: usize,
    blocked: usize,
    bytes: usize,
    items: Vec<QueueItem>,
    source_status: String,
    source_message: String,
    unbound_snapshots: usize,
}
pub(crate) fn confirm_account(
    path: &Path,
    fingerprint: &str,
    account: &str,
) -> std::result::Result<(), String> {
    Inbox::open(path)
        .and_then(|mut db| db.confirm(crate::pairing::origin(), fingerprint, account))
        .map_err(|_| {
            "Authenticated account confirmed, but local account context could not be saved.".into()
        })
}
fn error_queue(status: &'static str) -> QueueView {
    QueueView {
        status,
        message: if status == "STORAGE_FULL" {
            "Match storage is full. Your older data is safe, but new match data could not be saved."
        } else {
            "We cannot open match storage on this PC. Existing files have not been changed. Keep them when getting help."
        },
        snapshots: 0,
        captures: 0,
        conflicts: 0,
        waiting_assignment: 0,
        needs_auth: 0,
        acknowledged: 0,
        pending: 0,
        blocked: 0,
        bytes: 0,
        items: vec![],
        source_status: String::new(),
        source_message: String::new(),
        unbound_snapshots: 0,
    }
}
pub(super) fn refresh(
    path: &Path,
    config: Option<&Config>,
    records: Vec<Value>,
    mut file_view: history::HistoryView,
) -> history::HistoryView {
    let Some(config) = config.filter(|c| c.account.is_some()) else {
        return file_view;
    };
    let result = (|| {
        let source = source_id(config);
        let origin = crate::pairing::origin();
        let mut db = Inbox::open(path)?;
        let fingerprint = crate::pairing::credential_fingerprint();
        let account = db.account(origin, fingerprint.as_deref())?;
        let scope = if let Some(account) = &account {
            format!("{origin}|account:{account}")
        } else {
            format!("{origin}|unbound:{source}")
        };
        let available = disk_available(&db.parent)?;
        let ingestion = db.ingest(&scope, &source, account.is_some(), &records, available);
        let (matches, mut queue) = db.view(&scope, &source)?;
        if account.is_some() {
            queue.unbound_snapshots = db
                .db
                .query_row(
                    "SELECT COUNT(*) FROM sources WHERE scope=? AND source=?",
                    params![format!("{origin}|unbound:{source}"), source],
                    |r| r.get(0),
                )
                .map_err(|_| "STORAGE_UNAVAILABLE")?;
        }
        if let Err(status) = ingestion {
            queue.status = status;
            queue.message = error_queue(status).message;
        }
        Ok((matches, queue))
    })();
    match result {
        Ok((matches, queue)) => {
            file_view.matches = matches;
            file_view.queue = Some(queue);
            let queue = file_view.queue.as_mut().unwrap();
            queue.source_status = file_view.status.into();
            queue.source_message = file_view.message.into();
            if file_view.status != "READY" {
                file_view.message = "Current game file unavailable. Previously saved local captures remain available.";
            }
            // The source read status remains separate from durable local storage.
            file_view.status = "READY";
        }
        Err(status) => {
            let mut queue = error_queue(status);
            queue.source_status = file_view.status.into();
            queue.source_message = file_view.message.into();
            file_view.queue = Some(queue);
            file_view.matches.clear();
        }
    }
    file_view
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Value {
        let root: Value = serde_json::from_str(include_str!(
            "../../../tools/fixtures/product-70338-final.sanitized.json"
        ))
        .unwrap();
        root["captures"][0].clone()
    }
    fn database() -> (tempfile::TempDir, PathBuf, Inbox) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture-inbox-v1.sqlite3");
        let db = Inbox::open(&path).unwrap();
        (dir, path, db)
    }
    fn pending(revision: u32, stage: &str) -> Value {
        let mut value = fixture();
        value["captureRevision"] = json!(revision);
        let c = &mut value["capture"];
        c["state"] = json!("INCOMPLETE");
        c["stage"] = json!(stage);
        c["incompleteReason"] = json!("CAPTURE_PENDING");
        c["endedAt"] = Value::Null;
        c["outcome"] = json!({"kind":"UNKNOWN","winningObservedTeamKey":null});
        value
    }
    #[test]
    fn canonical_bytes_and_digest_have_an_independent_utf8_golden_vector() {
        let bytes = canonical(&json!({"z":{"x":"雪","b":1},"a":[null,"é",0]})).unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            "{\"a\":[null,\"é\",0],\"z\":{\"b\":1,\"x\":\"雪\"}}"
        );
        assert_eq!(
            digest(&bytes),
            "3c0a12b024485c41eb2ca490f71a774d611cea485cb1babe8d4ea07e7361e10d"
        );
        assert_ne!(
            canonical(&json!({"a":null})).unwrap(),
            canonical(&json!({})).unwrap()
        );
        assert_ne!(
            canonical(&json!([1, 2])).unwrap(),
            canonical(&json!([2, 1])).unwrap()
        );
        assert!(canonical(&json!(1.5)).is_err());
        assert!(canonical(&json!(-1)).is_err());
    }
    #[test]
    fn duplicate_imports_and_restart_keep_one_snapshot_with_exact_bytes() {
        let (_dir, path, mut db) = database();
        let value = fixture();
        let bytes = canonical(&value).unwrap();
        db.ingest("account-a", "source-a", true, &[value.clone()], u64::MAX)
            .unwrap();
        db.ingest("account-a", "source-a", true, &[value.clone()], u64::MAX)
            .unwrap();
        drop(db);
        let mut db = Inbox::open(&path).unwrap();
        db.ingest("account-a", "source-a", true, &[value], u64::MAX)
            .unwrap();
        let (history, queue) = db.view("account-a", "source-a").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(queue.snapshots, 1);
        assert_eq!(queue.waiting_assignment, 1);
        let stored: Vec<u8> = db
            .db
            .query_row("SELECT payload FROM snapshots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, bytes);
        // A cleared/unavailable game file supplies no records and cannot delete the inbox.
        db.ingest("account-a", "source-a", true, &[], u64::MAX)
            .unwrap();
        assert_eq!(db.view("account-a", "source-a").unwrap().0.len(), 1);
    }
    #[test]
    fn revisions_remain_immutable_and_late_lower_revisions_do_not_replace_final() {
        let (_dir, _path, mut db) = database();
        let first = pending(1, "RECORDING");
        let second = pending(2, "FINAL_PENDING");
        let final_record = fixture();
        db.ingest("a", "one", true, &[first.clone()], u64::MAX)
            .unwrap();
        db.ingest("a", "one", true, &[final_record.clone()], u64::MAX)
            .unwrap();
        db.ingest("a", "one", true, &[second.clone(), first], u64::MAX)
            .unwrap();
        let (history, queue) = db.view("a", "one").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(queue.snapshots, 3);
        assert_eq!(queue.conflicts, 0);
        assert_eq!(serde_json::to_value(&history[0]).unwrap()["revision"], 158);
        // Matching IDs in a second local source reuse the immutable account snapshot.
        db.ingest("a", "two", true, &[final_record], u64::MAX)
            .unwrap();
        assert_eq!(db.view("a", "two").unwrap().1.snapshots, 1);
        let count: usize = db
            .db
            .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }
    #[test]
    fn conflict_preserves_both_contents_and_blocks_the_capture_after_restart() {
        let (_dir, path, mut db) = database();
        let original = fixture();
        let mut changed = original.clone();
        changed["capture"]["roster"][0]["damage"] = json!(99);
        db.ingest("a", "one", true, &[original.clone()], u64::MAX)
            .unwrap();
        db.ingest("a", "one", true, &[changed.clone()], u64::MAX)
            .unwrap();
        drop(db);
        let mut db = Inbox::open(&path).unwrap();
        db.ingest(
            "a",
            "one",
            true,
            &[original.clone(), changed.clone()],
            u64::MAX,
        )
        .unwrap();
        let (history, queue) = db.view("a", "one").unwrap();
        assert!(history.is_empty());
        assert_eq!(queue.conflicts, 1);
        assert_eq!(queue.snapshots, 2);
        let stored: Vec<Vec<u8>> = db
            .db
            .prepare("SELECT payload FROM snapshots ORDER BY digest")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert!(stored.contains(&canonical(&original).unwrap()));
        assert!(stored.contains(&canonical(&changed).unwrap()));
    }
    #[test]
    fn terminal_rewrites_are_conflicts_even_with_higher_revision() {
        let (_dir, _path, mut db) = database();
        let original = fixture();
        let mut changed = original.clone();
        changed["captureRevision"] = json!(159);
        changed["capture"]["outcome"]["winningObservedTeamKey"] = json!("0");
        db.ingest("a", "one", true, &[original, changed], u64::MAX)
            .unwrap();
        assert_eq!(db.view("a", "one").unwrap().1.conflicts, 1);
    }
    #[test]
    fn changed_token_or_character_binding_cannot_replace_an_active_attempt() {
        for field in ["assignmentCode", "observedCharacter"] {
            let (_dir, _path, mut db) = database();
            let first = pending(1, "RECORDING");
            let mut later = pending(2, "RECORDING");
            if field == "assignmentCode" {
                later[field] = json!("0123456789ABCDEG");
            } else {
                later[field]["name"] = json!("DifferentCharacter");
            }
            db.ingest("a", "one", true, &[first, later], u64::MAX)
                .unwrap();
            assert_eq!(db.view("a", "one").unwrap().1.conflicts, 1);
        }
    }
    #[test]
    fn account_credential_origin_and_source_contexts_cannot_rebind_old_snapshots() {
        let (_dir, _path, mut db) = database();
        let a = "00000000-0000-4000-8000-000000000001";
        let b = "00000000-0000-4000-8000-000000000002";
        db.confirm("local", &"a".repeat(64), a).unwrap();
        db.confirm("local", &"b".repeat(64), b).unwrap();
        assert_eq!(
            db.account("local", Some(&"a".repeat(64)))
                .unwrap()
                .as_deref(),
            Some(a)
        );
        assert!(
            db.account("production", Some(&"a".repeat(64)))
                .unwrap()
                .is_none()
        );
        assert!(db.confirm("local", &"a".repeat(64), b).is_err());
        db.ingest(a, "source", true, &[fixture()], u64::MAX)
            .unwrap();
        assert!(db.view(b, "source").unwrap().0.is_empty());
        assert!(db.view(a, "other-source").unwrap().0.is_empty());
        db.ingest("unbound", "source", false, &[fixture()], u64::MAX)
            .unwrap();
        assert_eq!(db.view("unbound", "source").unwrap().1.needs_auth, 1);
        assert_eq!(db.view(a, "source").unwrap().1.waiting_assignment, 1);
    }
    #[test]
    fn full_storage_low_disk_and_failed_transaction_never_delete_existing_data() {
        let (_dir, _path, mut db) = database();
        let mut values = vec![];
        for i in 1..=99 {
            let mut v = fixture();
            v["captureId"] = json!(format!("00000000-0000-4000-8000-{i:012}"));
            values.push(v);
        }
        db.ingest("a", "one", true, &values, u64::MAX).unwrap();
        let mut one = fixture();
        one["captureId"] = json!("00000000-0000-4000-8000-000000000100");
        let mut two = fixture();
        two["captureId"] = json!("00000000-0000-4000-8000-000000000101");
        assert_eq!(
            db.ingest("a", "one", true, &[one.clone(), two.clone()], u64::MAX),
            Err("STORAGE_FULL")
        );
        assert_eq!(db.view("a", "one").unwrap().1.snapshots, 99); // entire batch rolled back
        assert_eq!(
            db.ingest("a", "one", true, &[one.clone()], 0),
            Err("STORAGE_FULL")
        );
        db.db.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON snapshots BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        assert!(
            db.ingest("a", "one", true, &[one.clone()], u64::MAX)
                .is_err()
        );
        assert_eq!(db.view("a", "one").unwrap().1.snapshots, 99);
        db.db.execute_batch("DROP TRIGGER fail_insert;").unwrap();
        db.ingest("a", "one", true, &[one], u64::MAX).unwrap();
        assert_eq!(
            db.ingest("a", "one", true, &[two], u64::MAX),
            Err("STORAGE_FULL")
        );
        assert_eq!(db.view("a", "one").unwrap().1.snapshots, 100);
    }
    #[test]
    fn corrupted_or_future_database_is_preserved_and_payload_tampering_is_rejected() {
        let (dir, path, mut db) = database();
        db.ingest("a", "one", true, &[fixture()], u64::MAX).unwrap();
        db.db
            .execute("UPDATE snapshots SET payload=?", params![b"{}".as_slice()])
            .unwrap();
        assert!(db.view("a", "one").is_err());
        drop(db);
        let path2 = dir.path().join("future.sqlite3");
        let future = Connection::open(&path2).unwrap();
        future.execute_batch("PRAGMA user_version=9;").unwrap();
        drop(future);
        let before = fs::read(&path2).unwrap();
        assert!(Inbox::open(&path2).is_err());
        assert_eq!(fs::read(&path2).unwrap(), before);
        fs::write(&path, b"not sqlite, retain me").unwrap();
        assert!(Inbox::open(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not sqlite, retain me");
    }
    #[test]
    fn archived_history_remains_explicitly_local_when_the_source_is_missing() {
        let (_dir, _path, mut db) = database();
        db.ingest("a", "one", true, &[fixture()], u64::MAX).unwrap();
        let (history, queue) = db.view("a", "one").unwrap();
        let ui = serde_json::to_string(&queue).unwrap();
        assert!(!ui.contains("0123456789ABCDEF"));
        assert!(!ui.contains("fixture-player-guid"));
        assert_eq!(history.len(), 1);
        assert_eq!(queue.items[0].state, "WAITING_ASSIGNMENT");
    }
    #[test]
    #[ignore = "Explicit opt-in: archives the selected real product source locally; no game write/upload"]
    fn inspect_and_archive_selected_product_source() {
        assert_eq!(
            std::env::var("RBGS_INSPECT_LOCAL_INBOX").ok().as_deref(),
            Some("1")
        );
        let folder =
            PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join("io.rbgs.companion");
        let path = folder.join("capture-inbox-v1.sqlite3");
        if let Ok(connection) = tauri::async_runtime::block_on(crate::pairing::verify_connection())
        {
            confirm_account(
                &path,
                &connection.credential_fingerprint,
                &connection.account_id,
            )
            .unwrap();
        }
        let mut manager = SourceManager::default();
        manager
            .load(&folder.join("saved-variables-source.json"))
            .unwrap();
        let config = manager.config.clone();
        let source_view = manager.history.read(config.as_ref());
        let records = manager.history.records.clone();
        assert!(!records.is_empty());
        let view = refresh(&path, config.as_ref(), records.clone(), source_view.clone());
        assert_eq!(view.queue.as_ref().unwrap().status, "READY");
        assert!(!view.matches.is_empty());
        // Reopen the actual SQLite store with no game records, simulating startup/file loss.
        let restored = refresh(&path, config.as_ref(), vec![], source_view);
        assert_eq!(restored.matches.len(), view.matches.len());
        println!(
            "Local inbox: {} saved revisions, {} completed captures; reopen recovery confirmed",
            restored.queue.unwrap().snapshots,
            restored.matches.len()
        );
    }
}
