use serde::{Deserialize, Serialize};

use crate::db::{
    EmailRepository, Message, NewMessage, NewOutboxMessage, OutboxMessage, UpdateOutboxStatus,
    UpsertSyncState,
};

const STATUS_PENDING: &str = "pending";
const STATUS_SENDING: &str = "sending";
const STATUS_SENT: &str = "sent";
const STATUS_FAILED: &str = "failed";
const BACKOFF_BASE_SECONDS: i64 = 5;

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

#[derive(Debug, Clone)]
pub struct SendEmailRequest {
    pub account_id: i64,
    pub to: String,
    pub subject: String,
    pub body_text: String,
    pub now_ts: i64,
    pub failure_mode: Option<SendFailureMode>,
}

#[derive(Debug, Clone)]
pub struct ReplyEmailRequest {
    pub account_id: i64,
    pub in_reply_to_message_id: String,
    pub body_text: String,
    pub now_ts: i64,
    pub failure_mode: Option<SendFailureMode>,
}

#[derive(Debug, Clone)]
pub struct RetryOutboxRequest {
    pub outbox_id: i64,
    pub now_ts: i64,
    pub failure_mode: Option<SendFailureMode>,
}

#[derive(Debug, Clone)]
pub enum SendFailureMode {
    Network,
    Provider,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    Validation,
    Network,
    Provider,
    Storage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<ApiError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendResult {
    pub outbox_id: i64,
    pub status: String,
    pub retries: i64,
    pub provider_message_id: Option<String>,
    pub next_attempt_at: i64,
}

/// M2 plugin skeleton with SQLite-backed operations and local provider stubs.
pub struct EmailPlugin<'a> {
    repo: EmailRepository<'a>,
}

impl<'a> EmailPlugin<'a> {
    pub fn new(repo: EmailRepository<'a>) -> Self {
        Self { repo }
    }

    /// email.sync
    pub fn sync(&self, req: SyncRequest) -> Result<(), ApiError> {
        self.repo
            .upsert_sync_state(&UpsertSyncState {
                account_id: req.account_id,
                folder_id: req.folder_id,
                cursor: req.cursor,
                last_synced_at: Some(req.now_ts),
                status: Some("ok".to_string()),
                now_ts: req.now_ts,
            })
            .map_err(storage_err)?;
        Ok(())
    }

    /// Helper used by tests / future sync ingestion.
    pub fn ingest_message(&self, msg: NewMessage) -> Result<i64, ApiError> {
        self.repo.upsert_message(&msg).map_err(storage_err)
    }

    /// email.list
    pub fn list(&self, req: ListMessagesRequest) -> Result<Vec<Message>, ApiError> {
        self.repo
            .list_messages(req.account_id, req.limit)
            .map_err(storage_err)
    }

    /// email.get
    pub fn get(&self, req: GetMessageRequest) -> Result<Option<Message>, ApiError> {
        self.repo
            .get_message(req.account_id, &req.message_id)
            .map_err(storage_err)
    }

    /// email.search
    pub fn search(&self, req: SearchMessagesRequest) -> Result<Vec<Message>, ApiError> {
        self.repo
            .search_messages(req.account_id, &req.query, req.limit)
            .map_err(storage_err)
    }

    /// email.send
    pub fn send(&self, req: SendEmailRequest) -> ApiResponse<SendResult> {
        if req.to.trim().is_empty()
            || req.subject.trim().is_empty()
            || req.body_text.trim().is_empty()
        {
            return fail(
                ErrorCode::Validation,
                "to/subject/body_text cannot be empty",
            );
        }

        let outbox_id = match self.repo.create_outbox_message(&NewOutboxMessage {
            account_id: req.account_id,
            to_recipients: req.to,
            subject: req.subject,
            body_text: req.body_text,
            in_reply_to_message_id: None,
            status: STATUS_PENDING.to_string(),
            retries: 0,
            last_error: None,
            next_attempt_at: req.now_ts,
            now_ts: req.now_ts,
        }) {
            Ok(id) => id,
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };

        self.deliver_outbox(outbox_id, req.now_ts, req.failure_mode)
    }

    /// email.reply
    pub fn reply(&self, req: ReplyEmailRequest) -> ApiResponse<SendResult> {
        if req.in_reply_to_message_id.trim().is_empty() || req.body_text.trim().is_empty() {
            return fail(
                ErrorCode::Validation,
                "in_reply_to_message_id/body_text cannot be empty",
            );
        }

        let parent = match self
            .repo
            .get_message(req.account_id, &req.in_reply_to_message_id)
        {
            Ok(m) => m,
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };
        let parent = match parent {
            Some(m) => m,
            None => {
                return fail(
                    ErrorCode::Validation,
                    "in_reply_to_message_id does not exist for this account",
                )
            }
        };

        let to = parent.sender.unwrap_or_default();
        if to.trim().is_empty() {
            return fail(
                ErrorCode::Validation,
                "cannot infer reply recipient from parent sender",
            );
        }

        let outbox_id = match self.repo.create_outbox_message(&NewOutboxMessage {
            account_id: req.account_id,
            to_recipients: to,
            subject: format!(
                "Re: {}",
                parent.subject.unwrap_or_else(|| "(no subject)".to_string())
            ),
            body_text: req.body_text,
            in_reply_to_message_id: Some(req.in_reply_to_message_id),
            status: STATUS_PENDING.to_string(),
            retries: 0,
            last_error: None,
            next_attempt_at: req.now_ts,
            now_ts: req.now_ts,
        }) {
            Ok(id) => id,
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };

        self.deliver_outbox(outbox_id, req.now_ts, req.failure_mode)
    }

    /// Manual retry trigger for failed/pending outbox records.
    pub fn retry_outbox(&self, req: RetryOutboxRequest) -> ApiResponse<SendResult> {
        self.deliver_outbox(req.outbox_id, req.now_ts, req.failure_mode)
    }

    pub fn get_outbox(&self, outbox_id: i64) -> Result<Option<OutboxMessage>, ApiError> {
        self.repo.get_outbox_message(outbox_id).map_err(storage_err)
    }

    fn deliver_outbox(
        &self,
        outbox_id: i64,
        now_ts: i64,
        failure_mode: Option<SendFailureMode>,
    ) -> ApiResponse<SendResult> {
        let outbox = match self.repo.get_outbox_message(outbox_id) {
            Ok(Some(v)) => v,
            Ok(None) => return fail(ErrorCode::Validation, "outbox record not found"),
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };

        if let Err(e) = self.repo.update_outbox_status(&UpdateOutboxStatus {
            id: outbox_id,
            status: STATUS_SENDING.to_string(),
            retries: outbox.retries,
            last_error: None,
            provider_message_id: outbox.provider_message_id.clone(),
            next_attempt_at: outbox.next_attempt_at,
            now_ts,
        }) {
            return fail(ErrorCode::Storage, &e.to_string());
        }

        match send_via_provider_stub(outbox_id, failure_mode) {
            Ok(provider_message_id) => {
                if let Err(e) = self.repo.update_outbox_status(&UpdateOutboxStatus {
                    id: outbox_id,
                    status: STATUS_SENT.to_string(),
                    retries: outbox.retries,
                    last_error: None,
                    provider_message_id: Some(provider_message_id.clone()),
                    next_attempt_at: now_ts,
                    now_ts,
                }) {
                    return fail(ErrorCode::Storage, &e.to_string());
                }

                ok(SendResult {
                    outbox_id,
                    status: STATUS_SENT.to_string(),
                    retries: outbox.retries,
                    provider_message_id: Some(provider_message_id),
                    next_attempt_at: now_ts,
                })
            }
            Err(provider_err) => {
                let next_retries = outbox.retries + 1;
                let backoff = BACKOFF_BASE_SECONDS * (1_i64 << std::cmp::min(next_retries, 10));
                let next_attempt_at = now_ts + backoff;
                let code = match provider_err.mode {
                    SendFailureMode::Network => ErrorCode::Network,
                    SendFailureMode::Provider => ErrorCode::Provider,
                };

                if let Err(e) = self.repo.update_outbox_status(&UpdateOutboxStatus {
                    id: outbox_id,
                    status: STATUS_FAILED.to_string(),
                    retries: next_retries,
                    last_error: Some(provider_err.message.clone()),
                    provider_message_id: None,
                    next_attempt_at,
                    now_ts,
                }) {
                    return fail(ErrorCode::Storage, &e.to_string());
                }

                ApiResponse {
                    ok: false,
                    data: Some(SendResult {
                        outbox_id,
                        status: STATUS_FAILED.to_string(),
                        retries: next_retries,
                        provider_message_id: None,
                        next_attempt_at,
                    }),
                    error: Some(ApiError {
                        code,
                        message: provider_err.message,
                    }),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderError {
    mode: SendFailureMode,
    message: String,
}

fn send_via_provider_stub(
    outbox_id: i64,
    failure_mode: Option<SendFailureMode>,
) -> Result<String, ProviderError> {
    if let Some(mode) = failure_mode {
        let message = match mode {
            SendFailureMode::Network => "simulated network timeout".to_string(),
            SendFailureMode::Provider => "simulated provider rejection".to_string(),
        };
        return Err(ProviderError { mode, message });
    }

    Ok(format!("stub-provider-{}", outbox_id))
}

fn ok<T>(data: T) -> ApiResponse<T> {
    ApiResponse {
        ok: true,
        data: Some(data),
        error: None,
    }
}

fn fail<T>(code: ErrorCode, message: &str) -> ApiResponse<T> {
    ApiResponse {
        ok: false,
        data: None,
        error: Some(ApiError {
            code,
            message: message.to_string(),
        }),
    }
}

fn storage_err(e: impl std::fmt::Display) -> ApiError {
    ApiError {
        code: ErrorCode::Storage,
        message: e.to_string(),
    }
}
