// Interactive OAuth2 authorization: prints the Google consent URL, waits for the browser
// redirect on a local listener, exchanges the auth code for tokens, looks up the authenticated
// account's email address, and writes token-<email>.json.
//
// Two entry points share this flow:
//   - `run()`: blocking, used by the standalone `gmail-mcp --auth` CLI invocation.
//   - `start_background()`/`status()`: non-blocking, used by the gmail_start_auth/
//     gmail_auth_status MCP tools so the auth flow can run without stalling the stdio loop.

use crate::config;
use crate::gmail;
use crate::http;
use crate::tokens::Tokens;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

pub fn run() -> Result<(), String> {
    let (client_id, client_secret, redirect_url, port) = client_config()?;
    let auth_url = auth_url(&client_id, &redirect_url);

    eprintln!("Open this URL in your browser to authorize gmail-mcp (read-only access):\n\n{auth_url}\n");
    eprintln!("Waiting for redirect on 127.0.0.1:{port} ...");

    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("could not bind 127.0.0.1:{port}: {e}"))?;
    let code = wait_for_code(&listener)?;
    let email = complete(&code, &client_id, &client_secret, &redirect_url)?;
    eprintln!("Saved tokens for {email}");

    Ok(())
}

#[derive(Clone)]
pub enum AuthStatus {
    Idle,
    Pending,
    Done(String),
    Failed(String),
}

fn state() -> &'static Mutex<AuthStatus> {
    static STATE: OnceLock<Mutex<AuthStatus>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(AuthStatus::Idle))
}

pub fn status() -> AuthStatus {
    state().lock().unwrap().clone()
}

/// Starts the OAuth flow on a background thread and returns the consent URL immediately -
/// the caller (an MCP tool) doesn't block waiting for the user to complete the browser login.
/// Also best-effort opens the URL in the default browser.
pub fn start_background() -> Result<String, String> {
    if matches!(*state().lock().unwrap(), AuthStatus::Pending) {
        return Err(
            "an authorization is already in progress; open the URL from that request, or check gmail_auth_status"
                .to_string(),
        );
    }

    let (client_id, client_secret, redirect_url, port) = client_config()?;
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("could not bind 127.0.0.1:{port}: {e}"))?;
    let auth_url = auth_url(&client_id, &redirect_url);

    *state().lock().unwrap() = AuthStatus::Pending;
    open_browser(&auth_url);

    std::thread::spawn(move || {
        let result = wait_for_code(&listener).and_then(|code| complete(&code, &client_id, &client_secret, &redirect_url));
        let new_state = match result {
            Ok(email) => AuthStatus::Done(email),
            Err(e) => AuthStatus::Failed(e),
        };
        *state().lock().unwrap() = new_state;
    });

    Ok(auth_url)
}

fn client_config() -> Result<(String, String, String, u16), String> {
    let client_id = config::client_id()?;
    let client_secret = config::client_secret()?;
    let redirect_url =
        env::var("GOOGLE_REDIRECT_URL").unwrap_or_else(|_| "http://localhost:8765/callback".to_string());
    let port = redirect_url
        .rsplit_once(':')
        .and_then(|(_, rest)| rest.split('/').next())
        .and_then(|p| p.parse::<u16>().ok())
        .ok_or("GOOGLE_REDIRECT_URL must include an explicit port, e.g. http://localhost:8765/callback")?;
    Ok((client_id, client_secret, redirect_url, port))
}

fn auth_url(client_id: &str, redirect_url: &str) -> String {
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        http::url_encode(client_id),
        http::url_encode(redirect_url),
        http::url_encode(SCOPE),
    )
}

#[cfg(target_os = "macos")]
fn open_browser(url: &str) {
    Command::new("open").arg(url).spawn().ok();
}

#[cfg(target_os = "linux")]
fn open_browser(url: &str) {
    Command::new("xdg-open").arg(url).spawn().ok();
}

#[cfg(target_os = "windows")]
fn open_browser(url: &str) {
    Command::new("cmd").args(["/C", "start", "", url]).spawn().ok();
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn open_browser(_url: &str) {}

/// Exchanges an auth code for tokens, resolves the account's email, and saves token-<email>.json.
/// Returns the resolved email on success.
fn complete(code: &str, client_id: &str, client_secret: &str, redirect_url: &str) -> Result<String, String> {
    let body = format!(
        "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
        http::url_encode(code),
        http::url_encode(client_id),
        http::url_encode(client_secret),
        http::url_encode(redirect_url),
    );

    let resp = http::request(
        "POST",
        "oauth2.googleapis.com",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some(body.as_bytes()),
    )?;

    if resp.status != 200 {
        return Err(format!("token exchange failed ({}): {}", resp.status, resp.text()));
    }

    let json = resp.json()?;
    let access_token = json["access_token"]
        .as_str()
        .ok_or("token exchange response missing access_token")?
        .to_string();
    let refresh_token = json["refresh_token"]
        .as_str()
        .ok_or("token exchange response missing refresh_token (try again with prompt=consent, or revoke prior access at https://myaccount.google.com/permissions)")?
        .to_string();
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;

    let tokens = Tokens {
        access_token,
        refresh_token,
        expiry_date: now_ms + expires_in * 1000,
        token_type: "Bearer".to_string(),
    };

    let profile = gmail::get_profile(&tokens)?;
    let email = profile
        .get("emailAddress")
        .and_then(serde_json::Value::as_str)
        .ok_or("could not determine the account's email address from Gmail's profile API")?
        .to_string();

    let path = config::token_path(&email);
    crate::tokens::save(&path, &tokens)?;

    Ok(email)
}

fn wait_for_code(listener: &TcpListener) -> Result<String, String> {
    let (stream, _) = listener.accept().map_err(|e| format!("accept failed: {e}"))?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| format!("read redirect request failed: {e}"))?;

    // "GET /callback?code=XYZ&scope=... HTTP/1.1"
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or("malformed redirect request")?;
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");

    let mut code = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == "code" {
                code = Some(url_decode(v));
            }
        }
    }

    let mut stream = stream;
    let response_body = "<html><body>Authorization complete. You can close this tab.</body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes()).ok();

    code.ok_or_else(|| format!("redirect had no `code` parameter: {query}"))
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
