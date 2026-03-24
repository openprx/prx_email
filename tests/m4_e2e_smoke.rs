use prx_email::db::{EmailRepository, EmailStore, NewAccount};
use prx_email::plugin::{
    AttachmentPolicy, AuthConfig, EmailPlugin, EmailTransportConfig, ImapConfig, ReplyEmailRequest, SendEmailRequest,
    SmtpConfig, SyncRequest,
};

#[test]
#[ignore = "requires real IMAP/SMTP credentials via env vars"]
fn e2e_sync_send_reply_smoke() {
    let imap_host = std::env::var("E2E_IMAP_HOST").expect("E2E_IMAP_HOST");
    let imap_port: u16 = std::env::var("E2E_IMAP_PORT")
        .unwrap_or_else(|_| "993".to_string())
        .parse()
        .expect("E2E_IMAP_PORT");
    let smtp_host = std::env::var("E2E_SMTP_HOST").expect("E2E_SMTP_HOST");
    let smtp_port: u16 = std::env::var("E2E_SMTP_PORT")
        .unwrap_or_else(|_| "465".to_string())
        .parse()
        .expect("E2E_SMTP_PORT");
    let user = std::env::var("E2E_EMAIL_USER").expect("E2E_EMAIL_USER");
    let password = std::env::var("E2E_EMAIL_PASS").ok();
    let oauth_token = std::env::var("E2E_OAUTH_TOKEN").ok();
    assert!(
        password.is_some() ^ oauth_token.is_some(),
        "set exactly one of E2E_EMAIL_PASS / E2E_OAUTH_TOKEN"
    );
    let target = std::env::var("E2E_TARGET_EMAIL").unwrap_or_else(|_| user.clone());

    let now = 1_800_000_000i64;
    let store = EmailStore::open_in_memory().expect("open");
    store.migrate().expect("migrate");
    let repo = EmailRepository::new(&store);
    let account_id = repo
        .create_account(&NewAccount {
            email: user.clone(),
            display_name: Some("E2E".to_string()),
            now_ts: now,
        })
        .expect("create account");

    let plugin = EmailPlugin::new_with_config(
        repo,
        EmailTransportConfig {
            imap: ImapConfig {
                host: imap_host,
                port: imap_port,
                user: user.clone(),
                auth: AuthConfig {
                    password: password.clone(),
                    oauth_token: oauth_token.clone(),
                },
            },
            smtp: SmtpConfig {
                host: smtp_host,
                port: smtp_port,
                user: user.clone(),
                auth: AuthConfig { password, oauth_token },
            },
            attachment_store: None,
            attachment_policy: AttachmentPolicy::default(),
        },
    );

    plugin
        .set_account_feature(account_id, "email_send", true, now)
        .expect("enable send");
    plugin
        .set_account_feature(account_id, "email_reply", true, now)
        .expect("enable reply");

    plugin
        .sync(SyncRequest {
            account_id,
            folder: Some("INBOX".to_string()),
            cursor: None,
            now_ts: now,
            max_messages: 20,
        })
        .expect("sync inbox");

    let send_res = plugin.send(SendEmailRequest {
        account_id,
        to: target,
        subject: format!("[prx_email e2e] {}", now),
        body_text: "smoke send body".to_string(),
        now_ts: now + 1,
        attachment: None,
        failure_mode: None,
    });
    assert!(send_res.ok, "send should succeed: {:?}", send_res.error);

    plugin
        .sync(SyncRequest {
            account_id,
            folder: Some("Sent".to_string()),
            cursor: None,
            now_ts: now + 2,
            max_messages: 20,
        })
        .expect("sync sent");

    let parent = plugin
        .list(prx_email::plugin::ListMessagesRequest { account_id, limit: 20 })
        .expect("list")
        .into_iter()
        .find(|m| m.sender.as_deref() == Some(user.as_str()))
        .expect("find a sent message to reply");

    let reply_res = plugin.reply(ReplyEmailRequest {
        account_id,
        in_reply_to_message_id: parent.message_id,
        body_text: "smoke reply body".to_string(),
        now_ts: now + 3,
        attachment: None,
        failure_mode: None,
    });
    assert!(reply_res.ok, "reply should succeed: {:?}", reply_res.error);
}
