use prx_email::db::{EmailRepository, EmailStore, NewAccount, NewMessage};
use prx_email::plugin::{
    EmailPlugin, ErrorCode, GetMessageRequest, ListMessagesRequest, ReplyEmailRequest,
    RetryOutboxRequest, SearchMessagesRequest, SendEmailRequest, SendFailureMode,
};
use tempfile::NamedTempFile;

const FEATURE_EMAIL_SEND: &str = "email_send";
const FEATURE_EMAIL_REPLY: &str = "email_reply";
const FEATURE_OUTBOX_RETRY: &str = "outbox_retry";

#[test]
fn inbox_list_get_search_regression() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);

    let account_a = create_account(&repo, "alice@example.com", 1);
    let account_b = create_account(&repo, "eve@example.com", 1);

    let plugin = EmailPlugin::new(repo);
    plugin
        .ingest_message(NewMessage {
            account_id: account_a,
            folder_id: None,
            message_id: "m-1".to_string(),
            subject: Some("Rust roadmap".to_string()),
            sender: Some("bob@example.com".to_string()),
            recipients: Some("alice@example.com".to_string()),
            snippet: Some("release planning".to_string()),
            body_text: Some("body-1".to_string()),
            received_at: Some(10),
            now_ts: 10,
        })
        .expect("ingest m-1");
    plugin
        .ingest_message(NewMessage {
            account_id: account_a,
            folder_id: None,
            message_id: "m-2".to_string(),
            subject: Some("Lunch".to_string()),
            sender: Some("carol@example.com".to_string()),
            recipients: Some("alice@example.com".to_string()),
            snippet: Some("sushi at noon".to_string()),
            body_text: Some("body-2".to_string()),
            received_at: Some(20),
            now_ts: 20,
        })
        .expect("ingest m-2");
    plugin
        .ingest_message(NewMessage {
            account_id: account_b,
            folder_id: None,
            message_id: "m-x".to_string(),
            subject: Some("Sensitive".to_string()),
            sender: Some("mallory@example.com".to_string()),
            recipients: Some("eve@example.com".to_string()),
            snippet: Some("should not leak".to_string()),
            body_text: Some("body-x".to_string()),
            received_at: Some(30),
            now_ts: 30,
        })
        .expect("ingest m-x");

    let listed = plugin
        .list(ListMessagesRequest {
            account_id: account_a,
            limit: 10,
        })
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].message_id, "m-2");
    assert_eq!(listed[1].message_id, "m-1");

    let fetched = plugin
        .get(GetMessageRequest {
            account_id: account_a,
            message_id: "m-1".to_string(),
        })
        .expect("get m-1")
        .expect("message exists");
    assert_eq!(fetched.subject.as_deref(), Some("Rust roadmap"));

    let missing = plugin
        .get(GetMessageRequest {
            account_id: account_a,
            message_id: "does-not-exist".to_string(),
        })
        .expect("get missing");
    assert!(missing.is_none());

    let sender_hit = plugin
        .search(SearchMessagesRequest {
            account_id: account_a,
            query: "carol".to_string(),
            limit: 10,
        })
        .expect("search sender");
    assert_eq!(sender_hit.len(), 1);
    assert_eq!(sender_hit[0].message_id, "m-2");

    let snippet_hit = plugin
        .search(SearchMessagesRequest {
            account_id: account_a,
            query: "planning".to_string(),
            limit: 10,
        })
        .expect("search snippet");
    assert_eq!(snippet_hit.len(), 1);
    assert_eq!(snippet_hit[0].message_id, "m-1");

    let isolated = plugin
        .search(SearchMessagesRequest {
            account_id: account_a,
            query: "Sensitive".to_string(),
            limit: 10,
        })
        .expect("search isolated");
    assert!(isolated.is_empty());
}

#[test]
fn send_reply_regression_with_safe_defaults_and_enablement() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = create_account(&repo, "alice@example.com", 1);
    let plugin = EmailPlugin::new(repo);

    let blocked_send = plugin.send(SendEmailRequest {
        account_id,
        to: "bob@example.com".to_string(),
        subject: "Blocked".to_string(),
        body_text: "body".to_string(),
        now_ts: 100,
        failure_mode: None,
    });
    assert!(!blocked_send.ok);
    assert_eq!(
        blocked_send.error.as_ref().map(|e| &e.code),
        Some(&ErrorCode::FeatureDisabled)
    );

    plugin
        .set_account_feature(account_id, FEATURE_EMAIL_SEND, true, 101)
        .expect("enable send");
    plugin
        .set_account_feature(account_id, FEATURE_EMAIL_REPLY, true, 101)
        .expect("enable reply");

    let sent = plugin.send(SendEmailRequest {
        account_id,
        to: "bob@example.com".to_string(),
        subject: "Hello".to_string(),
        body_text: "Body".to_string(),
        now_ts: 102,
        failure_mode: None,
    });
    assert!(sent.ok);
    let sent_data = sent.data.expect("sent data");
    let outbox = plugin
        .get_outbox(sent_data.outbox_id)
        .expect("get outbox")
        .expect("exists");
    assert_eq!(outbox.status, "sent");
    assert!(outbox.provider_message_id.is_some());

    plugin
        .ingest_message(NewMessage {
            account_id,
            folder_id: None,
            message_id: "remote-1".to_string(),
            subject: Some("Original".to_string()),
            sender: Some("bob@example.com".to_string()),
            recipients: Some("alice@example.com".to_string()),
            snippet: Some("hello".to_string()),
            body_text: Some("hello".to_string()),
            received_at: Some(103),
            now_ts: 103,
        })
        .expect("ingest parent");

    let reply = plugin.reply(ReplyEmailRequest {
        account_id,
        in_reply_to_message_id: "remote-1".to_string(),
        body_text: "reply-body".to_string(),
        now_ts: 104,
        failure_mode: None,
    });
    assert!(reply.ok);
    let reply_data = reply.data.expect("reply data");
    let reply_outbox = plugin
        .get_outbox(reply_data.outbox_id)
        .expect("get reply outbox")
        .expect("exists");
    assert_eq!(
        reply_outbox.in_reply_to_message_id.as_deref(),
        Some("remote-1")
    );
    assert_eq!(reply_outbox.to_recipients, "bob@example.com");
}

#[test]
fn outbox_retry_and_failure_recovery_regression() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = create_account(&repo, "alice@example.com", 1);
    let plugin = EmailPlugin::new(repo);

    plugin
        .set_account_feature(account_id, FEATURE_EMAIL_SEND, true, 10)
        .expect("enable send");

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
    let first_state = first.data.expect("first state");
    assert_eq!(first_state.status, "failed");
    assert_eq!(first_state.retries, 1);

    let blocked_retry = plugin.retry_outbox(RetryOutboxRequest {
        outbox_id: first_state.outbox_id,
        now_ts: 110,
        failure_mode: None,
    });
    assert!(!blocked_retry.ok);
    assert_eq!(
        blocked_retry.error.as_ref().map(|e| &e.code),
        Some(&ErrorCode::FeatureDisabled)
    );

    plugin
        .set_account_feature(account_id, FEATURE_OUTBOX_RETRY, true, 111)
        .expect("enable retry");

    let second = plugin.retry_outbox(RetryOutboxRequest {
        outbox_id: first_state.outbox_id,
        now_ts: 200,
        failure_mode: Some(SendFailureMode::Provider),
    });
    assert!(!second.ok);
    assert_eq!(
        second.error.as_ref().map(|e| &e.code),
        Some(&ErrorCode::Provider)
    );
    let second_state = second.data.expect("second state");
    assert_eq!(second_state.status, "failed");
    assert_eq!(second_state.retries, 2);
    assert!(second_state.next_attempt_at > 200);

    let recovered = plugin.retry_outbox(RetryOutboxRequest {
        outbox_id: second_state.outbox_id,
        now_ts: 300,
        failure_mode: None,
    });
    assert!(recovered.ok);
    let recovered_state = recovered.data.expect("recovered state");
    assert_eq!(recovered_state.status, "sent");
    assert_eq!(recovered_state.retries, 2);

    let final_outbox = plugin
        .get_outbox(recovered_state.outbox_id)
        .expect("get outbox")
        .expect("exists");
    assert_eq!(final_outbox.status, "sent");
    assert_eq!(final_outbox.retries, 2);
    assert_eq!(final_outbox.last_error, None);
    assert!(final_outbox.provider_message_id.is_some());
}

#[test]
fn migration_idempotency_across_repeated_runs() {
    let temp_db = NamedTempFile::new().expect("temp db");
    let db_path = temp_db.path().to_string_lossy().into_owned();

    for _ in 0..5 {
        let store = EmailStore::open(&db_path).expect("open");
        store.migrate().expect("migrate");
    }

    let store = EmailStore::open(&db_path).expect("re-open");
    store.migrate().expect("migrate final");
    let repo = EmailRepository::new(&store);

    let flags = repo.list_feature_flags().expect("list feature flags");
    assert_eq!(flags.len(), 5);

    let account_id = create_account(&repo, "migrate@example.com", 5);
    repo.set_account_feature_flag(account_id, FEATURE_EMAIL_SEND, true, 6)
        .expect("set feature");
    repo.set_account_feature_flag(account_id, FEATURE_EMAIL_SEND, false, 7)
        .expect("set feature again");

    let row_count: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM account_feature_flags WHERE account_id = ?1 AND feature_key = ?2",
            rusqlite::params![account_id, FEATURE_EMAIL_SEND],
            |r| r.get(0),
        )
        .expect("count account feature rows");
    assert_eq!(row_count, 1);
}

#[test]
fn staged_rollout_percentage_enablement_regression() {
    let store = EmailStore::open_in_memory().expect("open in memory");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = create_account(&repo, "rollout@example.com", 1);
    let plugin = EmailPlugin::new(repo);

    assert!(!plugin
        .is_feature_enabled(account_id, FEATURE_EMAIL_SEND)
        .expect("feature state"));

    let enabled_0 = plugin
        .apply_percentage_rollout(account_id, FEATURE_EMAIL_SEND, 0, 20)
        .expect("apply rollout 0");
    assert!(!enabled_0);
    assert!(!plugin
        .is_feature_enabled(account_id, FEATURE_EMAIL_SEND)
        .expect("feature state after 0"));

    let enabled_100 = plugin
        .apply_percentage_rollout(account_id, FEATURE_EMAIL_SEND, 100, 21)
        .expect("apply rollout 100");
    assert!(enabled_100);
    assert!(plugin
        .is_feature_enabled(account_id, FEATURE_EMAIL_SEND)
        .expect("feature state after 100"));

    plugin
        .set_feature_default(FEATURE_EMAIL_REPLY, true, 22)
        .expect("set feature default");
    assert!(plugin
        .is_feature_enabled(account_id, FEATURE_EMAIL_REPLY)
        .expect("reply state"));
}

fn create_account(repo: &EmailRepository<'_>, email: &str, now_ts: i64) -> i64 {
    repo.create_account(&NewAccount {
        email: email.to_string(),
        display_name: Some("Test User".to_string()),
        now_ts,
    })
    .expect("create account")
}
