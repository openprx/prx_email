use base64::Engine;
use imap::types::Fetch;
use imap::Authenticator;
use lettre::message::{header::ContentType, Attachment, Mailbox, Message as SmtpMessage, MultiPart, SinglePart};
use lettre::{
    transport::smtp::authentication::{Credentials, Mechanism},
    SmtpTransport, Transport,
};
use mail_parser::{Address, MessageParser, MimeHeaders};
use rustls_connector::RustlsConnector;
use serde::{Deserialize, Serialize};
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
    config: Option<EmailTransportConfig>,
}

impl<'a> EmailPlugin<'a> {
    pub fn new(repo: EmailRepository<'a>) -> Self {
        Self { repo, config: None }
    }

    pub fn new_with_config(repo: EmailRepository<'a>, config: EmailTransportConfig) -> Self {
        if let Err(err) = validate_transport_config(&config) {
            log_debug("transport config validation failed", &err.message);
        }
        Self {
            repo,
            config: Some(config),
        }
    }

    pub fn sync(&self, req: SyncRequest) -> Result<(), ApiError> {
        let cfg = self.config.as_ref().ok_or_else(|| ApiError {
            code: ErrorCode::Validation,
            message: "imap/smtp config missing".to_string(),
        })?;
        validate_transport_config(cfg)?;

        let tls = RustlsConnector::default();
        let tcp = TcpStream::connect((cfg.imap.host.as_str(), cfg.imap.port)).map_err(network_err)?;
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
        Ok(())
    }

    fn ensure_folder(&self, account_id: i64, folder_path: &str, now_ts: i64) -> Result<i64, ApiError> {
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
        self.repo
            .list_messages(req.account_id, req.limit)
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
        self.repo
            .search_messages(req.account_id, &req.query, req.limit)
            .map_err(storage_err)
    }

    pub fn send(&self, req: SendEmailRequest) -> ApiResponse<SendResult> {
        if let Err(e) = self.require_feature(req.account_id, FEATURE_EMAIL_SEND) {
            return ApiResponse { ok: false, data: None, error: Some(e) };
        }

        if req.to.trim().is_empty() || req.subject.trim().is_empty() || req.body_text.trim().is_empty() {
            return fail(ErrorCode::Validation, "to/subject/body_text cannot be empty");
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
            return ApiResponse { ok: false, data: None, error: Some(e) };
        }

        if req.in_reply_to_message_id.trim().is_empty() || req.body_text.trim().is_empty() {
            return fail(ErrorCode::Validation, "in_reply_to_message_id/body_text cannot be empty");
        }

        let parent = match self.repo.get_message(req.account_id, &req.in_reply_to_message_id) {
            Ok(Some(m)) => m,
            Ok(None) => {
                return fail(
                    ErrorCode::Validation,
                    "in_reply_to_message_id does not exist for this account",
                )
            }
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };

        let to = parent.sender.unwrap_or_default();
        if to.trim().is_empty() {
            return fail(ErrorCode::Validation, "cannot infer reply recipient from parent sender");
        }

        let outbox_id = match self.repo.create_outbox_message(&NewOutboxMessage {
            account_id: req.account_id,
            to_recipients: to,
            subject: format!("Re: {}", parent.subject.unwrap_or_else(|| "(no subject)".to_string())),
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
        let account_id = match self.repo.get_outbox_message(req.outbox_id) {
            Ok(Some(outbox)) => outbox.account_id,
            Ok(None) => return fail(ErrorCode::Validation, "outbox record not found"),
            Err(e) => return fail(ErrorCode::Storage, &e.to_string()),
        };
        if let Err(e) = self.require_feature(account_id, FEATURE_OUTBOX_RETRY) {
            return ApiResponse { ok: false, data: None, error: Some(e) };
        }

        self.deliver_outbox(req.outbox_id, req.now_ts, None, req.failure_mode)
    }

    pub fn get_outbox(&self, outbox_id: i64) -> Result<Option<OutboxMessage>, ApiError> {
        self.repo.get_outbox_message(outbox_id).map_err(storage_err)
    }

    pub fn set_feature_default(&self, feature_key: &str, enabled: bool, now_ts: i64) -> Result<(), ApiError> {
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

        match self.send_via_provider(&outbox, attachment, failure_mode) {
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

                log_debug(
                    "outbox send failed",
                    &format!(
                        "account={} to={} err={} debug={}",
                        outbox.account_id,
                        redact_email(&outbox.to_recipients),
                        provider_err.message,
                        provider_err.debug_message
                    ),
                );
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
            return Err(ProviderError { mode, message, debug_message: "simulated failure".to_string() });
        }

        let cfg = self.config.as_ref().ok_or_else(|| ProviderError {
            mode: SendFailureMode::Provider,
            message: "smtp config missing".to_string(),
            debug_message: "smtp transport config is None".to_string(),
        })?;

        let from = Mailbox::new(None, cfg.smtp.user.parse().map_err(|e| ProviderError {
            mode: SendFailureMode::Provider,
            message: "invalid smtp sender address".to_string(),
            debug_message: format!("invalid smtp user address: {e}"),
        })?);
        let to = Mailbox::new(None, outbox.to_recipients.parse().map_err(|e| ProviderError {
            mode: SendFailureMode::Provider,
            message: "invalid recipient address".to_string(),
            debug_message: format!("invalid recipient address: {e}"),
        })?);

        let mut msg_builder = SmtpMessage::builder()
            .from(from)
            .to(to)
            .subject(outbox.subject.clone());
        if let Some(in_reply_to) = &outbox.in_reply_to_message_id {
            msg_builder = msg_builder.in_reply_to(in_reply_to.clone());
            if let Ok(Some(parent)) = self.repo.get_message(outbox.account_id, in_reply_to) {
                let references = build_references_chain(parent.references_header.as_deref(), &parent.message_id);
                if !references.is_empty() {
                    msg_builder = msg_builder.references(references);
                }
            }
        }

        let body = SinglePart::builder()
            .header(ContentType::TEXT_PLAIN)
            .body(outbox.body_text.clone());

        let message = if let Some(att) = attachment {
            let bytes = read_attachment_bytes(&att)?;
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
                builder = builder.credentials(Credentials::new(cfg.smtp.user.clone(), password.clone()));
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
                message: format!("feature '{}' is disabled for account {}", feature_key, account_id),
            }),
            None => Err(ApiError {
                code: ErrorCode::Validation,
                message: format!("unknown feature flag: {}", feature_key),
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderError {
    mode: SendFailureMode,
    message: String,
    debug_message: String,
}

fn read_attachment_bytes(input: &AttachmentInput) -> Result<Vec<u8>, ProviderError> {
    match (&input.base64, &input.path) {
        (Some(b64), None) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(provider_err),
        (None, Some(path)) => fs::read(path).map_err(network_err_provider),
        _ => Err(ProviderError {
            mode: SendFailureMode::Provider,
            message: "attachment requires exactly one of base64 or path".to_string(),
            debug_message: "attachment input has both/none of base64/path".to_string(),
        }),
    }
}

fn parse_imap_fetch(
    account_id: i64,
    folder_id: Option<i64>,
    now_ts: i64,
    fetch: &Fetch,
    attachment_store: Option<&AttachmentStoreConfig>,
) -> Option<NewMessage> {
    let raw = fetch.body()?;
    let parsed = parse_mime_message(raw, attachment_store, account_id, now_ts)?;
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
        .or_else(|| body_html.as_ref().map(|v| v.chars().take(120).collect::<String>()));

    let message_id = message
        .message_id()
        .map(|v| v.to_string())
        .unwrap_or_else(|| format!("generated-{}", raw.len()));

    let attachments = message
        .attachments()
        .enumerate()
        .map(|(idx, part)| {
            let filename = part.attachment_name().map(|n| n.to_string());
            let local_path = persist_attachment(
                attachment_store,
                account_id,
                &message_id,
                idx,
                filename.as_deref(),
                part.contents(),
                now_ts,
            );
            AttachmentMeta {
                filename,
                content_type: part
                    .content_type()
                    .map(|c| format!("{}/{}", c.c_type, c.c_subtype.clone().unwrap_or_else(|| "octet-stream".into()))),
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

fn persist_attachment(
    attachment_store: Option<&AttachmentStoreConfig>,
    account_id: i64,
    message_id: &str,
    index: usize,
    filename: Option<&str>,
    bytes: &[u8],
    now_ts: i64,
) -> Option<String> {
    let cfg = attachment_store?;
    if !cfg.enabled {
        return None;
    }

    let root = Path::new(&cfg.dir);
    let safe_message_id = sanitize_path_component(message_id);
    let safe_filename = sanitize_path_component(filename.unwrap_or("attachment.bin"));
    let account_dir = root.join(account_id.to_string()).join(safe_message_id);
    fs::create_dir_all(&account_dir).ok()?;
    let path: PathBuf = account_dir.join(format!("{}-{}-{}", now_ts, index, safe_filename));
    fs::write(&path, bytes).ok()?;
    Some(path.to_string_lossy().to_string())
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
        return Err(ApiError { code: ErrorCode::Validation, message: "imap/smtp host is required".to_string() });
    }
    validate_auth_config("imap", &cfg.imap.user, &cfg.imap.auth)?;
    validate_auth_config("smtp", &cfg.smtp.user, &cfg.smtp.auth)?;
    Ok(())
}

fn validate_auth_config(protocol: &str, user: &str, auth: &AuthConfig) -> Result<(), ApiError> {
    if user.trim().is_empty() {
        return Err(ApiError { code: ErrorCode::Validation, message: format!("{protocol}.user is required") });
    }
    match (&auth.password, &auth.oauth_token) {
        (Some(_), None) | (None, Some(_)) => Ok(()),
        _ => Err(ApiError { code: ErrorCode::Validation, message: format!("{protocol}.auth must set exactly one of password/oauth_token") }),
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
    use super::{build_references_chain, parse_mime_message, ReplyEmailRequest};
    use crate::db::{EmailRepository, EmailStore, NewAccount, NewMessage};
    use crate::plugin::EmailPlugin;

    #[test]
    fn parse_mime_extracts_text_html_and_attachments() {
        let raw = b"From: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Hello\r\nMessage-ID: <m1@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=abc\r\n\r\n--abc\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n--abc\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body>HTML body</body></html>\r\n--abc\r\nContent-Type: text/plain; name=file.txt\r\nContent-Disposition: attachment; filename=file.txt\r\n\r\nhello\r\n--abc--\r\n";
        let parsed = parse_mime_message(raw, None, 1, 1).expect("parse");
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
        assert_eq!(outbox.in_reply_to_message_id.as_deref(), Some("<root@example.com>"));
    }
}
