use rusqlite::{params, OptionalExtension};
use thiserror::Error;

use super::{
    Account, EmailStore, Folder, Message, NewAccount, NewFolder, NewMessage, NewOutboxMessage,
    OutboxMessage, SyncState, UpdateOutboxStatus, UpsertSyncState,
};

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct EmailRepository<'a> {
    store: &'a EmailStore,
}

impl<'a> EmailRepository<'a> {
    pub fn new(store: &'a EmailStore) -> Self {
        Self { store }
    }

    pub fn create_account(&self, input: &NewAccount) -> Result<i64, RepoError> {
        self.store.conn().execute(
            "INSERT INTO accounts (email, display_name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![input.email, input.display_name, input.now_ts],
        )?;
        Ok(self.store.conn().last_insert_rowid())
    }

    pub fn create_folder(&self, input: &NewFolder) -> Result<i64, RepoError> {
        self.store.conn().execute(
            "INSERT INTO folders (account_id, name, path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![input.account_id, input.name, input.path, input.now_ts],
        )?;
        Ok(self.store.conn().last_insert_rowid())
    }

    pub fn upsert_message(&self, input: &NewMessage) -> Result<i64, RepoError> {
        self.store.conn().execute(
            "INSERT INTO messages (account_id, folder_id, message_id, subject, sender, recipients, snippet, body_text, received_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(account_id, message_id) DO UPDATE SET
                folder_id=excluded.folder_id,
                subject=excluded.subject,
                sender=excluded.sender,
                recipients=excluded.recipients,
                snippet=excluded.snippet,
                body_text=excluded.body_text,
                received_at=excluded.received_at,
                updated_at=excluded.updated_at",
            params![
                input.account_id,
                input.folder_id,
                input.message_id,
                input.subject,
                input.sender,
                input.recipients,
                input.snippet,
                input.body_text,
                input.received_at,
                input.now_ts
            ],
        )?;

        let id: i64 = self.store.conn().query_row(
            "SELECT id FROM messages WHERE account_id = ?1 AND message_id = ?2",
            params![input.account_id, input.message_id],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn list_messages(&self, account_id: i64, limit: i64) -> Result<Vec<Message>, RepoError> {
        let mut stmt = self.store.conn().prepare(
            "SELECT id, account_id, folder_id, message_id, subject, sender, recipients, snippet, body_text, received_at, created_at, updated_at
             FROM messages WHERE account_id = ?1 ORDER BY received_at DESC, id DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![account_id, limit], map_message)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_message(
        &self,
        account_id: i64,
        message_id: &str,
    ) -> Result<Option<Message>, RepoError> {
        let result = self
            .store
            .conn()
            .query_row(
                "SELECT id, account_id, folder_id, message_id, subject, sender, recipients, snippet, body_text, received_at, created_at, updated_at
                 FROM messages WHERE account_id = ?1 AND message_id = ?2",
                params![account_id, message_id],
                map_message,
            )
            .optional()?;
        Ok(result)
    }

    pub fn search_messages(
        &self,
        account_id: i64,
        query: &str,
        limit: i64,
    ) -> Result<Vec<Message>, RepoError> {
        let like = format!("%{}%", query);
        let mut stmt = self.store.conn().prepare(
            "SELECT id, account_id, folder_id, message_id, subject, sender, recipients, snippet, body_text, received_at, created_at, updated_at
             FROM messages
             WHERE account_id = ?1 AND (subject LIKE ?2 OR sender LIKE ?2 OR snippet LIKE ?2)
             ORDER BY received_at DESC, id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, like, limit], map_message)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn upsert_sync_state(&self, input: &UpsertSyncState) -> Result<i64, RepoError> {
        self.store.conn().execute(
            "INSERT INTO sync_state (account_id, folder_id, cursor, last_synced_at, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id, folder_id) DO UPDATE SET
                cursor=excluded.cursor,
                last_synced_at=excluded.last_synced_at,
                status=excluded.status,
                updated_at=excluded.updated_at",
            params![
                input.account_id,
                input.folder_id,
                input.cursor,
                input.last_synced_at,
                input.status,
                input.now_ts
            ],
        )?;

        let id: i64 = self.store.conn().query_row(
            "SELECT id FROM sync_state WHERE account_id = ?1 AND folder_id IS ?2",
            params![input.account_id, input.folder_id],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn get_sync_state(
        &self,
        account_id: i64,
        folder_id: Option<i64>,
    ) -> Result<Option<SyncState>, RepoError> {
        let result = self
            .store
            .conn()
            .query_row(
                "SELECT id, account_id, folder_id, cursor, last_synced_at, status, updated_at
                 FROM sync_state WHERE account_id = ?1 AND folder_id IS ?2",
                params![account_id, folder_id],
                |r| {
                    Ok(SyncState {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        folder_id: r.get(2)?,
                        cursor: r.get(3)?,
                        last_synced_at: r.get(4)?,
                        status: r.get(5)?,
                        updated_at: r.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    pub fn get_account(&self, account_id: i64) -> Result<Option<Account>, RepoError> {
        let result = self
            .store
            .conn()
            .query_row(
                "SELECT id, email, display_name, created_at, updated_at FROM accounts WHERE id = ?1",
                params![account_id],
                |r| {
                    Ok(Account {
                        id: r.get(0)?,
                        email: r.get(1)?,
                        display_name: r.get(2)?,
                        created_at: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    pub fn list_folders(&self, account_id: i64) -> Result<Vec<Folder>, RepoError> {
        let mut stmt = self.store.conn().prepare(
            "SELECT id, account_id, name, path, created_at, updated_at
             FROM folders WHERE account_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![account_id], |r| {
            Ok(Folder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                name: r.get(2)?,
                path: r.get(3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_outbox_message(&self, input: &NewOutboxMessage) -> Result<i64, RepoError> {
        self.store.conn().execute(
            "INSERT INTO outbox (account_id, to_recipients, subject, body_text, in_reply_to_message_id, provider_message_id, status, retries, last_error, next_attempt_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                input.account_id,
                input.to_recipients,
                input.subject,
                input.body_text,
                input.in_reply_to_message_id,
                input.status,
                input.retries,
                input.last_error,
                input.next_attempt_at,
                input.now_ts,
            ],
        )?;
        Ok(self.store.conn().last_insert_rowid())
    }

    pub fn get_outbox_message(&self, outbox_id: i64) -> Result<Option<OutboxMessage>, RepoError> {
        let result = self
            .store
            .conn()
            .query_row(
                "SELECT id, account_id, to_recipients, subject, body_text, in_reply_to_message_id, provider_message_id, status, retries, last_error, next_attempt_at, created_at, updated_at
                 FROM outbox WHERE id = ?1",
                params![outbox_id],
                map_outbox,
            )
            .optional()?;
        Ok(result)
    }

    pub fn update_outbox_status(&self, input: &UpdateOutboxStatus) -> Result<(), RepoError> {
        self.store.conn().execute(
            "UPDATE outbox
             SET status = ?2,
                 retries = ?3,
                 last_error = ?4,
                 provider_message_id = ?5,
                 next_attempt_at = ?6,
                 updated_at = ?7
             WHERE id = ?1",
            params![
                input.id,
                input.status,
                input.retries,
                input.last_error,
                input.provider_message_id,
                input.next_attempt_at,
                input.now_ts,
            ],
        )?;
        Ok(())
    }
}

fn map_message(r: &rusqlite::Row<'_>) -> Result<Message, rusqlite::Error> {
    Ok(Message {
        id: r.get(0)?,
        account_id: r.get(1)?,
        folder_id: r.get(2)?,
        message_id: r.get(3)?,
        subject: r.get(4)?,
        sender: r.get(5)?,
        recipients: r.get(6)?,
        snippet: r.get(7)?,
        body_text: r.get(8)?,
        received_at: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn map_outbox(r: &rusqlite::Row<'_>) -> Result<OutboxMessage, rusqlite::Error> {
    Ok(OutboxMessage {
        id: r.get(0)?,
        account_id: r.get(1)?,
        to_recipients: r.get(2)?,
        subject: r.get(3)?,
        body_text: r.get(4)?,
        in_reply_to_message_id: r.get(5)?,
        provider_message_id: r.get(6)?,
        status: r.get(7)?,
        retries: r.get(8)?,
        last_error: r.get(9)?,
        next_attempt_at: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}
