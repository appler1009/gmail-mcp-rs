# gmail-mcp

A read-only Gmail MCP server, written in Rust with almost no dependencies: no async runtime, no HTTP framework, no MCP SDK. Just `serde_json` for JSON and `native-tls` for TLS (via your OS's own TLS stack — Security.framework on macOS, SChannel on Windows, OpenSSL on Linux). Everything else — the HTTP/1.1 client, the OAuth2 flow, the MCP stdio protocol — is a few hundred lines of `std`.

Releases build to a single static-ish binary under 500KB.

## Features

- **Read-only by design.** No send, no draft, no delete, no label changes — just list/search/read messages and threads, list labels, and download attachments.
- **Multi-account.** Link as many Gmail addresses as you want; tools take an optional `account` parameter (auto-selected when you only have one).
- **Self-service linking from inside a chat.** Once installed, ask Claude to link another Gmail account and it drives the whole OAuth flow itself — no terminal needed.
- **No config files to hand-edit.** Tokens live in your OS's standard per-user app-data directory, keyed by email address.

## Tools

| Tool | Description |
|---|---|
| `gmail_list_accounts` | List linked Gmail addresses. |
| `gmail_start_auth` | Link a new Gmail account — opens the Google consent screen in your browser and returns immediately. |
| `gmail_auth_status` | Check on an in-progress `gmail_start_auth` call. |
| `gmail_list_messages` | List messages, optionally filtered by a Gmail search query. |
| `gmail_get_message` | Get a specific message by ID. |
| `gmail_search_messages` | Search messages using Gmail search syntax. |
| `gmail_list_threads` | List threads, optionally filtered by a Gmail search query. |
| `gmail_get_thread` | Get a specific thread by ID. |
| `gmail_list_labels` | List all labels in the mailbox. |
| `gmail_download_attachment` | Download a message attachment to local disk (`~/Downloads` by default). |

All tools except `gmail_list_accounts`/`gmail_start_auth`/`gmail_auth_status` accept an optional `account` (email address) argument. If you only have one account linked, you can omit it.

## Setup

### 1. Create a Google OAuth client

1. Create or open a project at the [Google Cloud Console](https://console.cloud.google.com).
2. Enable the **Gmail API** for it.
3. Under **APIs & Services → Credentials**, create an **OAuth client ID** of type **Desktop app**. (Not "Web application" — Desktop apps are designed for the loopback-redirect flow this server uses, per [RFC 8252](https://datatracker.ietf.org/doc/html/rfc8252).)
4. Add `http://localhost:8765/callback` as an authorized redirect URI (or pick your own port and set `GOOGLE_REDIRECT_URL` accordingly later).
5. Under **OAuth consent screen**, add every Gmail address you plan to link as a **test user** — while the app is unverified (the default, fine for personal use), only test users can complete the consent flow, and unverified apps in Testing mode issue refresh tokens that expire after 7 days.

### 2. Build

```bash
cargo build --release
```

The binary lands at `target/release/gmail-mcp`. Copy it somewhere on your `PATH`, e.g.:

```bash
cp target/release/gmail-mcp ~/.local/bin/gmail-mcp
```

### 3. Provide your OAuth client credentials

`src/config.rs` has two `REPLACE_ME` placeholders where you can bake in your client ID/secret at build time — convenient if you're the only one who'll ever run this binary. Otherwise, leave them as placeholders and pass `GOOGLE_CLIENT_ID`/`GOOGLE_CLIENT_SECRET` as environment variables at runtime instead (see the Claude Desktop config below). Either way works; env vars always take priority.

> An installed app's `client_secret` isn't a confidential value in the traditional sense — per RFC 8252 it identifies the app, not the user, since every user still logs into their own Google account and grants their own consent. But it does identify *your* Google Cloud project, so avoid committing real values to a public repo.

### 4. Link your first account

```bash
gmail-mcp --auth
```

This prints a consent URL (and tries to open it in your default browser), waits for the redirect, exchanges the code for tokens, looks up the account's email address via the Gmail API, and saves `token-<email>.json` to your OS's per-user app-data directory:

- macOS: `~/Library/Application Support/gmail-mcp/`
- Linux: `$XDG_DATA_HOME/gmail-mcp/` (or `~/.local/share/gmail-mcp/`)
- Windows: `%APPDATA%\gmail-mcp\`

Repeat for as many accounts as you want — or, once the server is installed in Claude Desktop, just ask Claude to link another account and it'll call `gmail_start_auth` for you instead.

### 5. Install in Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "gmail": {
      "command": "/path/to/gmail-mcp",
      "env": {
        "GOOGLE_CLIENT_ID": "your-client-id.apps.googleusercontent.com",
        "GOOGLE_CLIENT_SECRET": "your-client-secret"
      }
    }
  }
}
```

Omit the `env` block entirely if you baked real credentials into `src/config.rs` at build time. Restart Claude Desktop to pick it up.

## Environment variables

| Variable | Purpose |
|---|---|
| `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` | Override the compiled-in OAuth client. |
| `GOOGLE_REDIRECT_URL` | Override the default `http://localhost:8765/callback` used during `--auth` / `gmail_start_auth`. |
| `GMAIL_DOWNLOAD_DIR` | Override the default `~/Downloads` save location for `gmail_download_attachment`. |
| `GMAIL_TOKEN` | A JSON token blob, used in place of any file-based account when no `account` argument is given. Takes priority over auto-selecting a linked account. |
| `GMAIL_TOKEN_FILE` | A path to a token JSON file, used the same way as `GMAIL_TOKEN` but read from disk. |

## How account resolution works

- If a tool call passes `account`, its `token-<email>.json` is loaded directly.
- Otherwise: `GMAIL_TOKEN` env var, then `GMAIL_TOKEN_FILE` env var, then — if exactly one account is linked — that account is used automatically.
- If no `account` is given and multiple accounts are linked, the call fails with a list of the available addresses, so the caller (Claude) knows to specify one.

## A note on unverified apps

Unless you complete Google's OAuth verification process for your Cloud project (unnecessary for personal, single-user use), the app stays in "Testing" status. That means:

- Only accounts you've added as test users can authorize it.
- Refresh tokens for test users expire after **7 days** — you'll need to rerun `--auth` (or `gmail_start_auth`) periodically to keep an account linked.
