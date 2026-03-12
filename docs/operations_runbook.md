# PRX Email M4.1 Operations Runbook (SQLite-only)

## Scope

This runbook is for operating `prx_email` M4.2 in production with SQLite persistence.

## Configuration

`EmailStore` default runtime settings are tuned for production-safe SQLite behavior:

- `PRAGMA foreign_keys = ON`
- `PRAGMA journal_mode = WAL`
- `PRAGMA synchronous = NORMAL`
- `PRAGMA busy_timeout = 5000ms`
- `PRAGMA wal_autocheckpoint = 1000`

If needed, create the store with explicit options via `StoreConfig`:

- `enable_wal`
- `busy_timeout_ms`
- `wal_autocheckpoint_pages`
- `synchronous` (`Full`, `Normal`, `Off`)

Recommended defaults for production are the current defaults in `StoreConfig::default()`.

## M4.1 Operational Notes

## M4.2 Auth & Security Closure

### Unified auth config (IMAP + SMTP)

Both transports use the same auth schema:

- `auth.password`
- `auth.oauth_token`

Exactly one must be set for each protocol.

### Startup validation

`EmailPlugin::new_with_config` and runtime sync/send paths validate transport config before network calls:

- host/user required
- exactly one auth method required

Validation errors are human-readable and do not include raw credentials/tokens.

### Security logging

Failure logs now redact recipient mailbox in debug lines and never log raw `password`/`oauth_token`.

### Error grading

Provider/network errors are split into:

- user-facing message (safe, concise)
- debug detail (captured in server logs for tracing)


### Reply threading (`References`)

- `reply` now sends both:
  - `In-Reply-To: <parent-message-id>`
  - `References: <existing-parent-references...> <parent-message-id>`
- This improves thread stitching across Gmail/Outlook/IMAP clients.

### Multi-folder sync

- `email.sync` now accepts a folder path (for example `INBOX`, `Sent`).
- Sync cursor is tracked per `(account_id, folder_id)` in `sync_state`.
- If request cursor is omitted, last folder cursor is reused automatically.

### Attachment local persistence (optional)

- Configure attachment persistence in transport config:
  - `attachment_store.enabled = true`
  - `attachment_store.dir = /path/to/attachment-cache`
- On inbound sync, each attachment metadata includes `local_path` when write succeeds.
- Suggested ops policy:
  - keep directory on local encrypted disk
  - schedule periodic cleanup (age/size caps)
  - back up DB metadata and files together for consistency

## Migration Procedure

1. Stop write traffic or place the service in maintenance mode.
2. Back up the database (see Backup section).
3. Start the updated service; `EmailStore::migrate()` is idempotent and safe to re-run.
4. Verify feature-flag seed data exists:
   - `inbox_read` (default enabled)
   - `inbox_search` (default enabled)
   - `email_send` (default disabled)
   - `email_reply` (default disabled)
   - `outbox_retry` (default disabled)

## Staged Rollout Strategy

High-risk features are disabled by default and must be explicitly enabled.

### Stage 0: Read-only

- Keep defaults unchanged.
- Inbox APIs remain enabled; outbound APIs stay blocked.

### Stage 1: Internal canary

- Enable account-level overrides only for internal accounts:
  - `set_account_feature(account_id, "email_send", true, now_ts)`
  - `set_account_feature(account_id, "email_reply", true, now_ts)`
  - `set_account_feature(account_id, "outbox_retry", true, now_ts)`

### Stage 2: Percentage rollout

- Use deterministic account bucketing:
  - `apply_percentage_rollout(account_id, feature, percentage, now_ts)`
- Increase in small steps (for example 5% -> 25% -> 50% -> 100%).

### Stage 3: General availability

- Flip global defaults after confidence is high:
  - `set_feature_default(feature, true, now_ts)`
- Optional: remove account-level overrides where no longer needed.

## SQLite Backup / Restore

### Online backup (preferred)

Use SQLite backup command against a live DB with WAL checkpoint:

```bash
sqlite3 /path/to/prx_email.db "PRAGMA wal_checkpoint(FULL);" ".backup '/backup/prx_email_$(date +%F_%H%M%S).db'"
```

### File copy backup (maintenance window)

1. Stop writes.
2. Copy DB and sidecar files if they exist:

```bash
cp /path/to/prx_email.db /backup/
cp /path/to/prx_email.db-wal /backup/ 2>/dev/null || true
cp /path/to/prx_email.db-shm /backup/ 2>/dev/null || true
```

### Restore

1. Stop service.
2. Replace the DB with a backup.
3. Start service and run standard health checks.
4. Run `PRAGMA integrity_check;` post-restore.

## Troubleshooting

### `database is locked`

- Ensure only one writer process is active.
- Confirm `busy_timeout` is configured.
- Check for long-running write transactions.

### Unexpected outbound API failures (`FeatureDisabled`)

- Confirm feature defaults and per-account overrides.
- Verify account IDs are correct in rollout automation.

### Retry queue grows without recovery

- Inspect `outbox` statuses and `last_error`.
- Confirm `outbox_retry` is enabled for affected accounts.
- Verify retry caller passes a moving `now_ts` and handles backoff.

### Migration issues

- Re-run `migrate()`; migrations are idempotent.
- Validate schema objects in `sqlite_master` and flag rows in `feature_flags`.

## Health Checks

Run periodic queries:

- Outbox status counts:

```sql
SELECT status, COUNT(*) FROM outbox GROUP BY status;
```

- Old failed rows:

```sql
SELECT COUNT(*) FROM outbox WHERE status = 'failed' AND updated_at < strftime('%s','now') - 86400;
```

- Feature defaults:

```sql
SELECT key, default_enabled, risk_level FROM feature_flags ORDER BY key;
```
