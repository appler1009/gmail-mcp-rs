// Minimal MCP server over stdio: newline-delimited JSON-RPC 2.0, handling `initialize`,
// `tools/list`, and `tools/call`. No SDK dependency - the protocol surface we need is small.

use crate::auth::{self, AuthStatus};
use crate::config;
use crate::gmail;
use crate::http;
use crate::tokens::{self, Tokens};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin read failed: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("failed to parse request: {e}");
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        // Notifications (no id) get no response.
        if req.get("id").is_none() {
            continue;
        }

        let response = match method {
            "initialize" => ok(id, initialize_result()),
            "tools/list" => ok(id, json!({ "tools": tool_defs() })),
            "tools/call" => handle_call(id, req.get("params").cloned().unwrap_or(Value::Null)),
            other => err(id, -32601, &format!("method not found: {other}")),
        };

        writeln!(stdout, "{}", serde_json::to_string(&response).unwrap())
            .map_err(|e| format!("stdout write failed: {e}"))?;
        stdout.flush().ok();
    }

    Ok(())
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "gmail-mcp", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn tool_defs() -> Value {
    let account_prop = json!({ "type": "string", "description": "Gmail address to act on, from gmail_list_accounts. Only needed if more than one account is linked." });

    json!([
        {
            "name": "gmail_list_accounts",
            "description": "List Gmail addresses linked to this server. Call this first when a user has more than one account, to find the right `account` address to pass to other tools.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "gmail_start_auth",
            "description": "Start linking a new Gmail account: opens the Google OAuth consent screen in the user's browser and returns the URL (in case it needs to be opened manually). Runs in the background - call gmail_auth_status afterward to find out when it's done and which email address was linked. Also the fix when another tool errors with 'refresh token is invalid or revoked' - call this to re-link that account.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "gmail_auth_status",
            "description": "Check on an authorization started with gmail_start_auth: pending, done (with the linked email), failed (with an error), or idle (nothing in progress).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "gmail_list_messages",
            "description": "List messages in the mailbox, optionally filtered by a Gmail search query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_prop.clone(),
                    "query": { "type": "string", "description": "Gmail search syntax, e.g. is:unread" },
                    "maxResults": { "type": "integer" },
                    "pageToken": { "type": "string" }
                }
            }
        },
        {
            "name": "gmail_get_message",
            "description": "Get a specific message by ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_prop.clone(),
                    "id": { "type": "string" },
                    "format": { "type": "string", "description": "full, metadata, minimal, or raw" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "gmail_search_messages",
            "description": "Search for messages using Gmail search syntax.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_prop.clone(),
                    "query": { "type": "string" },
                    "maxResults": { "type": "integer" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "gmail_list_threads",
            "description": "List threads in the mailbox, optionally filtered by a Gmail search query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_prop.clone(),
                    "query": { "type": "string" },
                    "maxResults": { "type": "integer" },
                    "pageToken": { "type": "string" }
                }
            }
        },
        {
            "name": "gmail_get_thread",
            "description": "Get a specific thread by ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_prop.clone(),
                    "id": { "type": "string" },
                    "format": { "type": "string", "description": "full, metadata, minimal" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "gmail_list_labels",
            "description": "List all labels in the mailbox.",
            "inputSchema": {
                "type": "object",
                "properties": { "account": account_prop.clone() }
            }
        },
        {
            "name": "gmail_download_attachment",
            "description": "Download a message attachment to local disk. Get `messageId` and `attachmentId` from gmail_get_message's payload.parts (each part with a body.attachmentId).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_prop.clone(),
                    "messageId": { "type": "string" },
                    "attachmentId": { "type": "string" },
                    "filename": { "type": "string", "description": "Save name (basename only). Defaults to the attachment ID if omitted." },
                    "outputDir": { "type": "string", "description": "Directory to save into. Defaults to ~/Downloads, or GMAIL_DOWNLOAD_DIR if set." }
                },
                "required": ["messageId", "attachmentId"]
            }
        }
    ])
}

fn handle_call(id: Value, params: Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    if name == "gmail_list_accounts" {
        return ok(
            id,
            json!({ "content": [{ "type": "text", "text": list_accounts().to_string() }] }),
        );
    }
    if name == "gmail_start_auth" {
        return match auth::start_background() {
            Ok(url) => ok(
                id,
                json!({ "content": [{ "type": "text", "text": json!({
                    "authUrl": url,
                    "note": "A browser window should have opened automatically. If not, open this URL. Call gmail_auth_status once the user confirms they've signed in."
                }).to_string() }] }),
            ),
            Err(e) => tool_error(id, &e),
        };
    }
    if name == "gmail_auth_status" {
        return ok(
            id,
            json!({ "content": [{ "type": "text", "text": auth_status().to_string() }] }),
        );
    }

    let account = args.get("account").and_then(Value::as_str);
    let mut loaded = match tokens::load(account) {
        Ok(t) => t,
        Err(e) => return tool_error(id, &e),
    };
    if let Err(e) = tokens::ensure_fresh(&mut loaded) {
        return tool_error(id, &e);
    }

    let result = dispatch(name, &args, &loaded.tokens);
    match result {
        Ok(value) => ok(
            id,
            json!({ "content": [{ "type": "text", "text": value.to_string() }] }),
        ),
        Err(e) => tool_error(id, &e),
    }
}

fn list_accounts() -> Value {
    json!({ "accounts": config::list_account_emails() })
}

fn auth_status() -> Value {
    match auth::status() {
        AuthStatus::Idle => json!({ "status": "idle" }),
        AuthStatus::Pending => json!({ "status": "pending" }),
        AuthStatus::Done(email) => json!({ "status": "done", "email": email }),
        AuthStatus::Failed(e) => json!({ "status": "failed", "error": e }),
    }
}

fn dispatch(name: &str, args: &Value, tokens: &Tokens) -> Result<Value, String> {
    match name {
        "gmail_list_messages" => gmail::list_messages(
            tokens,
            args.get("query").and_then(Value::as_str),
            args.get("maxResults").and_then(Value::as_u64).map(|n| n as u32),
            args.get("pageToken").and_then(Value::as_str),
        ),
        "gmail_get_message" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("`id` is required")?;
            gmail::get_message(tokens, id, args.get("format").and_then(Value::as_str))
        }
        "gmail_search_messages" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or("`query` is required")?;
            gmail::search_messages(
                tokens,
                query,
                args.get("maxResults").and_then(Value::as_u64).map(|n| n as u32),
            )
        }
        "gmail_list_threads" => gmail::list_threads(
            tokens,
            args.get("query").and_then(Value::as_str),
            args.get("maxResults").and_then(Value::as_u64).map(|n| n as u32),
            args.get("pageToken").and_then(Value::as_str),
        ),
        "gmail_get_thread" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("`id` is required")?;
            gmail::get_thread(tokens, id, args.get("format").and_then(Value::as_str))
        }
        "gmail_list_labels" => gmail::list_labels(tokens),
        "gmail_download_attachment" => {
            let message_id = args
                .get("messageId")
                .and_then(Value::as_str)
                .ok_or("`messageId` is required")?;
            let attachment_id = args
                .get("attachmentId")
                .and_then(Value::as_str)
                .ok_or("`attachmentId` is required")?;
            let filename = sanitize_filename(args.get("filename").and_then(Value::as_str).unwrap_or(attachment_id));
            let output_dir = args
                .get("outputDir")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(config::downloads_dir);

            let attachment = gmail::get_attachment(tokens, message_id, attachment_id)?;
            let data = attachment
                .get("data")
                .and_then(Value::as_str)
                .ok_or("attachment response missing data")?;
            let bytes = http::base64url_decode(data)?;

            std::fs::create_dir_all(&output_dir)
                .map_err(|e| format!("could not create {output_dir}: {e}"))?;
            let path = format!("{output_dir}/{filename}");
            std::fs::write(&path, &bytes).map_err(|e| format!("could not write {path}: {e}"))?;

            Ok(json!({ "path": path, "bytes": bytes.len() }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Keeps only the basename and strips anything that could escape outputDir (path separators,
/// `..`) since the filename may come from untrusted message content.
fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c == '\0' { '_' } else { c })
        .collect();
    match cleaned.trim() {
        "" | "." | ".." => "attachment".to_string(),
        other => other.to_string(),
    }
}

fn tool_error(id: Value, message: &str) -> Value {
    ok(
        id,
        json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    )
}
