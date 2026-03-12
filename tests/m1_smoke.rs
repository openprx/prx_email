use prx_email::db::{EmailRepository, EmailStore, NewAccount, NewMessage};
use prx_email::plugin::{
    EmailPlugin, ErrorCode, GetMessageRequest, ListMessagesRequest, ReplyEmailRequest,
    RetryOutboxRequest, SearchMessagesRequest, SendEmailRequest, SendFailureMode, SyncRequest,
};

#[test]
fn m1_smoke_flow() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");

    let repo = EmailRepository::new(&store);
    let account_id = repo
        .create_account(&NewAccount {
            email: "alice@example.com".to_string(),
            display_name: Some("Alice".to_string()),
            now_ts: 1,
        })
        .expect("create account");

    let plugin = EmailPlugin::new(repo);

    let msg_id = plugin
        .ingest_message(NewMessage {
            account_id,
            folder_id: None,
            message_id: "msg-1".to_string(),
            subject: Some("Hello Rust".to_string()),
            sender: Some("bob@example.com".to_string()),
            recipients: Some("alice@example.com".to_string()),
            snippet: Some("hello".to_string()),
            body_text: Some("hello world".to_string()),
            received_at: Some(2),
            now_ts: 2,
        })
        .expect("ingest");
    assert!(msg_id > 0);

    plugin
        .sync(SyncRequest {
            account_id,
            folder_id: None,
            cursor: Some("cur-1".to_string()),
            now_ts: 3,
        })
        .expect("sync");

    let listed = plugin
        .list(ListMessagesRequest {
            account_id,
            limit: 10,
        })
        .expect("list");
    assert_eq!(listed.len(), 1);

    let got = plugin
        .get(GetMessageRequest {
            account_id,
            message_id: "msg-1".to_string(),
        })
        .expect("get")
        .expect("message exists");
    assert_eq!(got.subject.as_deref(), Some("Hello Rust"));

    let searched = plugin
        .search(SearchMessagesRequest {
            account_id,
            query: "Rust".to_string(),
            limit: 10,
        })
        .expect("search");
    assert_eq!(searched.len(), 1);
}

#[test]
fn m2_send_enqueues_and_marks_sent() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = repo
        .create_account(&NewAccount {
            email: "alice@example.com".to_string(),
            display_name: Some("Alice".to_string()),
            now_ts: 1,
        })
        .expect("create account");
    let plugin = EmailPlugin::new(repo);

    let resp = plugin.send(SendEmailRequest {
        account_id,
        to: "bob@example.com".to_string(),
        subject: "Hello".to_string(),
        body_text: "Body".to_string(),
        now_ts: 10,
        failure_mode: None,
    });

    assert!(resp.ok);
    let data = resp.data.expect("send data");
    assert_eq!(data.status, "sent");
    assert!(data.provider_message_id.is_some());

    let outbox = plugin
        .get_outbox(data.outbox_id)
        .expect("get outbox")
        .unwrap();
    assert_eq!(outbox.status, "sent");
    assert_eq!(outbox.retries, 0);
    assert_eq!(outbox.last_error, None);
}

#[test]
fn m2_reply_references_message_id() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = repo
        .create_account(&NewAccount {
            email: "alice@example.com".to_string(),
            display_name: Some("Alice".to_string()),
            now_ts: 1,
        })
        .expect("create account");
    let plugin = EmailPlugin::new(repo);

    plugin
        .ingest_message(NewMessage {
            account_id,
            folder_id: None,
            message_id: "remote-1".to_string(),
            subject: Some("Original".to_string()),
            sender: Some("bob@example.com".to_string()),
            recipients: Some("alice@example.com".to_string()),
            snippet: Some("hi".to_string()),
            body_text: Some("hello".to_string()),
            received_at: Some(2),
            now_ts: 2,
        })
        .expect("ingest");

    let resp = plugin.reply(ReplyEmailRequest {
        account_id,
        in_reply_to_message_id: "remote-1".to_string(),
        body_text: "reply-body".to_string(),
        now_ts: 11,
        failure_mode: None,
    });

    assert!(resp.ok);
    let data = resp.data.expect("reply data");
    let outbox = plugin
        .get_outbox(data.outbox_id)
        .expect("get outbox")
        .unwrap();
    assert_eq!(outbox.in_reply_to_message_id.as_deref(), Some("remote-1"));
    assert_eq!(outbox.to_recipients, "bob@example.com");
}

#[test]
fn m2_failed_retry_flow() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = repo
        .create_account(&NewAccount {
            email: "alice@example.com".to_string(),
            display_name: Some("Alice".to_string()),
            now_ts: 1,
        })
        .expect("create account");
    let plugin = EmailPlugin::new(repo);

    let first = plugin.send(SendEmailRequest {
        account_id,
        to: "bob@example.com".to_string(),
        subject: "Hello".to_string(),
        body_text: "Body".to_string(),
        now_ts: 100,
        failure_mode: Some(SendFailureMode::Network),
    });

    assert!(!first.ok);
    assert_eq!(
        first.error.as_ref().map(|e| &e.code),
        Some(&ErrorCode::Network)
    );
    let failed = first.data.expect("failed data");
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.retries, 1);
    assert!(failed.next_attempt_at > 100);

    let retry = plugin.retry_outbox(RetryOutboxRequest {
        outbox_id: failed.outbox_id,
        now_ts: 200,
        failure_mode: None,
    });

    assert!(retry.ok);
    let sent = retry.data.expect("retry data");
    assert_eq!(sent.status, "sent");
    assert_eq!(sent.retries, 1);

    let outbox = plugin
        .get_outbox(sent.outbox_id)
        .expect("get outbox")
        .unwrap();
    assert_eq!(outbox.status, "sent");
    assert_eq!(outbox.retries, 1);
    assert_eq!(outbox.last_error, None);
}
