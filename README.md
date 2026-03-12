# prx_email

`prx_email` is the M1 (SQLite-only) foundation for an email plugin/service.

## Scope in this round

- Rust project bootstrap
- SQLite migration/schema setup
- Basic data access layer for:
  - `accounts`
  - `folders`
  - `messages`
  - `sync_state`
- Plugin interface skeleton for:
  - `email.sync`
  - `email.list`
  - `email.get`
  - `email.search`
- Minimal tests and green build

## Quick start

```bash
cargo test
cargo build
```

## Layout

- `migrations/0001_init.sql` — base schema
- `src/db/` — storage + repositories
- `src/plugin/` — plugin operation skeleton
