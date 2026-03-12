# PRX Email M3 Performance and Capacity Guidance (SQLite-only)

## SQLite runtime settings

Use WAL mode and bounded checkpointing for mixed read/write workloads.

Recommended baseline (matches default `StoreConfig`):

- `journal_mode=WAL`
- `synchronous=NORMAL`
- `busy_timeout=5000`
- `wal_autocheckpoint=1000`

When durability requirements are strict and write latency is acceptable, use `synchronous=FULL`.

## Indexing guidance

Current schema includes critical indexes:

- `messages(account_id)` and `messages(folder_id)`
- `messages(subject)` for broad LIKE filter support
- `outbox(account_id)` and `outbox(status, next_attempt_at)`
- `sync_state(account_id, folder_id)`
- `account_feature_flags(account_id)`

Operational checks:

1. Run `EXPLAIN QUERY PLAN` for slow inbox/search/retry queries.
2. Add targeted indexes only after query-plan confirmation.
3. Avoid index bloat from speculative indexes.

## Capacity planning

Key growth drivers:

- `messages`: dominant table for inbox retention.
- `outbox`: accumulates sent + failed history.
- WAL file size: rises with write bursts until checkpoint.

Suggested thresholds:

1. Track DB file size and WAL size independently.
2. Alert when WAL remains large over multiple checkpoints.
3. Alert when outbox failed backlog exceeds operational SLO.

## Data cleanup / retention

Repository helpers are available for routine cleanup:

- `delete_sent_outbox_before(cutoff_ts)`
- `delete_old_messages_before(cutoff_ts)`

Suggested jobs:

1. Daily: delete old `sent` outbox rows beyond retention.
2. Weekly: delete old message rows per policy.
3. After large deletes: run `VACUUM` in a maintenance window if file compaction is needed.

## Maintenance SQL snippets

Outbox retry pressure:

```sql
SELECT status, COUNT(*) FROM outbox GROUP BY status;
```

Largest message-age buckets:

```sql
SELECT
  CASE
    WHEN received_at >= strftime('%s','now') - 86400 THEN 'lt_1d'
    WHEN received_at >= strftime('%s','now') - 604800 THEN 'lt_7d'
    ELSE 'ge_7d'
  END AS age_bucket,
  COUNT(*)
FROM messages
GROUP BY age_bucket;
```

WAL checkpoint and size stabilization:

```sql
PRAGMA wal_checkpoint(TRUNCATE);
```
