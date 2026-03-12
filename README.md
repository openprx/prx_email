# prx_email

`prx_email` is a Rust email plugin for PRX with a **SQLite-only M3** implementation focused on operational safety and staged rollout.

## M3 capabilities

- Inbox operations: `email.list`, `email.get`, `email.search`
- Send/reply/outbox retry flows with deterministic provider stub for regression testing
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

## Operations docs

- [Operations Runbook](docs/operations_runbook.md)
- [Performance & Capacity](docs/performance_capacity.md)
