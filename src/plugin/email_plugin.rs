use thiserror::Error;

use crate::db::{EmailRepository, Message, NewMessage, UpsertSyncState};

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub account_id: i64,
    pub folder_id: Option<i64>,
    pub cursor: Option<String>,
    pub now_ts: i64,
}

#[derive(Debug, Clone)]
pub struct ListMessagesRequest {
    pub account_id: i64,
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct GetMessageRequest {
    pub account_id: i64,
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct SearchMessagesRequest {
    pub account_id: i64,
    pub query: String,
    pub limit: i64,
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("repo error: {0}")]
    Repo(String),
}

/// M1 plugin skeleton with SQLite-backed operations.
pub struct EmailPlugin<'a> {
    repo: EmailRepository<'a>,
}

impl<'a> EmailPlugin<'a> {
    pub fn new(repo: EmailRepository<'a>) -> Self {
        Self { repo }
    }

    /// email.sync
    pub fn sync(&self, req: SyncRequest) -> Result<(), PluginError> {
        self.repo
            .upsert_sync_state(&UpsertSyncState {
                account_id: req.account_id,
                folder_id: req.folder_id,
                cursor: req.cursor,
                last_synced_at: Some(req.now_ts),
                status: Some("ok".to_string()),
                now_ts: req.now_ts,
            })
            .map_err(|e| PluginError::Repo(e.to_string()))?;
        Ok(())
    }

    /// Helper used by tests / future sync ingestion.
    pub fn ingest_message(&self, msg: NewMessage) -> Result<i64, PluginError> {
        self.repo
            .upsert_message(&msg)
            .map_err(|e| PluginError::Repo(e.to_string()))
    }

    /// email.list
    pub fn list(&self, req: ListMessagesRequest) -> Result<Vec<Message>, PluginError> {
        self.repo
            .list_messages(req.account_id, req.limit)
            .map_err(|e| PluginError::Repo(e.to_string()))
    }

    /// email.get
    pub fn get(&self, req: GetMessageRequest) -> Result<Option<Message>, PluginError> {
        self.repo
            .get_message(req.account_id, &req.message_id)
            .map_err(|e| PluginError::Repo(e.to_string()))
    }

    /// email.search
    pub fn search(&self, req: SearchMessagesRequest) -> Result<Vec<Message>, PluginError> {
        self.repo
            .search_messages(req.account_id, &req.query, req.limit)
            .map_err(|e| PluginError::Repo(e.to_string()))
    }
}
