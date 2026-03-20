use base64::Engine;
use imap::Authenticator;
use imap::types::Fetch;
use lettre::message::{
    Attachment, Mailbox, Message as SmtpMessage, MultiPart, SinglePart, header::ContentType,
};
use lettre::{
    SmtpTransport, Transport,
    transport::smtp::authentication::{Credentials, Mechanism},
};
use mail_parser::{Address, MessageParser, MimeHeaders};
use rustls_connector::RustlsConnector;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use crate::db::{
    AttachmentMeta, EmailRepository, Message, NewMessage, NewOutboxMessage, OutboxMessage,
    UpdateOutboxStatus, UpsertSyncState,
};

const STATUS_PENDING: &str = "pending";
const STATUS_SENDING: &str = "sending";
const STATUS_SENT: &str = "sent";
const STATUS_FAILED: &str = "failed";
const BACKOFF_BASE_SECONDS: i64 = 5;
const FEATURE_INBOX_READ: &str = "inbox_read";
const FEATURE_INBOX_SEARCH: &str = "inbox_search";
const FEATURE_EMAIL_SEND: &str = "email_send";
const FEATURE_EMAIL_REPLY: &str = "email_reply";
const FEATURE_OUTBOX_RETRY: &str = "outbox_retry";
const MIN_LIST_LIMIT: i64 = 1;
const MAX_LIST_LIMIT: i64 = 500;
const MAX_DEBUG_MESSAGE_LEN: usize = 160;

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub password: Option<String>,
    pub oauth_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone)]
pub struct AttachmentStoreConfig {
    pub enabled: bool,
    pub dir: String,
}

#[derive(Debug, Clone)]
pub struct EmailTransportConfig {
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
    pub attachment_store: Option<AttachmentStoreConfig>,
    pub attachment_policy: AttachmentPolicy,
}

#[derive(Debug, Clone)]
pub struct AttachmentPolicy {
    pub max_size_bytes: usize,
    pub allowed_content_types: HashSet<String>,
}

impl Default for AttachmentPolicy {
    fn default() -> Self {
        let allowed_content_types = [
            "application/pdf",
            "image/jpeg",
            "image/png",
            "text/plain",
            "application/zip",
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect();
        Self {
            max_size_bytes: 25 * 1024 * 1024,
            allowed_content_types,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeMetrics {
    pub sync_attempts: u64,
    pub sync_success: u64,
    pub sync_failures: u64,
    pub send_failures: u64,
    pub retry_count: u64,
}

#[derive(Debug, Clone)]
pub struct SyncJob {
    pub account_id: i64,
    pub folder: String,
    pub max_messages: usize,
}

#[derive(Debug, Clone)]
pub struct SyncRunnerConfig {
    pub max_concurrency: usize,
    pub base_backoff_seconds: i64,
    pub max_backoff_seconds: i64,
}

impl Default for SyncRunnerConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            base_backoff_seconds: 10,
            max_backoff_seconds: 300,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncRunnerReport {
    pub run_id: String,
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Clone)]
pub struct RefreshedOAuthToken {
    pub token: String,
    pub expires_at: Option<i64>,
}

pub trait OAuthRefreshProvider {
    fn refresh_token(
        &self,
        protocol: &str,
        user: &str,
        current_token: &str,
    ) -> Result<RefreshedOAuthToken, ApiError>;
}

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub account_id: i64,
    pub folder: Option<String>,
    pub cursor: Option<String>,
    pub now_ts: i64,
    pub max_messages: usize,
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
pub struct AttachmentInput {
    pub filename: String,
    pub content_type: String,
    pub base64: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SendEmailRequest {
    pub account_id: i64,
    pub to: String,
    pub subject: String,
    pub body_text: String,
    pub now_ts: i64,
    pub attachment: Option<AttachmentInput>,
    pub failure_mode: Option<SendFailureMode>,
}

#[derive(Debug, Clone)]
pub struct ReplyEmailRequest {
    pub account_id: i64,
    pub in_reply_to_message_id: String,
    pub body_text: String,
    pub now_ts: i64,
    pub attachment: Option<AttachmentInput>,
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
    FeatureDisabled,
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

pub struct EmailPlugin<'a> {
    repo: EmailRepository<'a>,
    config: RefCell<Option<EmailTransportConfig>>,
    metrics: RefCell<RuntimeMetrics>,
    scheduler_state: RefCell<HashMap<String, (i64, u32)>>,
    oauth_expiry_ts: RefCell<HashMap<String, i64>>,
    refresh_provider: Option<Box<dyn OAuthRefreshProvider>>,
}

impl<'a> EmailPlugin<'a> {
    pub fn new(repo: EmailRepository<'a>) -> Self {
        Self {
            repo,
            config: RefCell::new(None),
            metrics: RefCell::new(RuntimeMetrics::default()),
            scheduler_state: RefCell::new(HashMap::new()),
            oauth_expiry_ts: RefCell::new(HashMap::new()),
            refresh_provider: None,
        }
    }

    pub fn new_with_config(repo: EmailRepository<'a>, mut config: EmailTransportConfig) -> Self {
        if config.attachment_policy.allowed_content_types.is_empty() {
            config.attachment_policy = AttachmentPolicy::default();
        }
        if let Err(err) = validate_transport_config(&config) {
            log_debug("transport config validation failed", &err.message);
        }
        Self {
            repo,
            config: RefCell::new(Some(config)),
            metrics: RefCell::new(RuntimeMetrics::default()),
            scheduler_state: RefCell::new(HashMap::new()),
            oauth_expiry_ts: RefCell::new(HashMap::new()),
            refresh_provider: None,
        }
    }

    pub fn with_refresh_provider(mut self, provider: Box<dyn OAuthRefreshProvider>) -> Self {
        self.refresh_provider = Some(provider);
        self
    }

    pub fn reload_config(&self, mut config: EmailTransportConfig) -> Result<(), ApiError> {
        if config.attachment_policy.allowed_content_types.is_empty() {
            config.attachment_policy = AttachmentPolicy::default();
        }
        validate_transport_config(&config)?;
        *self.config.borrow_mut() = Some(config);
        Ok(())
    }

    pub fn reload_auth_from_env(&self, prefix: &str) {
        let mut cfg_guard = self.config.borrow_mut();
        let Some(cfg) = cfg_guard.as_mut() else {
            return;
        };

        let imap_key = format!("{}_IMAP_OAUTH_TOKEN", prefix);
        if let Ok(v) = std::env::var(&imap_key) {
            cfg.imap.auth.oauth_token = Some(v);
            cfg.imap.auth.password = None;
        }
        let smtp_key = format!("{}_SMTP_OAUTH_TOKEN", prefix);
        if let Ok(v) = std::env::var(&smtp_key) {
            cfg.smtp.auth.oauth_token = Some(v);
            cfg.smtp.auth.password = None;
        }
        if let Ok(v) = std::env::var(format!("{}_IMAP_OAUTH_EXPIRES_AT", prefix))
            && let Ok(ts) = v.parse::<i64>()
        {
            self.oauth_expiry_ts
                .borrow_mut()
                .insert("imap".to_string(), ts);
        }
        if let Ok(v) = std::env::var(format!("{}_SMTP_OAUTH_EXPIRES_AT", prefix))
            && let Ok(ts) = v.parse::<i64>()
        {
            self.oauth_expiry_ts
                .borrow_mut()
                .insert("smtp".to_string(), ts);
        }
    }

    pub fn metrics_snapshot(&self) -> RuntimeMetrics {
        self.metrics.borrow().clone()
    }

    pub fn run_sync_runner(
        &self,
        jobs: &[SyncJob],
        now_ts: i64,
        runner_cfg: &SyncRunnerConfig,
    ) -> SyncRunnerReport {
        let run_id = format!("run-{}", now_ts);
        let mut attempted = 0usize;
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        let max_concurrency = runner_cfg.max_concurrency.max(1);
        let mut due_jobs = Vec::new();
        for job in jobs {
            let key = format!("{}::{}", job.account_id, job.folder);
            let (next_allowed_at, fails) = self
                .scheduler_state
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or((0, 0));
            if now_ts < next_allowed_at {
                continue;
            }
            due_jobs.push((job, key, fails));
        }

        for (job, key, fails) in due_jobs.into_iter().take(max_concurrency) {
            attempted += 1;
            let result = self.sync(SyncRequest {
                account_id: job.account_id,
                folder: Some(job.folder.clone()),
                cursor: None,
                now_ts,
                max_messages: job.max_messages,
            });
            match result {
                Ok(_) => {
                    succeeded += 1;
                    self.scheduler_state
                        .borrow_mut()
                        .insert(key.clone(), (now_ts, 0));
                    log_structured(
                        "sync_runner_ok",
                        job.account_id,
                        Some(&job.folder),
                        None,
                        &run_id,
                        None,
                    );
                }
                Err(err) => {
                    failed += 1;
                    self.metrics.borrow_mut().sync_failures += 1;
                    let next_fails = fails + 1;
                    let exp = i64::pow(2, next_fails.min(6));
                    let backoff =
                        (runner_cfg.base_backoff_seconds * exp).min(runner_cfg.max_backoff_seconds);
                    self.scheduler_state
                        .borrow_mut()
                        .insert(key.clone(), (now_ts + backoff, next_fails));
                    log_structured(
                        "sync_runner_failed",
                        job.account_id,
                        Some(&job.folder),
                        None,
                        &run_id,
                        Some(err.code),
                    );
                }
            }
        }

        SyncRunnerReport {
            run_id,
            attempted,
            succeeded,
            failed,
        }
    }

    pub fn sync(&self, req: SyncRequest) -> Result<(), ApiError> {
        self.metrics.borrow_mut().sync_attempts += 1;
        self.ensure_oauth_fresh(req.now_ts)?;
        let cfg_guard = self.config.borrow();
        let cfg = cfg_guard.as_ref().ok_or_else(|| ApiError {
            code: ErrorCode::Validation,
            message: "imap/smtp config missing".to_string(),
        })?;
        validate_transport_config(cfg)?;

        let tls = RustlsConnector::default();
        let tcp =
            TcpStream::connect((cfg.imap.host.as_str(), cfg.imap.port)).map_err(network_err)?;
        let tls_stream = tls.connect(&cfg.imap.host, tcp).map_err(network_err)?;
        let client = imap::Client::new(tls_stream);
        let mut session = imap_login(client, &cfg.imap)?;
        let folder_path = req.folder.unwrap_or_else(|| "INBOX".to_string());
        session.select(&folder_path).map_err(network_err)?;

        let folder_id = self.ensure_folder(req.account_id, &folder_path, req.now_ts)?;
        let start_uid = req
            .cursor
            .as_deref()
            .and_then(|v| v.parse::<u32>().ok())
            .or_else(|| {
                self.repo
                    .get_sync_state(req.account_id, Some(folder_id))
                    .ok()
                    .flatten()
                    .and_then(|s| s.cursor)
                    .and_then(|v| v.parse::<u32>().ok())
            })
            .unwrap_or(0);
        let uid_criteria = format!("UID {}:*", start_uid.saturating_add(1));
        let mut uids: Vec<u32> = session
            .uid_search(uid_criteria)
            .map_err(network_err)?
            .into_iter()
            .collect();
        uids.sort_unstable();
        if uids.len() > req.max_messages {
            uids = uids[uids.len() - req.max_messages..].to_vec();
        }

        let mut max_uid = start_uid;
        for uid in uids {
            let seq = uid.to_string();
            let fetches = session
                .uid_fetch(seq.as_str(), "UID RFC822")
                .map_err(network_err)?;
            for fetch in fetches.iter() {
                if let Some(msg) = parse_imap_fetch(
                    req.account_id,
                    Some(folder_id),
                    req.now_ts,
                    fetch,
                    cfg.attachment_store.as_ref(),
                    &cfg.attachment_policy,
                ) {
                    self.repo.upsert_message(&msg).map_err(storage_err)?;
                }
                if let Some(found_uid) = fetch.uid {
                    max_uid = max_uid.max(found_uid);
                }
            }
        }

        session.logout().map_err(network_err)?;

        self.repo
            .upsert_sync_state(&UpsertSyncState {
                account_id: req.account_id,
                folder_id: Some(folder_id),
                cursor: Some(max_uid.to_string()),
                last_synced_at: Some(req.now_ts),
                status: Some("ok".to_string()),
                now_ts: req.now_ts,
            })
            .map_err(storage_err)?;
        self.metrics.borrow_mut().sync_success += 1;
        Ok(())
    }

    fn ensure_folder(
        &self,
        account_id: i64,
        folder_path: &str,
        now_ts: i64,
    ) -> Result<i64, ApiError> {
        if let Some(folder) = self
            .repo
            .get_folder_by_path(account_id, folder_path)
            .map_err(storage_err)?
        {
            return Ok(folder.id);
        }

        self.repo
            .create_folder(&crate::db::NewFolder {
                account_id,
                name: folder_path.to_string(),
                path: folder_path.to_string(),
                now_ts,
            })
            .map_err(storage_err)
    }

    pub fn ingest_message(&self, msg: NewMessage) -> Result<i64, ApiError> {
        self.repo.upsert_message(&msg).map_err(storage_err)
    }

    pub fn list(&self, req: ListMessagesRequest) -> Result<Vec<Message>, ApiError> {
        self.require_feature(req.account_id, FEATURE_INBOX_READ)?;
        let limit = sanitize_limit(req.limit)?;
        self.repo
            .list_messages(req.account_id, limit)
            .map_err(storage_err)
    }

    pub fn get(&self, req: GetMessageRequest) -> Result<Option<Message>, ApiError> {
        self.require_feature(req.account_id, FEATURE_INBOX_READ)?;
        self.repo
            .get_message(req.account_id, &req.message_id)
            .map_err(storage_err)
    }

    pub fn search(&self, req: SearchMessagesRequest) -> Result<Vec<Message>, ApiError> {
        self.require_feature(req.account_id, FEATURE_INBOX_SEARCH)?;
        let limit = sanitize_limit(req.limit)?;
        self.repo
            .search_messages(req.account_id, &req.query, limit)
            .map_err(storage_err)
    }

    pub fn send(&self, req: SendEmailRequest) -> ApiResponse<SendResult> {
        if let Err(e) = self.require_feature(req.account_id, FEATURE_EMAIL_SEND) {
            return ApiResponse {
                ok: false,
                data: None,
                error: Some(e),
            };
        }

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

        self.deliver_outbox(outbox_id, req.now_ts, req.attachment, req.failure_mode)
    }

    pub fn reply(&self, req: ReplyEmailRequest) -> ApiResponse<SendResult> {
        if let Err(e) = self.require_feature(req.account_id, FEATURE_EMAIL_REPLY) {
            return ApiResponse {
                ok: false,
                data: None,
                error: Some(e),
            };
        }

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
            Ok(Some(m)) => m,
            Ok(None) => {
                return fail(
                    ErrorCode::Validation,
                    "in_reply_to_message_id does not exist for this account",
                );
            }
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
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

        self.deliver_outbox(outbox_id, req.now_ts, req.attachment, req.failure_mode)
    }

    pub fn retry_outbox(&self, req: RetryOutboxRequest) -> ApiResponse<SendResult> {
        let outbox = match self.repo.get_outbox_message(req.outbox_id) {
            Ok(Some(outbox)) => outbox,
            Ok(None) => return fail(ErrorCode::Validation, "outbox record not found"),
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };
        if let Err(e) = self.require_feature(outbox.account_id, FEATURE_OUTBOX_RETRY) {
            return ApiResponse {
                ok: false,
                data: None,
                error: Some(e),
            };
        }
        if !matches!(outbox.status.as_str(), STATUS_PENDING | STATUS_FAILED) {
            return fail(ErrorCode::Validation, "retry_not_allowed_for_status");
        }
        if outbox.next_attempt_at > req.now_ts {
            return fail(ErrorCode::Validation, "retry_not_due");
        }

        self.deliver_outbox(req.outbox_id, req.now_ts, None, req.failure_mode)
    }

    pub fn get_outbox(&self, outbox_id: i64) -> Result<Option<OutboxMessage>, ApiError> {
        self.repo.get_outbox_message(outbox_id).map_err(storage_err)
    }

    pub fn set_feature_default(
        &self,
        feature_key: &str,
        enabled: bool,
        now_ts: i64,
    ) -> Result<(), ApiError> {
        self.repo
            .set_feature_default(feature_key, enabled, now_ts)
            .map_err(storage_err)
    }

    pub fn set_account_feature(
        &self,
        account_id: i64,
        feature_key: &str,
        enabled: bool,
        now_ts: i64,
    ) -> Result<(), ApiError> {
        self.repo
            .set_account_feature_flag(account_id, feature_key, enabled, now_ts)
            .map_err(storage_err)
    }

    pub fn apply_percentage_rollout(
        &self,
        account_id: i64,
        feature_key: &str,
        percentage: u8,
        now_ts: i64,
    ) -> Result<bool, ApiError> {
        let bounded = percentage.min(100);
        let bucket = account_id.rem_euclid(100) as u8;
        let enabled = bucket < bounded;
        self.set_account_feature(account_id, feature_key, enabled, now_ts)?;
        Ok(enabled)
    }

    pub fn is_feature_enabled(&self, account_id: i64, feature_key: &str) -> Result<bool, ApiError> {
        self.repo
            .is_feature_enabled(account_id, feature_key)
            .map_err(storage_err)?
            .ok_or_else(|| ApiError {
                code: ErrorCode::Validation,
                message: format!("unknown feature flag: {}", feature_key),
            })
    }

    fn deliver_outbox(
        &self,
        outbox_id: i64,
        now_ts: i64,
        attachment: Option<AttachmentInput>,
        failure_mode: Option<SendFailureMode>,
    ) -> ApiResponse<SendResult> {
        if let Err(e) = self.ensure_oauth_fresh(now_ts) {
            return ApiResponse {
                ok: false,
                data: None,
                error: Some(e),
            };
        }

        let claimed = match self.repo.claim_outbox_for_send(outbox_id, now_ts) {
            Ok(v) => v,
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };
        if !claimed {
            let latest = match self.repo.get_outbox_message(outbox_id) {
                Ok(Some(v)) => v,
                Ok(None) => return fail(ErrorCode::Validation, "outbox record not found"),
                Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
            };
            return fail(
                ErrorCode::Validation,
                &format!(
                    "outbox_not_claimable status={} next_attempt_at={}",
                    latest.status, latest.next_attempt_at
                ),
            );
        }

        let outbox = match self.repo.get_outbox_message(outbox_id) {
            Ok(Some(v)) => v,
            Ok(None) => return fail(ErrorCode::Validation, "outbox record not found"),
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };

        match self.send_via_provider(&outbox, attachment, failure_mode) {
            Ok(provider_message_id) => {
                let updated = self.repo.update_outbox_status_if_current(
                    &UpdateOutboxStatus {
                        id: outbox_id,
                        status: STATUS_SENT.to_string(),
                        retries: outbox.retries,
                        last_error: None,
                        provider_message_id: Some(provider_message_id.clone()),
                        next_attempt_at: now_ts,
                        now_ts,
                    },
                    STATUS_SENDING,
                );
                match updated {
                    Ok(true) => ok(SendResult {
                        outbox_id,
                        status: STATUS_SENT.to_string(),
                        retries: outbox.retries,
                        provider_message_id: Some(provider_message_id),
                        next_attempt_at: now_ts,
                    }),
                    Ok(false) => fail(
                        ErrorCode::Validation,
                        "outbox_state_changed_before_finalize",
                    ),
                    Err(e) => fail(ErrorCode::Storage, &e.to_string()),
                }
            }
            Err(provider_err) => {
                let next_retries = outbox.retries + 1;
                self.metrics.borrow_mut().send_failures += 1;
                self.metrics.borrow_mut().retry_count += 1;
                let backoff = BACKOFF_BASE_SECONDS * (1_i64 << std::cmp::min(next_retries, 10));
                let next_attempt_at = now_ts + backoff;
                let code = match provider_err.mode {
                    SendFailureMode::Network => ErrorCode::Network,
                    SendFailureMode::Provider => ErrorCode::Provider,
                };

                log_debug(
                    "outbox send failed",
                    &format!(
                        "account={} to={} err={} debug={}",
                        outbox.account_id,
                        redact_email(&outbox.to_recipients),
                        provider_err.message,
                        sanitize_debug_message(&provider_err.debug_message)
                    ),
                );
                log_structured(
                    "send_failed",
                    outbox.account_id,
                    None,
                    Some(&outbox.id.to_string()),
                    &format!("send-{}", now_ts),
                    Some(code.clone()),
                );
                let updated = self.repo.update_outbox_status_if_current(
                    &UpdateOutboxStatus {
                        id: outbox_id,
                        status: STATUS_FAILED.to_string(),
                        retries: next_retries,
                        last_error: Some(provider_err.message.clone()),
                        provider_message_id: None,
                        next_attempt_at,
                        now_ts,
                    },
                    STATUS_SENDING,
                );
                match updated {
                    Ok(true) => ApiResponse {
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
                    },
                    Ok(false) => fail(
                        ErrorCode::Validation,
                        "outbox_state_changed_before_finalize",
                    ),
                    Err(e) => fail(ErrorCode::Storage, &e.to_string()),
                }
            }
        }
    }

    fn send_via_provider(
        &self,
        outbox: &OutboxMessage,
        attachment: Option<AttachmentInput>,
        failure_mode: Option<SendFailureMode>,
    ) -> Result<String, ProviderError> {
        if let Some(mode) = failure_mode {
            let message = match mode {
                SendFailureMode::Network => "simulated network timeout".to_string(),
                SendFailureMode::Provider => "simulated provider rejection".to_string(),
            };
            return Err(ProviderError {
                mode,
                message,
                debug_message: "simulated failure".to_string(),
            });
        }

        let cfg_guard = self.config.borrow();
        let cfg = cfg_guard.as_ref().ok_or_else(|| ProviderError {
            mode: SendFailureMode::Provider,
            message: "smtp config missing".to_string(),
            debug_message: "smtp transport config is None".to_string(),
        })?;

        let from = Mailbox::new(
            None,
            cfg.smtp.user.parse().map_err(|e| ProviderError {
                mode: SendFailureMode::Provider,
                message: "invalid smtp sender address".to_string(),
                debug_message: format!("invalid smtp user address: {e}"),
            })?,
        );
        let to = Mailbox::new(
            None,
            outbox.to_recipients.parse().map_err(|e| ProviderError {
                mode: SendFailureMode::Provider,
                message: "invalid recipient address".to_string(),
                debug_message: format!("invalid recipient address: {e}"),
            })?,
        );

        let idempotency_key = build_outbox_idempotency_key(outbox);
        let mut msg_builder = SmtpMessage::builder()
            .from(from)
            .to(to)
            .subject(outbox.subject.clone())
            .message_id(Some(format!("<{idempotency_key}@prx-email.local>")));
        if let Some(in_reply_to) = &outbox.in_reply_to_message_id {
            msg_builder = msg_builder.in_reply_to(in_reply_to.clone());
            if let Ok(Some(parent)) = self.repo.get_message(outbox.account_id, in_reply_to) {
                let references =
                    build_references_chain(parent.references_header.as_deref(), &parent.message_id);
                if !references.is_empty() {
                    msg_builder = msg_builder.references(references);
                }
            }
        }

        let body = SinglePart::builder()
            .header(ContentType::TEXT_PLAIN)
            .body(outbox.body_text.clone());

        let message = if let Some(att) = attachment {
            let bytes = read_attachment_bytes(&att, cfg)?;
            let content_type = ContentType::parse(&att.content_type).map_err(provider_err)?;
            let part = Attachment::new(att.filename).body(bytes, content_type);
            msg_builder
                .multipart(MultiPart::mixed().singlepart(body).singlepart(part))
                .map_err(provider_err)?
        } else {
            msg_builder.singlepart(body).map_err(provider_err)?
        };

        validate_transport_config(cfg).map_err(|err| ProviderError {
            mode: SendFailureMode::Provider,
            message: err.message.clone(),
            debug_message: format!("transport config invalid: {}", err.message),
        })?;

        let mut builder = SmtpTransport::relay(&cfg.smtp.host)
            .map_err(provider_err)?
            .port(cfg.smtp.port);
        match (&cfg.smtp.auth.password, &cfg.smtp.auth.oauth_token) {
            (Some(password), None) => {
                builder =
                    builder.credentials(Credentials::new(cfg.smtp.user.clone(), password.clone()));
            }
            (None, Some(token)) => {
                builder = builder
                    .authentication(vec![Mechanism::Xoauth2])
                    .credentials(Credentials::new(cfg.smtp.user.clone(), token.clone()));
            }
            _ => {
                return Err(ProviderError {
                    mode: SendFailureMode::Provider,
                    message: "smtp auth must set exactly one of password/oauth_token".to_string(),
                    debug_message: "smtp auth config invalid".to_string(),
                });
            }
        }
        let mailer = builder.build();

        let response = mailer.send(&message).map_err(network_err_provider)?;
        let provider_id = response.message().collect::<Vec<_>>().join(" ");
        Ok(format!("smtp-{}", provider_id))
    }

    fn require_feature(&self, account_id: i64, feature_key: &str) -> Result<(), ApiError> {
        match self
            .repo
            .is_feature_enabled(account_id, feature_key)
            .map_err(storage_err)?
        {
            Some(true) => Ok(()),
            Some(false) => Err(ApiError {
                code: ErrorCode::FeatureDisabled,
                message: format!(
                    "feature '{}' is disabled for account {}",
                    feature_key, account_id
                ),
            }),
            None => Err(ApiError {
                code: ErrorCode::Validation,
                message: format!("unknown feature flag: {}", feature_key),
            }),
        }
    }

    fn ensure_oauth_fresh(&self, now_ts: i64) -> Result<(), ApiError> {
        let mut cfg_guard = self.config.borrow_mut();
        let Some(cfg) = cfg_guard.as_mut() else {
            return Ok(());
        };
        refresh_protocol_token(
            "imap",
            &cfg.imap.user,
            &mut cfg.imap.auth,
            now_ts,
            &self.oauth_expiry_ts,
            self.refresh_provider.as_deref(),
        )?;
        refresh_protocol_token(
            "smtp",
            &cfg.smtp.user,
            &mut cfg.smtp.auth,
            now_ts,
            &self.oauth_expiry_ts,
            self.refresh_provider.as_deref(),
        )?;
        Ok(())
    }
}

fn refresh_protocol_token(
    protocol: &str,
    user: &str,
    auth: &mut AuthConfig,
    now_ts: i64,
    expiry_state: &RefCell<HashMap<String, i64>>,
    provider: Option<&dyn OAuthRefreshProvider>,
) -> Result<(), ApiError> {
    let Some(token) = auth.oauth_token.clone() else {
        return Ok(());
    };
    let expires_at = expiry_state.borrow().get(protocol).cloned();
    let needs_refresh = expires_at.map(|ts| ts <= now_ts + 60).unwrap_or(false);
    if !needs_refresh {
        return Ok(());
    }

    let provider = provider.ok_or_else(|| ApiError {
        code: ErrorCode::Provider,
        message: format!("{protocol} oauth token expired and no refresh provider configured"),
    })?;
    let refreshed = provider.refresh_token(protocol, user, &token)?;
    auth.oauth_token = Some(refreshed.token);
    if let Some(ts) = refreshed.expires_at {
        expiry_state.borrow_mut().insert(protocol.to_string(), ts);
    }
    Ok(())
}

fn log_structured(
    event: &str,
    account_id: i64,
    folder: Option<&str>,
    message_id: Option<&str>,
    run_id: &str,
    error_code: Option<ErrorCode>,
) {
    let payload = serde_json::json!({
        "event": event,
        "account": account_id,
        "folder": folder,
        "message_id": message_id,
        "run_id": run_id,
        "error_code": error_code,
    });
    eprintln!("[prx_email][structured] {}", payload);
}

#[derive(Debug, Clone)]
struct ProviderError {
    mode: SendFailureMode,
    message: String,
    debug_message: String,
}

fn read_attachment_bytes(
    input: &AttachmentInput,
    cfg: &EmailTransportConfig,
) -> Result<Vec<u8>, ProviderError> {
    enforce_attachment_policy(&cfg.attachment_policy, &input.content_type, 0)?;
    let bytes = match (&input.base64, &input.path) {
        (Some(b64), None) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(provider_err)?,
        (None, Some(path)) => {
            if let Some(store) = cfg.attachment_store.as_ref() {
                let resolved = safe_resolve_under_root(Path::new(&store.dir), Path::new(path))?;
                fs::read(resolved).map_err(network_err_provider)?
            } else {
                fs::read(path).map_err(network_err_provider)?
            }
        }
        _ => {
            return Err(ProviderError {
                mode: SendFailureMode::Provider,
                message: "attachment requires exactly one of base64 or path".to_string(),
                debug_message: "attachment input has both/none of base64/path".to_string(),
            });
        }
    };
    enforce_attachment_policy(&cfg.attachment_policy, &input.content_type, bytes.len())?;
    Ok(bytes)
}

fn parse_imap_fetch(
    account_id: i64,
    folder_id: Option<i64>,
    now_ts: i64,
    fetch: &Fetch,
    attachment_store: Option<&AttachmentStoreConfig>,
    attachment_policy: &AttachmentPolicy,
) -> Option<NewMessage> {
    let raw = fetch.body()?;
    let parsed = parse_mime_message(raw, attachment_store, attachment_policy, account_id, now_ts)?;
    Some(NewMessage {
        account_id,
        folder_id,
        message_id: parsed.message_id,
        subject: parsed.subject,
        sender: parsed.sender,
        recipients: parsed.recipients,
        snippet: parsed.snippet,
        body_text: parsed.body_text,
        body_html: parsed.body_html,
        attachments_json: Some(serde_json::to_string(&parsed.attachments).ok()?),
        references_header: parsed.references_header,
        received_at: Some(now_ts),
        now_ts,
    })
}

#[derive(Debug, Clone)]
struct ParsedMime {
    message_id: String,
    subject: Option<String>,
    sender: Option<String>,
    recipients: Option<String>,
    snippet: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    references_header: Option<String>,
    attachments: Vec<AttachmentMeta>,
}

fn parse_mime_message(
    raw: &[u8],
    attachment_store: Option<&AttachmentStoreConfig>,
    attachment_policy: &AttachmentPolicy,
    account_id: i64,
    now_ts: i64,
) -> Option<ParsedMime> {
    let message = MessageParser::default().parse(raw)?;
    let body_text = message
        .text_bodies()
        .find_map(|p| p.text_contents().map(|v| v.to_string()))
        .or_else(|| message.body_text(0).map(|v| v.into_owned()));
    let body_html = message
        .html_bodies()
        .find_map(|p| p.text_contents().map(|v| v.to_string()))
        .or_else(|| message.body_html(0).map(|v| v.into_owned()))
        .or_else(|| extract_html_part_from_raw(raw));
    let snippet = body_text
        .as_ref()
        .map(|v| v.chars().take(120).collect::<String>())
        .or_else(|| {
            body_html
                .as_ref()
                .map(|v| v.chars().take(120).collect::<String>())
        });

    let message_id = message
        .message_id()
        .map(|v| v.to_string())
        .unwrap_or_else(|| fallback_message_id(raw, account_id, now_ts));

    let attachments = message
        .attachments()
        .enumerate()
        .map(|(idx, part)| {
            let filename = part.attachment_name().map(|n| n.to_string());
            let content_type = part.content_type().map(|c| {
                format!(
                    "{}/{}",
                    c.c_type,
                    c.c_subtype.clone().unwrap_or_else(|| "octet-stream".into())
                )
            });
            let local_path = persist_attachment(
                attachment_store,
                attachment_policy,
                account_id,
                &message_id,
                idx,
                filename.as_deref(),
                content_type.as_deref(),
                part.contents(),
                now_ts,
            );
            AttachmentMeta {
                filename,
                content_type,
                size: part.contents().len(),
                local_path,
            }
        })
        .collect::<Vec<_>>();

    Some(ParsedMime {
        message_id,
        subject: message.subject().map(|v| v.to_string()),
        sender: first_address(message.from()),
        recipients: flatten_addresses(message.to()),
        snippet,
        body_text,
        body_html,
        references_header: message.references().as_text().map(|v| v.to_string()),
        attachments,
    })
}

fn extract_html_part_from_raw(raw: &[u8]) -> Option<String> {
    let raw_text = std::str::from_utf8(raw).ok()?;
    let marker = "Content-Type: text/html";
    let section_start = raw_text.find(marker)?;
    let after_header = raw_text[section_start..].find("\r\n\r\n")? + section_start + 4;
    let remaining = &raw_text[after_header..];
    let end = remaining.find("\r\n--").unwrap_or(remaining.len());
    Some(remaining[..end].trim().to_string())
}

fn flatten_addresses(address: Option<&Address<'_>>) -> Option<String> {
    match address {
        Some(Address::List(items)) => {
            let values: Vec<String> = items
                .iter()
                .filter_map(|a| a.address.as_ref().map(|v| v.to_string()))
                .collect();
            if values.is_empty() {
                None
            } else {
                Some(values.join(", "))
            }
        }
        Some(Address::Group(groups)) => {
            let values: Vec<String> = groups
                .iter()
                .flat_map(|g| g.addresses.iter())
                .filter_map(|a| a.address.as_ref().map(|v| v.to_string()))
                .collect();
            if values.is_empty() {
                None
            } else {
                Some(values.join(", "))
            }
        }
        None => None,
    }
}

fn first_address(address: Option<&Address<'_>>) -> Option<String> {
    flatten_addresses(address).and_then(|v| v.split(',').next().map(|s| s.trim().to_string()))
}

#[allow(clippy::too_many_arguments)]
fn persist_attachment(
    attachment_store: Option<&AttachmentStoreConfig>,
    policy: &AttachmentPolicy,
    account_id: i64,
    message_id: &str,
    index: usize,
    filename: Option<&str>,
    content_type: Option<&str>,
    bytes: &[u8],
    now_ts: i64,
) -> Option<String> {
    let cfg = attachment_store?;
    if !cfg.enabled {
        return None;
    }

    enforce_attachment_policy(
        policy,
        content_type.unwrap_or("application/octet-stream"),
        bytes.len(),
    )
    .ok()?;

    let root = Path::new(&cfg.dir);
    let safe_message_id = sanitize_path_component(message_id);
    let safe_filename = sanitize_path_component(filename.unwrap_or("attachment.bin"));
    let account_dir = root.join(account_id.to_string()).join(safe_message_id);
    fs::create_dir_all(&account_dir).ok()?;
    let path: PathBuf = account_dir.join(format!("{}-{}-{}", now_ts, index, safe_filename));
    let safe_target = safe_resolve_under_root(root, &path).ok()?;
    fs::write(&safe_target, bytes).ok()?;
    Some(safe_target.to_string_lossy().to_string())
}

fn enforce_attachment_policy(
    policy: &AttachmentPolicy,
    content_type: &str,
    size: usize,
) -> Result<(), ProviderError> {
    if size > policy.max_size_bytes {
        return Err(ProviderError {
            mode: SendFailureMode::Provider,
            message: "attachment exceeds size limit".to_string(),
            debug_message: format!("size={} max={}", size, policy.max_size_bytes),
        });
    }
    if !policy.allowed_content_types.contains(content_type) {
        return Err(ProviderError {
            mode: SendFailureMode::Provider,
            message: "attachment content type is not allowed".to_string(),
            debug_message: format!("content_type={content_type}"),
        });
    }
    Ok(())
}

fn safe_resolve_under_root(root: &Path, candidate: &Path) -> Result<PathBuf, ProviderError> {
    let root_abs = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root_abs.join(candidate)
    };
    let normalized = normalize_path(&joined);
    if !normalized.starts_with(&root_abs) {
        return Err(ProviderError {
            mode: SendFailureMode::Provider,
            message: "attachment path escapes storage root".to_string(),
            debug_message: format!(
                "root={} candidate={}",
                root_abs.display(),
                normalized.display()
            ),
        });
    }
    Ok(normalized)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn sanitize_path_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

fn extract_message_ids(header_value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    for ch in header_value.chars() {
        if ch == '<' {
            in_token = true;
            current.clear();
            current.push(ch);
        } else if ch == '>' && in_token {
            current.push(ch);
            out.push(current.clone());
            in_token = false;
            current.clear();
        } else if in_token {
            current.push(ch);
        }
    }
    out
}

fn build_references_chain(existing_references: Option<&str>, parent_message_id: &str) -> String {
    let mut refs = existing_references
        .map(extract_message_ids)
        .unwrap_or_default();
    if !refs.iter().any(|v| v == parent_message_id) {
        refs.push(parent_message_id.to_string());
    }
    refs.join(" ")
}

fn sanitize_limit(limit: i64) -> Result<i64, ApiError> {
    if !(MIN_LIST_LIMIT..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(ApiError {
            code: ErrorCode::Validation,
            message: format!(
                "limit_out_of_range:{} (expected {}..={})",
                limit, MIN_LIST_LIMIT, MAX_LIST_LIMIT
            ),
        });
    }
    Ok(limit)
}

fn fallback_message_id(raw: &[u8], account_id: i64, now_ts: i64) -> String {
    let digest = Sha256::digest(raw);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("generated-{account_id}-{now_ts}-{hex}")
}

fn build_outbox_idempotency_key(outbox: &OutboxMessage) -> String {
    format!("outbox-{}-{}", outbox.id, outbox.retries)
}

fn sanitize_debug_message(raw: &str) -> String {
    let mut redacted = raw.replace(['\n', '\r'], " ");
    for token in ["authorization:", "bearer ", "password="] {
        if let Some(idx) = redacted.to_ascii_lowercase().find(token) {
            redacted.truncate(idx + token.len());
            redacted.push_str("[REDACTED]");
            break;
        }
    }
    if redacted.len() > MAX_DEBUG_MESSAGE_LEN {
        redacted.truncate(MAX_DEBUG_MESSAGE_LEN);
        redacted.push('…');
    }
    redacted
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

fn network_err(e: impl std::fmt::Display) -> ApiError {
    ApiError {
        code: ErrorCode::Network,
        message: e.to_string(),
    }
}

fn provider_err(e: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        mode: SendFailureMode::Provider,
        message: "provider rejected the request".to_string(),
        debug_message: e.to_string(),
    }
}

fn network_err_provider(e: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        mode: SendFailureMode::Network,
        message: "network error while contacting provider".to_string(),
        debug_message: e.to_string(),
    }
}

fn validate_transport_config(cfg: &EmailTransportConfig) -> Result<(), ApiError> {
    if cfg.imap.host.trim().is_empty() || cfg.smtp.host.trim().is_empty() {
        return Err(ApiError {
            code: ErrorCode::Validation,
            message: "imap/smtp host is required".to_string(),
        });
    }
    if cfg.attachment_policy.max_size_bytes == 0
        || cfg.attachment_policy.allowed_content_types.is_empty()
    {
        return Err(ApiError {
            code: ErrorCode::Validation,
            message: "attachment policy must define max size and allowed types".to_string(),
        });
    }
    validate_auth_config("imap", &cfg.imap.user, &cfg.imap.auth)?;
    validate_auth_config("smtp", &cfg.smtp.user, &cfg.smtp.auth)?;
    Ok(())
}

fn validate_auth_config(protocol: &str, user: &str, auth: &AuthConfig) -> Result<(), ApiError> {
    if user.trim().is_empty() {
        return Err(ApiError {
            code: ErrorCode::Validation,
            message: format!("{protocol}.user is required"),
        });
    }
    match (&auth.password, &auth.oauth_token) {
        (Some(_), None) | (None, Some(_)) => Ok(()),
        _ => Err(ApiError {
            code: ErrorCode::Validation,
            message: format!("{protocol}.auth must set exactly one of password/oauth_token"),
        }),
    }
}

struct Xoauth2Authenticator {
    payload: String,
}

impl Authenticator for Xoauth2Authenticator {
    type Response = String;

    fn process(&self, _challenge: &[u8]) -> Self::Response {
        self.payload.clone()
    }
}

fn imap_login(
    client: imap::Client<rustls_connector::TlsStream<TcpStream>>,
    cfg: &ImapConfig,
) -> Result<imap::Session<rustls_connector::TlsStream<TcpStream>>, ApiError> {
    match (&cfg.auth.password, &cfg.auth.oauth_token) {
        (Some(password), None) => client
            .login(&cfg.user, password)
            .map_err(|e| network_err(e.0)),
        (None, Some(token)) => {
            let authenticator = Xoauth2Authenticator {
                payload: format!("user={}\x01auth=Bearer {}\x01\x01", cfg.user, token),
            };
            client
                .authenticate("XOAUTH2", &authenticator)
                .map_err(|e| network_err(e.0))
        }
        _ => Err(ApiError {
            code: ErrorCode::Validation,
            message: "imap.auth must set exactly one of password/oauth_token".to_string(),
        }),
    }
}

fn redact_email(value: &str) -> String {
    if let Some((local, domain)) = value.split_once('@') {
        let local_head = local.chars().next().unwrap_or('*');
        return format!("{}***@{}", local_head, domain);
    }
    "***".to_string()
}

fn log_debug(context: &str, details: &str) {
    eprintln!("[prx_email][debug] {} | {}", context, details);
}

#[cfg(test)]
mod tests {
    use super::{
        AttachmentPolicy, ListMessagesRequest, ReplyEmailRequest, SearchMessagesRequest,
        build_references_chain, parse_mime_message,
    };
    use crate::db::{EmailRepository, EmailStore, NewAccount, NewMessage};
    use crate::plugin::EmailPlugin;

    #[test]
    fn parse_mime_extracts_text_html_and_attachments() {
        let raw = b"From: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Hello\r\nMessage-ID: <m1@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=abc\r\n\r\n--abc\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n--abc\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body>HTML body</body></html>\r\n--abc\r\nContent-Type: text/plain; name=file.txt\r\nContent-Disposition: attachment; filename=file.txt\r\n\r\nhello\r\n--abc--\r\n";
        let parsed =
            parse_mime_message(raw, None, &AttachmentPolicy::default(), 1, 1).expect("parse");
        assert_eq!(parsed.body_text.as_deref(), Some("Plain body"));
        assert!(parsed.body_html.is_some());
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].filename.as_deref(), Some("file.txt"));
        assert!(parsed.attachments[0].local_path.is_none());
    }

    #[test]
    fn references_chain_appends_parent_message_id() {
        let refs = build_references_chain(Some("<a@x> <b@x>"), "<c@x>");
        assert_eq!(refs, "<a@x> <b@x> <c@x>");
        let no_dup = build_references_chain(Some("<a@x> <c@x>"), "<c@x>");
        assert_eq!(no_dup, "<a@x> <c@x>");
    }

    #[test]
    fn reply_sets_in_reply_to_header_on_outbox() {
        let store = EmailStore::open_in_memory().expect("open");
        store.migrate().expect("migrate");
        let repo = EmailRepository::new(&store);
        let account_id = repo
            .create_account(&NewAccount {
                email: "alice@example.com".to_string(),
                display_name: None,
                now_ts: 1,
            })
            .expect("create account");
        repo.set_account_feature_flag(account_id, "email_reply", true, 1)
            .expect("enable");

        repo.upsert_message(&NewMessage {
            account_id,
            folder_id: None,
            message_id: "<root@example.com>".to_string(),
            subject: Some("Root".to_string()),
            sender: Some("bob@example.com".to_string()),
            recipients: Some("alice@example.com".to_string()),
            snippet: Some("root".to_string()),
            body_text: Some("root".to_string()),
            body_html: None,
            attachments_json: None,
            references_header: None,
            received_at: Some(1),
            now_ts: 1,
        })
        .expect("insert parent");

        let plugin = EmailPlugin::new(repo);
        let res = plugin.reply(ReplyEmailRequest {
            account_id,
            in_reply_to_message_id: "<root@example.com>".to_string(),
            body_text: "hi".to_string(),
            now_ts: 2,
            attachment: None,
            failure_mode: Some(super::SendFailureMode::Provider),
        });
        assert!(!res.ok);
        let outbox = plugin
            .get_outbox(res.data.expect("has state").outbox_id)
            .expect("get")
            .expect("exists");
        assert_eq!(
            outbox.in_reply_to_message_id.as_deref(),
            Some("<root@example.com>")
        );
    }

    #[test]
    fn parse_mime_fallback_message_id_is_stable_and_unique_for_different_payloads() {
        let raw_a = b"From: a@example.com\r\nSubject: A\r\n\r\nhello";
        let raw_b = b"From: b@example.com\r\nSubject: B\r\n\r\nworld";
        assert_eq!(raw_a.len(), raw_b.len());

        let parsed_a =
            parse_mime_message(raw_a, None, &AttachmentPolicy::default(), 9, 100).expect("a");
        let parsed_b =
            parse_mime_message(raw_b, None, &AttachmentPolicy::default(), 9, 100).expect("b");

        assert!(parsed_a.message_id.starts_with("generated-9-100-"));
        assert!(parsed_b.message_id.starts_with("generated-9-100-"));
        assert_ne!(parsed_a.message_id, parsed_b.message_id);
    }

    #[test]
    fn list_search_reject_out_of_range_limit() {
        let store = EmailStore::open_in_memory().expect("open");
        store.migrate().expect("migrate");
        let repo = EmailRepository::new(&store);
        let account_id = repo
            .create_account(&NewAccount {
                email: "limit@example.com".to_string(),
                display_name: None,
                now_ts: 1,
            })
            .expect("create account");
        repo.set_account_feature_flag(account_id, "inbox_read", true, 1)
            .expect("enable read");
        repo.set_account_feature_flag(account_id, "inbox_search", true, 1)
            .expect("enable search");
        let plugin = EmailPlugin::new(repo);

        let list_err = plugin
            .list(ListMessagesRequest {
                account_id,
                limit: 0,
            })
            .expect_err("list limit should fail");
        assert_eq!(list_err.code, super::ErrorCode::Validation);

        let search_err = plugin
            .search(SearchMessagesRequest {
                account_id,
                query: "x".to_string(),
                limit: 501,
            })
            .expect_err("search limit should fail");
        assert_eq!(search_err.code, super::ErrorCode::Validation);
    }

    #[test]
    fn run_sync_runner_respects_max_concurrency_cap() {
        let store = EmailStore::open_in_memory().expect("open");
        store.migrate().expect("migrate");
        let repo = EmailRepository::new(&store);
        let plugin = EmailPlugin::new(repo);

        let jobs = vec![
            super::SyncJob {
                account_id: 1,
                folder: "INBOX".to_string(),
                max_messages: 10,
            },
            super::SyncJob {
                account_id: 2,
                folder: "INBOX".to_string(),
                max_messages: 10,
            },
            super::SyncJob {
                account_id: 3,
                folder: "INBOX".to_string(),
                max_messages: 10,
            },
        ];
        let cfg = super::SyncRunnerConfig {
            max_concurrency: 2,
            base_backoff_seconds: 1,
            max_backoff_seconds: 10,
        };
        let report = plugin.run_sync_runner(&jobs, 10, &cfg);
        assert_eq!(report.attempted, 2);
    }

    #[test]
    fn reload_auth_from_env_updates_tokens() {
        let store = EmailStore::open_in_memory().expect("open");
        store.migrate().expect("migrate");
        let repo = EmailRepository::new(&store);
        let plugin = EmailPlugin::new_with_config(
            repo,
            super::EmailTransportConfig {
                imap: super::ImapConfig {
                    host: "imap.example.com".to_string(),
                    port: 993,
                    user: "alice@example.com".to_string(),
                    auth: super::AuthConfig {
                        password: None,
                        oauth_token: Some("old-imap".to_string()),
                    },
                },
                smtp: super::SmtpConfig {
                    host: "smtp.example.com".to_string(),
                    port: 465,
                    user: "alice@example.com".to_string(),
                    auth: super::AuthConfig {
                        password: None,
                        oauth_token: Some("old-smtp".to_string()),
                    },
                },
                attachment_store: None,
                attachment_policy: super::AttachmentPolicy::default(),
            },
        );

        unsafe {
            std::env::set_var("PRX_TEST_IMAP_OAUTH_TOKEN", "new-imap");
            std::env::set_var("PRX_TEST_SMTP_OAUTH_TOKEN", "new-smtp");
        }
        plugin.reload_auth_from_env("PRX_TEST");
        let cfg = plugin.config.borrow();
        let cfg = cfg.as_ref().expect("cfg");
        assert_eq!(cfg.imap.auth.oauth_token.as_deref(), Some("new-imap"));
        assert_eq!(cfg.smtp.auth.oauth_token.as_deref(), Some("new-smtp"));
    }
}
