# prx_email

`prx_email` is a Rust email plugin for PRX with a **SQLite-first M4.1** implementation focused on operational safety, staged rollout, and practical inbox threading.

## M4.2 capabilities

- Inbox operations: `email.list`, `email.get`, `email.search`
- Send/reply/outbox retry flows with deterministic provider stub for regression testing
- Unified auth model for IMAP/SMTP: exactly one of `password` or `oauth_token`
- OAuth2 token auth support for IMAP/SMTP (while preserving user/password compatibility)
- Startup-time transport config validation with readable, non-secret error messages
- Sensitive log redaction for email identifiers and auth-secret related failures
- Error grading: user-facing failure text + debug-only diagnostics in logs
- Reply threading enhancement: outbound reply now sets both `In-Reply-To` and `References` chain (`parent References + parent Message-ID`)
- Multi-folder sync: `email.sync` supports configurable folders (`INBOX`, `Sent`, etc.) and keeps sync cursor per folder
- Attachment local persistence (optional): received attachments can be stored on disk, and metadata includes `local_path`
- SQLite migrations for core schema, outbox, and feature-flag rollout controls
- Staged feature rollout model (global defaults + per-account overrides + deterministic percentage rollout)
- Safe defaults for high-risk actions: `email_send`, `email_reply`, `outbox_retry` are disabled by default

## Quick start

```bash
# required local gates
cargo check
cargo test
cargo build
cargo clippy -- -D warnings
```

## E2E smoke template (sync -> send -> reply)

Use env vars (no secrets in repo):

```bash
export E2E_IMAP_HOST=imap.example.com
export E2E_IMAP_PORT=993
export E2E_SMTP_HOST=smtp.example.com
export E2E_SMTP_PORT=465
export E2E_EMAIL_USER=bot@example.com
# Auth mode A: user/password
export E2E_EMAIL_PASS='***'
# Auth mode B: OAuth2 token (set this instead of E2E_EMAIL_PASS)
# export E2E_OAUTH_TOKEN='ya29....'
export E2E_TARGET_EMAIL=bot@example.com # optional, defaults to E2E_EMAIL_USER

./tests/run_e2e_smoke.sh
```

This runs `tests/m4_e2e_smoke.rs` (ignored by default) and executes the smoke path:
1. sync `INBOX`
2. send mail
3. sync `Sent`
4. reply to a synced parent

## Operations docs

- [Operations Runbook](docs/operations_runbook.md)
- [Performance & Capacity](docs/performance_capacity.md)
