# PRX Email M4.3 Operations Runbook

## Scope

Runbook for operating `prx_email` in production with SQLite persistence.

## Runtime configuration

`EmailStore` defaults:

- `foreign_keys = ON`
- `journal_mode = WAL`
- `synchronous = NORMAL`
- `busy_timeout = 5000ms`
- `wal_autocheckpoint = 1000`

## OAuth token operations

### Refresh model

- OAuth refresh is abstracted by `OAuthRefreshProvider`.
- When `*_OAUTH_EXPIRES_AT <= now + 60s`, plugin attempts refresh.
- If token expired and no provider is configured, request fails fast with provider error.

### Manual hot reload from env

Set environment variables and trigger `reload_auth_from_env("PRX_EMAIL")`:

- `PRX_EMAIL_IMAP_OAUTH_TOKEN`
- `PRX_EMAIL_SMTP_OAUTH_TOKEN`
- `PRX_EMAIL_IMAP_OAUTH_EXPIRES_AT`
- `PRX_EMAIL_SMTP_OAUTH_EXPIRES_AT`

Use `reload_config(...)` for full transport/policy reload.

## Sync runner operations

Use `run_sync_runner` for periodic polling by account/folder.

- Input: `Vec<SyncJob { account_id, folder, max_messages }>`
- Guardrails:
  - `max_concurrency`
  - exponential failure backoff (`base_backoff_seconds`, `max_backoff_seconds`)
- Output: `SyncRunnerReport { run_id, attempted, succeeded, failed }`

## Observability baseline

### Metrics

`metrics_snapshot()` exposes:

- `sync_attempts`
- `sync_success`
- `sync_failures`
- `send_failures`
- `retry_count`

### Structured logs

`[prx_email][structured]` payload includes:

- `account`
- `folder`
- `message_id`
- `run_id`
- `error_code`

## Attachment governance

`AttachmentPolicy` enforces:

- max size (`max_size_bytes`)
- whitelist (`allowed_content_types`)

Path safety:

- attachment write path must resolve under configured store root
- traversal (`../`) escape is rejected

## Troubleshooting

### OAuth expired errors

- Verify `*_OAUTH_EXPIRES_AT` is Unix seconds and not stale.
- Ensure refresh provider is wired when using expiring OAuth tokens.
- If no provider exists, use manual env reload before next sync/send.

### Sync runner keeps skipping jobs

- Check backoff state after repeated failures.
- Verify network reachability and IMAP auth correctness.
- Temporarily lower backoff and re-run with a fresh `now_ts`.

### Attachments rejected

- Check MIME whitelist (`allowed_content_types`).
- Check file size against `max_size_bytes`.
- Ensure file path is under attachment storage root.

### High send failure rate

- Inspect structured logs by `run_id` and `error_code`.
- Check SMTP auth mode (exactly one of password/oauth).
- Validate provider/network availability before enabling broad rollout.

## Release gates

```bash
source ~/.cargo/env
cargo test
cargo build
cargo clippy -- -D warnings
```
