// Accounts are keyed by Gmail address, one token-<email>.json per account in the token
// directory. If no `account` is given and exactly one account is linked, it's used
// automatically. GMAIL_TOKEN (a JSON blob) / GMAIL_TOKEN_FILE remain as env-var overrides for
// callers that don't want file-based multi-account storage at all.

use crate::config;
use crate::http;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_date: i64, // ms since epoch
    #[serde(default = "default_token_type")]
    pub token_type: String,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

pub struct Loaded {
    pub tokens: Tokens,
    save_path: Option<String>,
}

pub fn load(account: Option<&str>) -> Result<Loaded, String> {
    if let Some(email) = account {
        let path = config::token_path(email);
        let raw = fs::read_to_string(&path).map_err(|e| {
            format!("could not read token file {path}: {e}. Run `gmail-mcp --auth` for {email} first.")
        })?;
        let tokens = serde_json::from_str(&raw).map_err(|e| format!("{path} is not valid JSON: {e}"))?;
        return Ok(Loaded { tokens, save_path: Some(path) });
    }

    if let Ok(raw) = env::var("GMAIL_TOKEN") {
        let tokens =
            serde_json::from_str(&raw).map_err(|e| format!("GMAIL_TOKEN is not valid JSON: {e}"))?;
        return Ok(Loaded { tokens, save_path: None });
    }

    if let Ok(path) = env::var("GMAIL_TOKEN_FILE") {
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("could not read GMAIL_TOKEN_FILE {path}: {e}"))?;
        let tokens = serde_json::from_str(&raw).map_err(|e| format!("{path} is not valid JSON: {e}"))?;
        return Ok(Loaded { tokens, save_path: Some(path) });
    }

    let emails = config::list_account_emails();
    match emails.len() {
        0 => Err("No Gmail accounts linked yet. Run `gmail-mcp --auth`.".to_string()),
        1 => load(Some(&emails[0])),
        _ => Err(format!(
            "Multiple accounts are linked ({}). Pass `account` with one of these addresses.",
            emails.join(", ")
        )),
    }
}

pub fn save(path: &str, tokens: &Tokens) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
    }
    let raw = serde_json::to_string_pretty(tokens).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| format!("could not write {path}: {e}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Refreshes the access token in place via the OAuth2 token endpoint if expired, and writes
/// it back to wherever it was loaded from (if anywhere - GMAIL_TOKEN has nowhere to write back to).
pub fn ensure_fresh(loaded: &mut Loaded) -> Result<(), String> {
    if loaded.tokens.expiry_date > now_ms() + 60_000 {
        return Ok(());
    }

    let client_id = config::client_id()?;
    let client_secret = config::client_secret()?;

    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        http::url_encode(&client_id),
        http::url_encode(&client_secret),
        http::url_encode(&loaded.tokens.refresh_token),
    );

    let resp = http::request(
        "POST",
        "oauth2.googleapis.com",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some(body.as_bytes()),
    )?;

    if resp.status != 200 {
        let text = resp.text();
        if text.contains("invalid_grant") {
            return Err(
                "refresh token is invalid or revoked - call gmail_start_auth to re-link this account"
                    .to_string(),
            );
        }
        return Err(format!("token refresh failed ({}): {}", resp.status, text));
    }

    let json = resp.json()?;
    let access_token = json["access_token"]
        .as_str()
        .ok_or("token refresh response missing access_token")?
        .to_string();
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);

    loaded.tokens.access_token = access_token;
    loaded.tokens.expiry_date = now_ms() + expires_in * 1000;

    if let Some(path) = &loaded.save_path {
        save(path, &loaded.tokens)?;
    }

    Ok(())
}
