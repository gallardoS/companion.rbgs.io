//! A web-issued ticket is redeemed only by native code. Device credentials never enter the webview.
use crate::pairing;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::Manager;
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Default)]
pub struct Progress {
    pub status: String,
    pub message: Option<String>,
}
#[derive(Default)]
pub struct WebConnectState(Mutex<Inner>);
#[derive(Default)]
struct Inner {
    pending: Option<Pending>,
    progress: Progress,
    completed_ticket: Option<String>,
}
struct Pending {
    ticket: String,
    proof: String,
    deadline: Instant,
    next: Instant,
    token: Option<String>,
}
impl WebConnectState {
    pub(crate) async fn is_pending(&self) -> bool {
        self.0.lock().await.pending.is_some()
    }
}
#[tauri::command]
pub async fn web_connection_state(
    state: tauri::State<'_, WebConnectState>,
) -> Result<Progress, String> {
    Ok(match state.0.try_lock() {
        Ok(inner) => inner.progress.clone(),
        Err(_) => Progress {
            status: "CONNECTING".into(),
            message: Some("Connecting to the account approved on rbgs.io…".into()),
        },
    })
}
pub(crate) fn ticket(url: &str) -> Option<&str> {
    url.strip_prefix("rbgs-companion://connect/")
        .filter(|s| pairing::valid_secret(s))
}
pub(crate) async fn begin(app: &tauri::AppHandle, secret: String) {
    use sha2::{Digest, Sha256};
    let _connection = pairing::CONNECTION_LOCK.lock().await;
    let state = app.state::<WebConnectState>();
    let mut inner = state.0.lock().await;
    if inner.completed_ticket.as_deref()
        == Some(&format!("{:x}", Sha256::digest(secret.as_bytes())))
    {
        return;
    }
    // Duplicate OS delivery must keep the same native proof, including after success.
    if inner.pending.as_ref().is_some_and(|p| p.ticket == secret) {
        return;
    }
    if inner.pending.is_some() {
        inner.progress.message = Some(
            "A connection is already in progress. Wait for it to finish before opening another."
                .into(),
        );
        return;
    }
    if let Err(message) = pairing::cancel_unissued(&app.state::<pairing::PairingState>()).await {
        inner.progress = Progress {
            status: "ERROR".into(),
            message: Some(message),
        };
        return;
    }
    let mut random = [0u8; 32];
    if getrandom::fill(&mut random).is_err() {
        inner.progress = Progress {
            status: "ERROR".into(),
            message: Some("Could not create a secure connection request".into()),
        };
        return;
    }
    let proof = random.iter().map(|b| format!("{b:02x}")).collect();
    inner.pending = Some(Pending {
        ticket: secret,
        proof,
        deadline: Instant::now() + Duration::from_secs(120),
        next: Instant::now(),
        token: None,
    });
    inner.progress = Progress {
        status: "CONNECTING".into(),
        message: Some("Connecting to the account approved on rbgs.io…".into()),
    };
}
#[derive(Deserialize)]
struct Token {
    status: String,
    token: Option<String>,
    scope: Option<String>,
}

async fn redeem(p: &Pending) -> Result<String, Failure> {
    let current = match pairing::entry().map_err(Failure::retry)?.get_password() {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => None,
        Err(_) => {
            return Err(Failure::retry(
                "Windows credential storage could not be read",
            ));
        }
    };
    let client = pairing::client().map_err(Failure::retry)?;
    redeem_transport(p, current, &client, pairing::origin()).await
}
async fn redeem_transport(
    p: &Pending,
    current: Option<String>,
    client: &reqwest::Client,
    origin: &str,
) -> Result<String, Failure> {
    let send = |existing: Option<String>| {
        let request = client
            .post(format!("{origin}/api/v1/companion/connect"))
            .json(&serde_json::json!({"launchSecret":p.ticket,"clientSecret":p.proof}));
        if let Some(token) = existing {
            request.bearer_auth(token)
        } else {
            request
        }
    };
    let mut response = send(current.clone())
        .send()
        .await
        .map_err(|_| Failure::retry("Connection unavailable. Retrying…"))?;
    // A new web approval can renew expired/revoked access. A valid different account returns 409 instead.
    if response.status() == reqwest::StatusCode::UNAUTHORIZED && current.is_some() {
        response = send(None)
            .send()
            .await
            .map_err(|_| Failure::retry("Connection unavailable. Retrying…"))?;
    }
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            409 => Failure::stop(
                "This PC is linked to another account, or the device limit was reached. Check web Settings. Its saved connection has been kept.",
            ),
            401 | 403 | 410 => Failure::stop(
                "This connection request expired or is unavailable. Open the companion again from rbgs.io.",
            ),
            _ => Failure::retry("The server could not confirm the connection. Retrying…"),
        });
    }
    let result: Token = response
        .json()
        .await
        .map_err(|_| Failure::retry("Invalid connection response"))?;
    if result.status != "PAIRED" || result.scope.as_deref() != Some(pairing::CAPTURE_SCOPE) {
        return Err(Failure::stop("Invalid connection permissions"));
    }
    result
        .token
        .or(current)
        .filter(|s| pairing::valid_secret(s))
        .ok_or_else(|| Failure::stop("Invalid connection credential"))
}
#[derive(Debug)]
struct Failure {
    message: String,
    terminal: bool,
}
impl Failure {
    fn retry(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            terminal: false,
        }
    }
    fn stop(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            terminal: true,
        }
    }
}
pub(crate) async fn advance(app: &tauri::AppHandle) {
    let _connection = pairing::CONNECTION_LOCK.lock().await;
    let state = app.state::<WebConnectState>();
    let mut inner = state.0.lock().await;
    let Some(p) = inner.pending.as_mut() else {
        return;
    };
    if Instant::now() < p.next {
        return;
    }
    p.next = Instant::now() + Duration::from_secs(5);
    let result = async {
        if p.token.is_none() {
            if Instant::now() >= p.deadline {
                return Err(Failure::stop(
                    "Connection request expired. Open the companion again from rbgs.io.",
                ));
            }
            p.token = Some(redeem(p).await?);
        }
        let token = p.token.as_ref().unwrap();
        let verified = pairing::verify_token(token).await.map_err(|message| {
            if message.starts_with("Device access expired") {
                Failure::stop(message)
            } else {
                Failure::retry(message)
            }
        })?;
        if !verified.capture_permissions {
            return Err(Failure::stop("Invalid connection permissions"));
        }
        pairing::entry()
            .map_err(Failure::retry)?
            .set_password(token)
            .map_err(|_| {
                Failure::retry("Could not save the credential in Windows. Retrying automatically…")
            })?;
        pairing::send_heartbeat().await.map_err(Failure::retry)?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            use sha2::{Digest, Sha256};
            inner.completed_ticket = inner
                .pending
                .as_ref()
                .map(|p| format!("{:x}", Sha256::digest(p.ticket.as_bytes())));
            inner.pending = None;
            inner.progress = Progress {
                status: "CONNECTED".into(),
                message: None,
            };
        }
        Err(error) => {
            if error.terminal {
                inner.pending = None;
            }
            inner.progress = Progress {
                status: if error.terminal {
                    "ERROR"
                } else {
                    "CONNECTING"
                }
                .into(),
                message: Some(error.message),
            };
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn pending() -> Pending {
        Pending {
            ticket: "a".repeat(43),
            proof: "b".repeat(64),
            deadline: Instant::now() + Duration::from_secs(120),
            next: Instant::now(),
            token: None,
        }
    }
    fn server(
        replies: Vec<(u16, String, Option<String>)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            for (status, body, authorization) in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Native request was not received");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    let count = stream.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            assert!(headers.starts_with("post /api/v1/companion/connect "));
                            assert!(
                                !headers.contains("origin:")
                                    && !headers.contains("cookie:")
                                    && !headers.contains("sec-fetch-site:")
                            );
                            match authorization {
                                Some(ref token) => assert!(
                                    headers.contains(&format!("authorization: bearer {token}"))
                                ),
                                None => assert!(!headers.contains("authorization:")),
                            }
                            let json: serde_json::Value =
                                serde_json::from_slice(&bytes[end + 4..]).unwrap();
                            assert_eq!(json["launchSecret"], "a".repeat(43));
                            assert_eq!(json["clientSecret"], "b".repeat(64));
                            assert!(json.get("name").is_none());
                            break;
                        }
                    }
                }
                write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        (origin, thread)
    }
    fn success(token: Option<&str>, scope: &str) -> String {
        serde_json::json!({"status":"PAIRED","token":token,"scope":scope}).to_string()
    }
    #[test]
    fn invalid_saved_access_retries_without_bearer_using_the_same_native_proof() {
        let old = "c".repeat(43);
        let new = "d".repeat(43);
        let (origin, thread) = server(vec![
            (401, "{}".into(), Some(old.clone())),
            (200, success(Some(&new), pairing::CAPTURE_SCOPE), None),
        ]);
        let result = tauri::async_runtime::block_on(redeem_transport(
            &pending(),
            Some(old),
            &pairing::client().unwrap(),
            &origin,
        ));
        thread.join().unwrap();
        assert_eq!(result.unwrap(), new);
    }
    #[test]
    fn existing_account_reuses_the_credential_without_rotation() {
        let old = "c".repeat(43);
        let (origin, thread) = server(vec![(
            200,
            success(None, pairing::CAPTURE_SCOPE),
            Some(old.clone()),
        )]);
        let result = tauri::async_runtime::block_on(redeem_transport(
            &pending(),
            Some(old.clone()),
            &pairing::client().unwrap(),
            &origin,
        ));
        thread.join().unwrap();
        assert_eq!(result.unwrap(), old);
    }
    #[test]
    fn conflict_expiry_and_browser_rejection_stop_without_retrying_or_overwriting() {
        for status in [409, 410, 403] {
            let old = "c".repeat(43);
            let (origin, thread) = server(vec![(status, "{}".into(), Some(old.clone()))]);
            let result = tauri::async_runtime::block_on(redeem_transport(
                &pending(),
                Some(old),
                &pairing::client().unwrap(),
                &origin,
            ));
            thread.join().unwrap();
            assert!(result.unwrap_err().terminal);
        }
    }
    #[test]
    fn wrong_scope_or_missing_new_credential_never_completes() {
        for body in [
            success(Some(&"d".repeat(43)), "companion:status"),
            success(None, pairing::CAPTURE_SCOPE),
        ] {
            let (origin, thread) = server(vec![(200, body, None)]);
            let result = tauri::async_runtime::block_on(redeem_transport(
                &pending(),
                None,
                &pairing::client().unwrap(),
                &origin,
            ));
            thread.join().unwrap();
            assert!(result.unwrap_err().terminal);
        }
    }
    #[test]
    fn deep_link_only_accepts_a_ticket_without_credentials_or_destination() {
        let secret = "a".repeat(43);
        assert_eq!(
            ticket(&format!("rbgs-companion://connect/{secret}")),
            Some(secret.as_str())
        );
        for bad in [
            format!("rbgs-companion://connect/{secret}?token=x"),
            format!("rbgs-companion://connect/{secret}#x"),
            format!("rbgs-companion://connect/{secret}/"),
            format!("rbgs-companion://connect:80/{secret}"),
            format!("rbgs-companion://user@connect/{secret}"),
            "rbgs-companion://connect/short".into(),
        ] {
            assert!(ticket(&bad).is_none());
        }
    }
}
