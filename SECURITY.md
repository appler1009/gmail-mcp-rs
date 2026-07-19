# Security Policy

## Scope

This server requests read-only Gmail access (`gmail.readonly`) and stores OAuth2 refresh
tokens locally, one file per linked account, in your OS's standard per-user app-data
directory (never inside this repository). It never transmits tokens anywhere except Google's
own OAuth endpoints and the Gmail API.

## Reporting a vulnerability

If you find a security issue — a way to read another account's mail without authorization,
a token handling flaw, a path traversal in attachment downloads, etc. — please open a
[private security advisory](../../security/advisories/new) on this repository rather than a
public issue, so it can be fixed before details are public.

## Known limitations

- **Unverified OAuth app.** Unless you complete Google's verification process for your own
  Cloud project, refresh tokens for test users expire after 7 days. This is a Google policy
  limitation, not a bug in this server.
- **`client_secret` is not a traditional secret.** Per [RFC 8252](https://datatracker.ietf.org/doc/html/rfc8252),
  installed-app OAuth clients don't keep their secret confidential — it identifies the app,
  not the user. Real account access still requires each user's own Google login and consent.
- **Local trust boundary.** Anyone with read access to your OS user account can read the
  saved token files and use them to read your Gmail, same as any other locally-stored
  credential (browser cookies, SSH keys, etc.).
