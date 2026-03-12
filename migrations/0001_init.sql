PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  email TEXT NOT NULL UNIQUE,
  display_name TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(account_id, path),
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL,
  folder_id INTEGER,
  message_id TEXT NOT NULL,
  subject TEXT,
  sender TEXT,
  recipients TEXT,
  snippet TEXT,
  body_text TEXT,
  body_html TEXT,
  attachments_json TEXT,
  received_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(account_id, message_id),
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,
  FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL,
  folder_id INTEGER,
  cursor TEXT,
  last_synced_at INTEGER,
  status TEXT,
  updated_at INTEGER NOT NULL,
  UNIQUE(account_id, folder_id),
  FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,
  FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_folders_account_id ON folders(account_id);
CREATE INDEX IF NOT EXISTS idx_messages_account_id ON messages(account_id);
CREATE INDEX IF NOT EXISTS idx_messages_folder_id ON messages(folder_id);
CREATE INDEX IF NOT EXISTS idx_messages_subject ON messages(subject);
CREATE INDEX IF NOT EXISTS idx_sync_state_account_folder ON sync_state(account_id, folder_id);
