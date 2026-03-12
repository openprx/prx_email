use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub display_name: Option<String>,
    pub now_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewFolder {
    pub account_id: i64,
    pub name: String,
    pub path: String,
    pub now_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub account_id: i64,
    pub folder_id: Option<i64>,
    pub message_id: String,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub recipients: Option<String>,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub received_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub account_id: i64,
    pub folder_id: Option<i64>,
    pub message_id: String,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub recipients: Option<String>,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub received_at: Option<i64>,
    pub now_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub id: i64,
    pub account_id: i64,
    pub folder_id: Option<i64>,
    pub cursor: Option<String>,
    pub last_synced_at: Option<i64>,
    pub status: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct UpsertSyncState {
    pub account_id: i64,
    pub folder_id: Option<i64>,
    pub cursor: Option<String>,
    pub last_synced_at: Option<i64>,
    pub status: Option<String>,
    pub now_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboxMessage {
    pub id: i64,
    pub account_id: i64,
    pub to_recipients: String,
    pub subject: String,
    pub body_text: String,
    pub in_reply_to_message_id: Option<String>,
    pub provider_message_id: Option<String>,
    pub status: String,
    pub retries: i64,
    pub last_error: Option<String>,
    pub next_attempt_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewOutboxMessage {
    pub account_id: i64,
    pub to_recipients: String,
    pub subject: String,
    pub body_text: String,
    pub in_reply_to_message_id: Option<String>,
    pub status: String,
    pub retries: i64,
    pub last_error: Option<String>,
    pub next_attempt_at: i64,
    pub now_ts: i64,
}

#[derive(Debug, Clone)]
pub struct UpdateOutboxStatus {
    pub id: i64,
    pub status: String,
    pub retries: i64,
    pub last_error: Option<String>,
    pub provider_message_id: Option<String>,
    pub next_attempt_at: i64,
    pub now_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureFlag {
    pub key: String,
    pub description: String,
    pub default_enabled: bool,
    pub risk_level: String,
    pub updated_at: i64,
}
