use prx_email::db::{EmailRepository, EmailStore, NewAccount, NewMessage};
use prx_email::plugin::{EmailPlugin, GetMessageRequest, ListMessagesRequest, SearchMessagesRequest, SyncRequest};

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
        .list(ListMessagesRequest { account_id, limit: 10 })
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
