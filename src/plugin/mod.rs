mod email_plugin;

pub use email_plugin::{
    ApiError, ApiResponse, EmailPlugin, ErrorCode, GetMessageRequest, ListMessagesRequest,
    ReplyEmailRequest, RetryOutboxRequest, SearchMessagesRequest, SendEmailRequest,
    SendFailureMode, SendResult, SyncRequest,
};
