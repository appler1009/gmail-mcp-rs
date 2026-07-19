// Read-only Gmail REST API calls: list/search/get messages, list/get threads, list labels.
// No send/draft/modify/trash/archive - this build is intentionally read-only.

use crate::http;
use crate::tokens::Tokens;
use serde_json::Value;

const HOST: &str = "gmail.googleapis.com";

fn auth_get(tokens: &Tokens, path: &str) -> Result<Value, String> {
    let resp = http::request(
        "GET",
        HOST,
        path,
        &[(
            "Authorization",
            &format!("Bearer {}", tokens.access_token),
        )],
        None,
    )?;
    let json = resp.json()?;
    if resp.status != 200 {
        let msg = json["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("Gmail API error ({}): {}", resp.status, msg));
    }
    Ok(json)
}

fn query_string(pairs: &[(&str, String)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", http::url_encode(v)))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

pub fn list_messages(
    tokens: &Tokens,
    query: Option<&str>,
    max_results: Option<u32>,
    page_token: Option<&str>,
) -> Result<Value, String> {
    let qs = query_string(&[
        ("q", query.unwrap_or("").to_string()),
        (
            "maxResults",
            max_results.map(|n| n.to_string()).unwrap_or_default(),
        ),
        ("pageToken", page_token.unwrap_or("").to_string()),
    ]);
    auth_get(tokens, &format!("/gmail/v1/users/me/messages{qs}"))
}

pub fn get_message(tokens: &Tokens, id: &str, format: Option<&str>) -> Result<Value, String> {
    let qs = query_string(&[("format", format.unwrap_or("full").to_string())]);
    auth_get(tokens, &format!("/gmail/v1/users/me/messages/{id}{qs}"))
}

pub fn search_messages(
    tokens: &Tokens,
    query: &str,
    max_results: Option<u32>,
) -> Result<Value, String> {
    list_messages(tokens, Some(query), max_results, None)
}

pub fn list_threads(
    tokens: &Tokens,
    query: Option<&str>,
    max_results: Option<u32>,
    page_token: Option<&str>,
) -> Result<Value, String> {
    let qs = query_string(&[
        ("q", query.unwrap_or("").to_string()),
        (
            "maxResults",
            max_results.map(|n| n.to_string()).unwrap_or_default(),
        ),
        ("pageToken", page_token.unwrap_or("").to_string()),
    ]);
    auth_get(tokens, &format!("/gmail/v1/users/me/threads{qs}"))
}

pub fn get_thread(tokens: &Tokens, id: &str, format: Option<&str>) -> Result<Value, String> {
    let qs = query_string(&[("format", format.unwrap_or("full").to_string())]);
    auth_get(tokens, &format!("/gmail/v1/users/me/threads/{id}{qs}"))
}

pub fn list_labels(tokens: &Tokens) -> Result<Value, String> {
    auth_get(tokens, "/gmail/v1/users/me/labels")
}

pub fn get_profile(tokens: &Tokens) -> Result<Value, String> {
    auth_get(tokens, "/gmail/v1/users/me/profile")
}

/// Returns {"size": ..., "data": "<base64url>"} - the raw attachment bytes, still encoded.
pub fn get_attachment(tokens: &Tokens, message_id: &str, attachment_id: &str) -> Result<Value, String> {
    auth_get(
        tokens,
        &format!("/gmail/v1/users/me/messages/{message_id}/attachments/{attachment_id}"),
    )
}
