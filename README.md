# prx_email

`prx_email` is a Rust-based email plugin foundation for PRX.
Current implementation is **SQLite-only** and focuses on core local data capabilities.

## What it does now

- Creates local email storage via SQLite migrations
- Provides basic data models and repositories for:
  - `accounts`
  - `folders`
  - `messages`
  - `sync_state`
- Exposes plugin operation skeletons for:
  - `email.sync`
  - `email.list`
  - `email.get`
  - `email.search`
- Includes smoke tests to verify schema/repository/plugin flow compiles and runs

## Quick start

```bash
# run tests
cargo test

# build
cargo build
```

## Project structure

- `migrations/0001_init.sql` — initial SQLite schema
- `src/db/` — models, storage, repositories
- `src/plugin/` — email operation skeletons
- `tests/m1_smoke.rs` — basic smoke test

## Current status

This repository currently provides the M1 foundation and local persistence layer.
Sending/receiving real email transport integration is not enabled yet.
