CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL,
  to_recipients TEXT NOT NULL,
  subject TEXT NOT NULL,
  body_text TEXT NOT NULL,
  in_reply_to_message_id TEXT,
  provider_message_id TEXT,
  status TEXT NOT NULL CHECK(status IN ('pending', 'sending', 'sent', 'failed')),
  retries INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  next_attempt_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_outbox_account_id ON outbox(account_id);
CREATE INDEX IF NOT EXISTS idx_outbox_status_next_attempt ON outbox(status, next_attempt_at);
