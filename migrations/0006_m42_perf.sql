CREATE INDEX IF NOT EXISTS idx_messages_account_received_id
  ON messages(account_id, received_at DESC, id DESC);
