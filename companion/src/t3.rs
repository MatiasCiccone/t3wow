//! Talking to T3 Code on this PC: finding it, signing in (OAuth with PKCE through the
//! browser), and calling its MCP tools over streamable HTTP.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
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

/// The T3 to talk to and the sign-in saved for it, read together so a token never goes to
/// another server than the one that issued it.
pub struct Target {
    /// Where its sign-in is saved (`Config::server_key`).
    pub key: String,
    pub origin: String,
    pub token: Option<String>,
}

/// The origin saved by `login --origin`, else the T3 running on this PC.
pub fn target() -> Option<Target> {
    target_in(&crate::config::Config::load())
}

pub fn target_in(config: &crate::config::Config) -> Option<Target> {
    let origin = config.origin.as_deref().map(crate::config::canonical_origin).or_else(discover_origin)?;
    Some(Target { key: config.server_key(), origin, token: config.token() })
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
    /// Epoch milliseconds; none when T3 gives no lifetime.
    pub expires_at: Option<f64>,
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
        expires_at: body["expires_in"].as_f64().filter(|s| *s > 0.0).map(|s| crate::config::now_ms() as f64 + s * 1000.0),
    })
}

/// The most of a request (line and headers) the loopback listener reads.
const MAX_REQUEST_HEAD: u64 = 16 * 1024;

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn respond(mut stream: &TcpStream, status: &str, page: &str) {
    let _ = stream.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes(),
    );
}

/// Serves the loopback redirect until the browser brings back the code. A callback without
/// this sign-in's state (another program, an old tab) is turned away and the wait goes on.
fn wait_for_code(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new((&stream).take(MAX_REQUEST_HEAD));
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        // Read the headers too: closing with them unread can reset the connection before the
        // browser reads the page.
        loop {
            let mut header = String::new();
            match reader.read_line(&mut header) {
                Ok(n) if n > 0 && !header.trim().is_empty() => continue,
                _ => break,
            }
        }
        if !line.ends_with('\n') {
            respond(&stream, "400 Bad Request", "");
            continue;
        }
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != "/callback" {
            respond(&stream, "404 Not Found", "");
            continue;
        }
        if query_param(query, "state").as_deref() != Some(state) {
            respond(&stream, "400 Bad Request", "<h2>This sign-in link is out of date.</h2><p>Finish the sign-in in the newest T3 tab.</p>");
            continue;
        }
        if let Some(error) = query_param(query, "error") {
            let reason = query_param(query, "error_description").unwrap_or(error);
            respond(&stream, "400 Bad Request", &format!("<h2>Sign-in failed</h2><p>{}</p>", html_escape(&reason)));
            return Err(format!("Sign-in failed: {reason}"));
        }
        match query_param(query, "code") {
            Some(code) => {
                respond(&stream, "200 OK", "<h2>T3 WoW is signed in.</h2><p>You can close this tab and go back to the game.</p>");
                return Ok(code);
            }
            None => respond(&stream, "400 Bad Request", "<h2>Sign-in failed</h2><p>T3 sent no code.</p>"),
        }
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

    pub fn token(&self) -> String {
        self.token.lock().unwrap().clone()
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

    /// A loopback listener waiting for `state`, and a way to send it raw requests.
    fn callback_server(state: &'static str) -> (u16, std::thread::JoinHandle<Result<String, String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = std::thread::spawn(move || wait_for_code(&listener, state, Duration::from_secs(10)));
        (port, waiting)
    }

    fn request(port: u16, raw: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = stream.write_all(raw.as_bytes());
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
        reply
    }

    fn get(port: u16, target: &str) -> String {
        request(port, &format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"))
    }

    #[test]
    fn callbacks_with_another_state_are_turned_away_and_the_wait_goes_on() {
        let (port, waiting) = callback_server("good");
        assert!(get(port, "/favicon.ico").starts_with("HTTP/1.1 404"));
        assert!(get(port, "/callback?code=stolen&state=bad").starts_with("HTTP/1.1 400"));
        assert!(get(port, "/callback?error=access_denied").starts_with("HTTP/1.1 400"));
        assert!(get(port, "/callback?code=stolen").starts_with("HTTP/1.1 400"));
        // Too long to read: refused (the reply may be lost to a reset), and the wait goes on.
        let long = format!("GET /callback?state=good&code={} HTTP/1.1\r\n\r\n", "x".repeat(20_000));
        assert!(!request(port, &long).starts_with("HTTP/1.1 200"));
        let reply = get(port, "/callback?code=the%20code&state=good");
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert_eq!(waiting.join().unwrap().as_deref(), Ok("the code"));
    }

    #[test]
    fn an_error_with_this_state_ends_the_sign_in_and_is_shown_escaped() {
        let (port, waiting) = callback_server("good");
        let reply = get(port, "/callback?error=access_denied&error_description=%3Cscript%3Ealert(1)%3C%2Fscript%3E&state=good");
        assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
        assert!(reply.contains("&lt;script&gt;alert(1)&lt;/script&gt;"), "{reply}");
        assert!(!reply.contains("<script>"), "{reply}");
        assert_eq!(waiting.join().unwrap(), Err("Sign-in failed: <script>alert(1)</script>".to_string()));
    }
}
