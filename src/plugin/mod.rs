mod email_plugin;

pub use email_plugin::{
    ApiError, ApiResponse, AttachmentInput, AttachmentStoreConfig, EmailPlugin,
    EmailTransportConfig, ErrorCode, GetMessageRequest, ImapConfig, ListMessagesRequest,
    ReplyEmailRequest, RetryOutboxRequest, SearchMessagesRequest, SendEmailRequest,
    SendFailureMode, SendResult, SmtpConfig, SyncRequest,
};
