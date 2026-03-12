CREATE TABLE IF NOT EXISTS feature_flags (
  key TEXT PRIMARY KEY,
  description TEXT NOT NULL,
  default_enabled INTEGER NOT NULL CHECK (default_enabled IN (0, 1)),
  risk_level TEXT NOT NULL CHECK (risk_level IN ('low', 'medium', 'high')),
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS account_feature_flags (
  account_id INTEGER NOT NULL,
  feature_key TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (account_id, feature_key),
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,
  FOREIGN KEY(feature_key) REFERENCES feature_flags(key) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_account_feature_flags_account
  ON account_feature_flags(account_id);

INSERT OR IGNORE INTO feature_flags (key, description, default_enabled, risk_level, updated_at) VALUES
  ('inbox_read', 'Controls inbox read APIs (list/get).', 1, 'low', 0),
  ('inbox_search', 'Controls inbox search API.', 1, 'low', 0),
  ('email_send', 'Controls outbound send API.', 0, 'high', 0),
  ('email_reply', 'Controls outbound reply API.', 0, 'high', 0),
  ('outbox_retry', 'Controls manual outbox retry API.', 0, 'high', 0);
