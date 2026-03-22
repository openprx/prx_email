# prx_email

`prx_email` is a Rust email plugin for PRX with SQLite persistence and M4.3 production-hardening primitives.

## M4.3 capabilities

- OAuth token lifecycle baseline:
  - token expiry tracking (`*_OAUTH_EXPIRES_AT`)
  - pluggable refresh abstraction (`OAuthRefreshProvider`)
  - manual/env-based token reload (`reload_auth_from_env`, `reload_config`)
- Multi-account / multi-folder periodic scheduler baseline:
  - `run_sync_runner(jobs, now_ts, runner_cfg)`
  - polling by `account + folder`
  - per-run hard cap by `max_concurrency` + failure backoff
- Outbox send safety:
  - atomic claim (`pending/failed` + `next_attempt_at <= now` -> `sending`)
  - conditional finalize (`sending` -> `sent/failed`) to prevent duplicate sends
  - deterministic SMTP Message-ID idempotency key (`outbox-<id>-<retries>`)
- API guardrails:
  - list/search `limit` must be within `1..=500`
  - retry only allowed for `pending/failed` and due records
- Observability baseline:
  - in-memory counters (`RuntimeMetrics`): sync attempts/success/failures, send failures, retry count
  - structured log payload with `account/folder/message_id/run_id/error_code`
- Attachment governance:
  - max size limit
  - MIME whitelist
  - safe storage-root resolution (directory traversal guard)
- Existing M4.2 features preserved (inbox list/get/search, send/reply/retry, staged rollout)

## WASM execute safety switch (M5.1)

`email.execute` in `wasm-plugin` now forwards calls to host backend execution via WIT host-calls (`email.sync/list/get/search/send/reply`).
Real IMAP/SMTP execution is **disabled by default** and gated by env:

```bash
export PRX_EMAIL_ENABLE_REAL_NETWORK=1
```

When disabled, network tools (`email.sync`, `email.send`, `email.reply`) return a controlled error with guard hint.
When host runtime capability is unavailable (non-wasm path), execute returns controlled `EMAIL_HOST_CAPABILITY_UNAVAILABLE`.

## Quick start (local gates)

```bash
source ~/.cargo/env
cargo test
cargo build
cargo clippy -- -D warnings
```

## OAuth reload examples

```bash
# runtime env reload (manual trigger)
export PRX_EMAIL_IMAP_OAUTH_TOKEN='...'
export PRX_EMAIL_SMTP_OAUTH_TOKEN='...'
export PRX_EMAIL_IMAP_OAUTH_EXPIRES_AT='1800000000'
export PRX_EMAIL_SMTP_OAUTH_EXPIRES_AT='1800000000'

plugin.reload_auth_from_env("PRX_EMAIL");
```

## Sync scheduler example

```rust
let jobs = vec![
    SyncJob { account_id: 1, folder: "INBOX".into(), max_messages: 100 },
    SyncJob { account_id: 1, folder: "Sent".into(), max_messages: 100 },
    SyncJob { account_id: 2, folder: "INBOX".into(), max_messages: 100 },
];
let report = plugin.run_sync_runner(&jobs, now_ts, &SyncRunnerConfig::default());
```

## Outlook OAuth2 Bootstrap (IMAP/SMTP)

One-time bootstrap script (minimal interaction): open consent URL once, paste callback URL/code, script exchanges and stores tokens locally.

```bash
cd /opt/worker/code/prx_email
chmod +x scripts/outlook_oauth_bootstrap.sh

CLIENT_ID='<azure-app-client-id>' \
TENANT='<tenant-id-or-common>' \
REDIRECT_URI='http://localhost:53682/callback' \
./scripts/outlook_oauth_bootstrap.sh
```

Notes:
- Default scope includes: `offline_access`, `https://outlook.office.com/IMAP.AccessAsUser.All`, `https://outlook.office.com/SMTP.Send`
- Output file defaults to `./outlook_oauth.local.env` with `chmod 600`
- You can override output path: `./scripts/outlook_oauth_bootstrap.sh --output ~/.config/prx_email/outlook_oauth.env`
- Optional dry-run (URL only): `./scripts/outlook_oauth_bootstrap.sh --dry-run`
- Never commit generated token files

## Links

- [Documentation](https://docs.openprx.dev/en/prx-email/) — Full documentation (10 languages)
- [Community](https://community.openprx.dev) — OpenPRX community forum

## Operations docs

- [Operations Runbook](docs/operations_runbook.md)
- [Performance & Capacity](docs/performance_capacity.md)
