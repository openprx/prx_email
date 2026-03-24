mod email_plugin;

pub use email_plugin::{
    ApiError, ApiResponse, AttachmentInput, AttachmentPolicy, AttachmentStoreConfig, AuthConfig, EmailPlugin,
    EmailTransportConfig, ErrorCode, GetMessageRequest, ImapConfig, ListMessagesRequest, OAuthRefreshProvider,
    RefreshedOAuthToken, ReplyEmailRequest, RetryOutboxRequest, RuntimeMetrics, SearchMessagesRequest,
    SendEmailRequest, SendFailureMode, SendResult, SmtpConfig, SyncJob, SyncRequest, SyncRunnerConfig,
    SyncRunnerReport,
};
