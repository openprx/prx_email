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

## Operations docs

- [Operations Runbook](docs/operations_runbook.md)
- [Performance & Capacity](docs/performance_capacity.md)
