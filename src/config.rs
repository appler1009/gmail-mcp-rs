// Shared OAuth client identity + per-user data paths.
//
// Per RFC 8252, an installed app's client_secret isn't a confidential secret - it identifies
// the app, not the user; each user still authenticates with their own Google account and grants
// their own consent. So we compile in one shared client_id/secret as the default, while still
// letting GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET env vars override for anyone using their own
// Google Cloud project.
//
// Replace these two placeholders with your own OAuth "Desktop app" client before distributing
// a build - see https://console.cloud.google.com/apis/credentials.
const DEFAULT_CLIENT_ID: &str = "REPLACE_ME.apps.googleusercontent.com";
const DEFAULT_CLIENT_SECRET: &str = "REPLACE_ME";

pub fn client_id() -> Result<String, String> {
    resolve("GOOGLE_CLIENT_ID", DEFAULT_CLIENT_ID)
}

pub fn client_secret() -> Result<String, String> {
    resolve("GOOGLE_CLIENT_SECRET", DEFAULT_CLIENT_SECRET)
}

fn resolve(env_var: &str, default: &str) -> Result<String, String> {
    if let Ok(v) = std::env::var(env_var) {
        return Ok(v);
    }
    if default.starts_with("REPLACE_ME") {
        return Err(format!(
            "{env_var} is not set and no default client credentials were compiled in. \
             Either set {env_var}, or rebuild with real values in src/config.rs."
        ));
    }
    Ok(default.to_string())
}

/// Per-account token storage directory, following each OS's conventional app-data location:
///   macOS:   ~/Library/Application Support/gmail-mcp/
///   Linux:   $XDG_DATA_HOME/gmail-mcp/, else ~/.local/share/gmail-mcp/
///   Windows: %APPDATA%\gmail-mcp\
fn token_dir() -> String {
    format!("{}/gmail-mcp", data_dir())
}

/// Token file path for a given Gmail address, e.g. token-someone@gmail.com.json.
pub fn token_path(email: &str) -> String {
    format!("{}/token-{email}.json", token_dir())
}

/// Email addresses with a saved token file, discovered by scanning the token directory.
pub fn list_account_emails() -> Vec<String> {
    let mut emails = Vec::new();
    if let Ok(entries) = std::fs::read_dir(token_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(email) = name.strip_prefix("token-").and_then(|s| s.strip_suffix(".json")) {
                emails.push(email.to_string());
            }
        }
    }
    emails.sort();
    emails
}

/// Default save location for gmail_download_attachment, overridable via GMAIL_DOWNLOAD_DIR.
pub fn downloads_dir() -> String {
    if let Ok(dir) = std::env::var("GMAIL_DOWNLOAD_DIR") {
        return dir;
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    format!("{home}/Downloads")
}

#[cfg(target_os = "macos")]
fn data_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/Library/Application Support")
}

#[cfg(target_os = "linux")]
fn data_dir() -> String {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return xdg;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.local/share")
}

#[cfg(target_os = "windows")]
fn data_dir() -> String {
    std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn data_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}
