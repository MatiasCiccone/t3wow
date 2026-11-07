//! Talking to T3 Code on this PC: finding it, signing in (OAuth with PKCE through the
//! browser), and calling its MCP tools over streamable HTTP.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// A thread read with long messages can be several MB; past this something is wrong.
const MAX_REPLY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum T3Error {
    /// T3 refused the sign-in: sign in again.
    Auth(String),
    Other(String),
}

impl std::fmt::Display for T3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            T3Error::Auth(message) | T3Error::Other(message) => f.write_str(message),
        }
    }
}

/// What the bridge needs from T3; a trait so tests can stand in for it.
pub trait T3: Send + Sync {
    fn call(&self, name: &str, args: Value) -> Result<Value, T3Error>;
}

/// The local T3 server's origin, from the runtime file the server writes while it runs.
pub fn discover_origin() -> Option<String> {
    let file = crate::config::t3_home().join("userdata").join("server-runtime.json");
    let text = std::fs::read_to_string(file).ok()?;
    let origin = serde_json::from_str::<Value>(&text).ok()?["origin"].as_str()?.to_string();
    origin.starts_with("http").then(|| origin.trim_end_matches('/').to_string())
}

fn random_url_safe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("random bytes");
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k) == key).then(|| percent_decode(v))
    })
}

pub fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

pub struct SignIn {
    pub token: String,
    pub scope: String,
    /// Epoch milliseconds.
    pub expires_at: f64,
}

/// HTTP statuses come back as responses (not errors), so callers read error bodies too.
fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build(),
    )
}

/// The status and JSON body of a response (Null when the body isn't JSON).
fn status_and_json(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, Value), String> {
    let mut response = response.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    Ok((status, response.body_mut().read_json().unwrap_or(Value::Null)))
}

/// T3's MCP sign-in: register a public client, send the user to the approval page, receive
/// the code on a loopback port, exchange it with PKCE. The approval page is where the user
/// picks the access (Full access lets it write to full-access threads).
pub fn login(origin: &str, log: &dyn Fn(&str)) -> Result<SignIn, String> {
    let state = random_url_safe(16);
    let verifier = random_url_safe(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("{origin}/mcp");
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("No local port for the sign-in: {e}"))?;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", listener.local_addr().map_err(|e| e.to_string())?.port());
    let http = agent();

    let (status, registration) = status_and_json(http.post(&format!("{origin}/oauth/mcp/register")).send_json(json!({
        "client_name": "T3 WoW",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    })))
    .map_err(|e| format!("Registration failed: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("Registration failed: {status} {registration}"));
    }
    let client_id = registration["client_id"].as_str().ok_or("Registration returned no client id")?.to_string();

    let params = [
        ("response_type", "code"),
        ("client_id", &client_id),
        ("redirect_uri", &redirect_uri),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", &state),
        ("resource", &resource),
        ("scope", "orchestration:read orchestration:operate"),
    ];
    let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect();
    let authorize = format!("{origin}/oauth/mcp/authorize?{}", query.join("&"));
    log("Opening the T3 sign-in page. Choose Full access so you can message your full-access threads.");
    log("If it asks for a pairing code, create one in T3: Settings > Sharing & access.");
    log(&format!("If no browser opened, visit: {authorize}"));
    open_browser(&authorize);

    let code = wait_for_code(&listener, &state, Duration::from_secs(600))?;

    let form: Vec<String> = [
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("client_id", &client_id),
        ("code_verifier", &verifier),
        ("resource", &resource),
    ]
    .iter()
    .map(|(k, v)| format!("{k}={}", percent_encode(v)))
    .collect();
    let (_, body) = status_and_json(
        http.post(&format!("{origin}/oauth/mcp/token"))
            .header("content-type", "application/x-www-form-urlencoded")
            .send(form.join("&")),
    )
    .map_err(|e| format!("Token exchange failed: {e}"))?;
    let token = body["access_token"].as_str().ok_or_else(|| {
        format!("Token exchange failed: {}", body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("no token"))
    })?;
    Ok(SignIn {
        token: token.to_string(),
        scope: body["scope"].as_str().unwrap_or("").to_string(),
        expires_at: crate::config::now_ms() as f64 + body["expires_in"].as_f64().unwrap_or(0.0) * 1000.0,
    })
}

/// Serves the loopback redirect until the browser brings back the code.
fn wait_for_code(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(&stream).read_line(&mut line);
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != "/callback" {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let error = query_param(query, "error");
        let code = query_param(query, "code").filter(|_| error.is_none() && query_param(query, "state").as_deref() == Some(state));
        let page = match &code {
            Some(_) => "<h2>T3 WoW is signed in.</h2><p>You can close this tab and go back to the game.</p>".to_string(),
            None => format!("<h2>Sign-in failed</h2><p>{}</p>", error.clone().unwrap_or_else(|| "state mismatch".into())),
        };
        let status = if code.is_some() { "200 OK" } else { "400 Bad Request" };
        let _ = stream.write_all(
            format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len())
                .as_bytes(),
        );
        return match code {
            Some(code) => Ok(code),
            None => Err(format!(
                "Sign-in failed: {}",
                query_param(query, "error_description").or(error).unwrap_or_else(|| "state mismatch".into())
            )),
        };
    }
    Err("Timed out waiting for approval (10 min).".into())
}

fn parse_event_stream(text: &str, id: u64) -> Option<Value> {
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let data: Vec<&str> = block.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect();
        if data.is_empty() {
            continue;
        }
        if let Ok(message) = serde_json::from_str::<Value>(&data.join("\n")) {
            if message["id"].as_u64() == Some(id) {
                return Some(message);
            }
        }
    }
    None
}

enum PostError {
    Fail(T3Error),
    SessionExpired,
}

/// Minimal MCP client over streamable HTTP: initialize once, then tools/call. Safe to use
/// from several threads at once.
pub struct McpClient {
    endpoint: String,
    token: Mutex<String>,
    session: Mutex<Option<String>>,
    ready: Mutex<bool>,
    next_id: AtomicU64,
    http: ureq::Agent,
}

impl McpClient {
    pub fn new(origin: &str, token: &str) -> McpClient {
        McpClient {
            endpoint: format!("{origin}/mcp"),
            token: Mutex::new(token.to_string()),
            session: Mutex::new(None),
            ready: Mutex::new(false),
            next_id: AtomicU64::new(1),
            http: agent(),
        }
    }

    pub fn set_token(&self, token: &str) {
        *self.token.lock().unwrap() = token.to_string();
        *self.ready.lock().unwrap() = false;
    }

    fn post(&self, body: &Value, expect_reply: bool) -> Result<Option<Value>, PostError> {
        let token = self.token.lock().unwrap().clone();
        let session = self.session.lock().unwrap().clone();
        let mut request = self
            .http
            .post(&self.endpoint)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(id) = &session {
            request = request.header("mcp-session-id", id).header("mcp-protocol-version", PROTOCOL_VERSION);
        }
        let mut response = request
            .send(body.to_string())
            .map_err(|e| PostError::Fail(T3Error::Other(format!("T3 unreachable: {e}"))))?;
        let status = response.status().as_u16();
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        if status == 401 || status == 403 {
            return Err(PostError::Fail(T3Error::Auth(format!("T3 refused the sign-in ({status}). Sign in again from the tray menu."))));
        }
        if let Some(id) = header("mcp-session-id") {
            *self.session.lock().unwrap() = Some(id);
        }
        if !expect_reply {
            return Ok(None);
        }
        if status == 404 && session.is_some() {
            *self.session.lock().unwrap() = None;
            *self.ready.lock().unwrap() = false;
            return Err(PostError::SessionExpired);
        }
        let content_type = header("content-type").unwrap_or_default();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_REPLY_BYTES)
            .read_to_string()
            .map_err(|e| PostError::Fail(T3Error::Other(e.to_string())))?;
        if !(200..300).contains(&status) {
            return Err(PostError::Fail(T3Error::Other(format!("MCP {status}: {}", text.chars().take(300).collect::<String>()))));
        }
        let id = body["id"].as_u64().unwrap_or(0);
        let message = if content_type.contains("text/event-stream") {
            parse_event_stream(&text, id)
        } else {
            serde_json::from_str(&text).ok()
        }
        .ok_or(PostError::Fail(T3Error::Other("MCP reply missing".into())))?;
        if let Some(error) = message.get("error") {
            return Err(PostError::Fail(T3Error::Other(format!("MCP error {}: {}", error["code"], error["message"].as_str().unwrap_or("")))));
        }
        Ok(Some(message["result"].clone()))
    }

    fn ensure_ready(&self) -> Result<(), T3Error> {
        let mut ready = self.ready.lock().unwrap();
        if *ready {
            return Ok(());
        }
        *self.session.lock().unwrap() = None;
        let init = json!({
            "jsonrpc": "2.0",
            "id": self.next_id.fetch_add(1, Ordering::SeqCst),
            "method": "initialize",
            "params": { "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": { "name": "t3wow", "version": env!("CARGO_PKG_VERSION") } },
        });
        for (body, reply) in [(init, true), (json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), false)] {
            match self.post(&body, reply) {
                Ok(_) => {}
                Err(PostError::Fail(e)) => return Err(e),
                Err(PostError::SessionExpired) => return Err(T3Error::Other("MCP session refused".into())),
            }
        }
        *ready = true;
        Ok(())
    }
}

impl T3 for McpClient {
    fn call(&self, name: &str, args: Value) -> Result<Value, T3Error> {
        for attempt in 0..2 {
            self.ensure_ready()?;
            let body = json!({
                "jsonrpc": "2.0",
                "id": self.next_id.fetch_add(1, Ordering::SeqCst),
                "method": "tools/call",
                "params": { "name": name, "arguments": args },
            });
            match self.post(&body, true) {
                Ok(result) => {
                    let result = result.unwrap_or(Value::Null);
                    let text = result["content"]
                        .as_array()
                        .and_then(|parts| parts.iter().find(|p| p["type"] == "text"))
                        .and_then(|p| p["text"].as_str())
                        .map(str::to_string);
                    if result["isError"] == true {
                        return Err(T3Error::Other(text.unwrap_or_else(|| format!("{name} failed"))));
                    }
                    if let Some(structured) = result.get("structuredContent").filter(|v| !v.is_null()) {
                        return Ok(structured.clone());
                    }
                    return Ok(text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({})));
                }
                Err(PostError::SessionExpired) if attempt == 0 => continue,
                Err(PostError::SessionExpired) => return Err(T3Error::Other("MCP session expired".into())),
                Err(PostError::Fail(e)) => return Err(e),
            }
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_round_trip() {
        let encoded = percent_encode("a b&c=d/é");
        assert_eq!(encoded, "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(query_param(&format!("x=1&code={encoded}&state=s"), "code").as_deref(), Some("a b&c=d/é"));
        assert_eq!(query_param("error=access_denied", "code"), None);
    }

    #[test]
    fn event_streams_yield_the_reply_with_our_id() {
        let text = "event: message\ndata: {\"id\":1,\"result\":{}}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\n\ndata: {\"id\":7,\"result\":{\"ok\":true}}\n\n";
        assert_eq!(parse_event_stream(text, 7).unwrap()["result"]["ok"], true);
    }
}
