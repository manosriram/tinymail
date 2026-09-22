use crate::account::Account;
use base64::Engine;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use mail_parser::{MessageParser, MimeHeaders};
use serde::Serialize;
use std::net::TcpStream;
use std::sync::Mutex;

pub type ImapSession = imap::Session<native_tls::TlsStream<TcpStream>>;

/// Caches one authenticated IMAP session per app run so folder navigation only
/// pays for a SELECT, not a fresh TCP connect + TLS handshake + LOGIN each click.
/// Also remembers which mailbox is currently SELECTed on that session, so an
/// action that targets the same folder as the last one (e.g. opening a message
/// right after listing it) can skip SELECT too — it's a round trip in its own
/// right, and often the dominant cost of "open a mail" on a slow connection.
pub struct SessionCache {
    session: Mutex<Option<ImapSession>>,
    selected: Mutex<String>,
}

impl SessionCache {
    pub fn new() -> Self {
        Self { session: Mutex::new(None), selected: Mutex::new(String::new()) }
    }
}

fn imap_connect(account: &Account, password: &str) -> Result<ImapSession, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let client = imap::connect(
        (account.imap_host.as_str(), account.imap_port),
        account.imap_host.as_str(),
        &tls,
    )
    .map_err(|e| e.to_string())?;
    client
        .login(&account.username, password)
        .map_err(|(e, _)| e.to_string())
}

/// Runs `f` against a cached, already-authenticated session; reconnects once and
/// retries if the cached session turned out to be dead (network drop, timeout, etc).
/// Does not touch mailbox selection — for commands like APPEND that don't need one.
fn with_session<F, R>(cache: &SessionCache, account: &Account, password: &str, f: F) -> Result<R, String>
where
    F: Fn(&mut ImapSession) -> Result<R, String>,
{
    let mut guard = cache.session.lock().map_err(|_| "session lock poisoned".to_string())?;
    if guard.is_none() {
        *guard = Some(imap_connect(account, password)?);
        *cache.selected.lock().unwrap() = String::new();
    }
    match f(guard.as_mut().unwrap()) {
        Ok(v) => Ok(v),
        Err(_) => {
            let mut fresh = imap_connect(account, password)?;
            // A reconnected session has nothing SELECTed yet — clear the tracked
            // mailbox before the retry so with_connected's skip-check below can't
            // mistake a stale value for "already selected on this session".
            *cache.selected.lock().unwrap() = String::new();
            let retry = f(&mut fresh);
            *guard = Some(fresh);
            retry
        }
    }
}

/// Same as `with_session`, but first SELECTs `mailbox` — skipped when the session
/// already has that mailbox selected, saving a round trip on back-to-back actions
/// against the same folder (list → open, open → archive, ...).
fn with_connected<F, R>(cache: &SessionCache, account: &Account, password: &str, mailbox: &str, f: F) -> Result<R, String>
where
    F: Fn(&mut ImapSession) -> Result<R, String>,
{
    with_session(cache, account, password, |session| {
        let mut selected = cache.selected.lock().map_err(|_| "session lock poisoned".to_string())?;
        if selected.as_str() != mailbox {
            session.select(mailbox).map_err(|e| e.to_string())?;
            *selected = mailbox.to_string();
        }
        drop(selected);
        f(session)
    })
}

fn resolve_folder(account: &Account, folder: &str) -> String {
    match folder {
        "INBOX" => "INBOX".to_string(),
        "SENT" => account.sent_folder.clone(),
        "DRAFTS" => account.drafts_folder.clone(),
        "ARCHIVE" => account.archive_folder.clone(),
        "TRASH" => account.trash_folder.clone(),
        other => other.to_string(),
    }
}

#[derive(Serialize)]
pub struct MessageSummary {
    pub uid: u32,
    pub from: String,
    pub subject: String,
    pub date: String,
    pub unread: bool,
}

#[derive(Serialize)]
pub struct AttachmentInfo {
    pub filename: String,
    pub size: usize,
    pub content_type: String,
}

#[derive(Serialize)]
pub struct MessageDetail {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    pub subject: String,
    pub date: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
}

fn envelope_to_summary(f: &imap::types::Fetch) -> Option<MessageSummary> {
    let uid = f.uid?;
    let envelope = f.envelope()?;
    let unread = !f.flags().contains(&imap::types::Flag::Seen);
    let from = envelope
        .from
        .as_ref()
        .and_then(|addrs| addrs.first())
        .map(|a| {
            let mailbox = a
                .mailbox
                .map(|m| String::from_utf8_lossy(m).to_string())
                .unwrap_or_default();
            let host = a
                .host
                .map(|h| String::from_utf8_lossy(h).to_string())
                .unwrap_or_default();
            format!("{}@{}", mailbox, host)
        })
        .unwrap_or_default();
    let subject = envelope
        .subject
        .map(|s| String::from_utf8_lossy(s).to_string())
        .unwrap_or_default();
    let date = envelope
        .date
        .map(|d| String::from_utf8_lossy(d).to_string())
        .unwrap_or_default();
    Some(MessageSummary { uid, from, subject, date, unread })
}

pub fn list_messages(cache: &SessionCache, account: &Account, password: &str, folder: &str) -> Result<Vec<MessageSummary>, String> {
    let mailbox_name = resolve_folder(account, folder);
    // Always a real SELECT (never the skip-if-same-mailbox path): the whole point
    // is to learn the current message count, which a stale SELECT can't tell us.
    with_session(cache, account, password, |session| {
        let mailbox = session.select(&mailbox_name).map_err(|e| e.to_string())?;
        *cache.selected.lock().unwrap() = mailbox_name.clone();
        if mailbox.exists == 0 {
            return Ok(vec![]);
        }

        // last 50 messages, newest first
        let start = mailbox.exists.saturating_sub(49).max(1);
        let range = format!("{}:{}", start, mailbox.exists);
        let fetches = session
            .fetch(range, "(UID ENVELOPE FLAGS)")
            .map_err(|e| e.to_string())?;

        let mut summaries: Vec<MessageSummary> = fetches.iter().filter_map(envelope_to_summary).collect();

        // Sort by each message's actual Date header rather than trusting IMAP
        // sequence/arrival order: a message COPY'd into a mailbox (unarchive,
        // restore-from-trash) is appended at the end regardless of when it was
        // originally sent, so sequence order would show it out of place.
        summaries.sort_by_key(|m| std::cmp::Reverse(parse_date_timestamp(&m.date)));
        Ok(summaries)
    })
}

fn parse_date_timestamp(date: &str) -> i64 {
    mail_parser::DateTime::parse_rfc822(date)
        .map(|d| d.to_timestamp())
        .unwrap_or(i64::MIN)
}

/// Sanitizes an HTML mail body for direct rendering (in a sandboxed iframe on
/// the frontend): strips scripts, inline event handlers, and anything else
/// that isn't a safe content tag, while preserving real markup — links,
/// images, tables, formatting — instead of flattening it all to plain text.
fn sanitize_html(html: &str) -> String {
    ammonia::Builder::default()
        .add_tags(["style"])
        .rm_clean_content_tags(["style"])
        .add_generic_attributes(["style", "class", "align", "valign", "bgcolor", "width", "height"])
        // cid: references (inline images) need to survive sanitization intact
        // so parse_message_detail can swap them for data: URIs afterward —
        // deliberately *after* sanitizing, so ammonia only ever parses the
        // small HTML skeleton instead of megabytes of embedded base64 image
        // data (which made opening image-heavy HTML mail take several seconds).
        .add_url_schemes(["cid"])
        .clean(html)
        .to_string()
}

fn parse_message_detail(raw: &[u8]) -> Result<MessageDetail, String> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or("failed to parse message")?;

    let from = parsed
        .from()
        .and_then(|f| f.first())
        .and_then(|a| a.address())
        .unwrap_or_default()
        .to_string();
    let to = parsed
        .to()
        .and_then(|t| t.first())
        .and_then(|a| a.address())
        .unwrap_or_default()
        .to_string();
    let cc = parsed
        .cc()
        .and_then(|c| c.as_list())
        .map(|addrs| addrs.iter().filter_map(|a| a.address()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let subject = parsed.subject().unwrap_or_default().to_string();
    let date = parsed
        .date()
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    // Sanitize first, while `cid:` references are still short opaque strings.
    // Embedding inline images as data: URIs happens below, deliberately after
    // this — ammonia parsing megabytes of base64 image data (instead of a few
    // KB of markup) is what was making "open a mail" take several seconds.
    let mut html_body = parsed.body_html(0).map(|html| sanitize_html(&html));
    let body = parsed.body_text(0).map(|b| b.to_string()).unwrap_or_default();

    // mail_parser puts every non-body part in `attachments()`, including inline
    // images (logos, signature graphics) that the HTML references via `cid:` —
    // those aren't meant to be downloadable files, they're part of the message.
    // Embed any part whose Content-ID is actually referenced in the HTML as a
    // data: URI in place, and leave it out of the attachment list; everything
    // else (real attachments, and unreferenced inline parts) is listed as before.
    let mut attachments = Vec::new();
    for a in parsed.attachments() {
        let cid_ref = a.content_id().map(|cid| format!("cid:{cid}"));
        let referenced = match (&cid_ref, &html_body) {
            (Some(cid_ref), Some(html)) => html.contains(cid_ref.as_str()),
            _ => false,
        };
        if referenced {
            let content_type = a
                .content_type()
                .map(|ct| format!("{}/{}", ct.c_type, ct.c_subtype.as_deref().unwrap_or("octet-stream")))
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let data_uri = format!(
                "data:{};base64,{}",
                content_type,
                base64::engine::general_purpose::STANDARD.encode(a.contents())
            );
            if let Some(html) = html_body.as_mut() {
                *html = html.replace(cid_ref.as_ref().unwrap(), &data_uri);
            }
            continue;
        }

        let filename = a.attachment_name().unwrap_or("attachment").to_string();
        let content_type = mime_guess::from_path(&filename)
            .first_or_octet_stream()
            .to_string();
        attachments.push(AttachmentInfo { filename, size: a.contents().len(), content_type });
    }

    Ok(MessageDetail { from, to, cc, subject, date, body, body_html: html_body, attachments })
}

pub fn get_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<MessageDetail, String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, &mailbox_name, |session| {
        let fetches = session
            .uid_fetch(uid.to_string(), "BODY[]")
            .map_err(|e| e.to_string())?;
        let fetch = fetches.first().ok_or("message not found")?;
        let raw = fetch.body().ok_or("empty message body")?;
        parse_message_detail(raw)
    })
}

pub fn mark_read(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<(), String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, &mailbox_name, |session| {
        session
            .uid_store(uid.to_string(), "+FLAGS (\\Seen)")
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Moves a message to `dest_mailbox` via COPY + STORE \Deleted + EXPUNGE
/// rather than the IMAP MOVE extension (RFC 6851), since not every server
/// (especially smaller/custom ones) implements it.
fn move_message(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    folder: &str,
    uid: u32,
    dest_mailbox: &str,
) -> Result<(), String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, &mailbox_name, |session| {
        session.uid_copy(uid.to_string(), dest_mailbox).map_err(|e| e.to_string())?;
        session
            .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
            .map_err(|e| e.to_string())?;
        session.expunge().map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub fn delete_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<(), String> {
    move_message(cache, account, password, folder, uid, &account.trash_folder)
}

pub fn archive_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<(), String> {
    move_message(cache, account, password, folder, uid, &account.archive_folder)
}

pub fn unarchive_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<(), String> {
    move_message(cache, account, password, folder, uid, "INBOX")
}

pub fn restore_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<(), String> {
    move_message(cache, account, password, folder, uid, "INBOX")
}

/// Permanently removes a message: STORE \Deleted + EXPUNGE, no copy anywhere
/// first. Unlike `move_message`, this is not recoverable.
pub fn permanently_delete_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<(), String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, &mailbox_name, |session| {
        session
            .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
            .map_err(|e| e.to_string())?;
        session.expunge().map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub fn save_attachment(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    folder: &str,
    uid: u32,
    filename: &str,
    dest_path: &str,
) -> Result<(), String> {
    let mailbox_name = resolve_folder(account, folder);
    let dest_path = dest_path.to_string();
    with_connected(cache, account, password, &mailbox_name, |session| {
        let fetches = session
            .uid_fetch(uid.to_string(), "BODY[]")
            .map_err(|e| e.to_string())?;
        let fetch = fetches.first().ok_or("message not found")?;
        let raw = fetch.body().ok_or("empty message body")?;
        let parsed = MessageParser::default().parse(raw).ok_or("failed to parse message")?;

        let attachment = parsed
            .attachments()
            .find(|a| a.attachment_name() == Some(filename))
            .ok_or("attachment not found")?;

        std::fs::write(&dest_path, attachment.contents()).map_err(|e| e.to_string())
    })
}

pub fn get_attachment_data(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    folder: &str,
    uid: u32,
    filename: &str,
) -> Result<String, String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, &mailbox_name, |session| {
        let fetches = session
            .uid_fetch(uid.to_string(), "BODY[]")
            .map_err(|e| e.to_string())?;
        let fetch = fetches.first().ok_or("message not found")?;
        let raw = fetch.body().ok_or("empty message body")?;
        let parsed = MessageParser::default().parse(raw).ok_or("failed to parse message")?;

        let attachment = parsed
            .attachments()
            .find(|a| a.attachment_name() == Some(filename))
            .ok_or("attachment not found")?;

        Ok(base64::engine::general_purpose::STANDARD.encode(attachment.contents()))
    })
}

/// Parses a comma-separated list of addresses (as typed into the Cc/Bcc
/// fields) into mailboxes, skipping blank entries so an empty or
/// trailing-comma field just yields no recipients rather than an error.
fn parse_addresses(input: &str) -> Result<Vec<Mailbox>, String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<Mailbox>().map_err(|e| e.to_string()))
        .collect()
}

fn build_message(
    account: &Account,
    to: &str,
    cc: &str,
    subject: &str,
    body: &str,
    attachment_paths: &[String],
) -> Result<Vec<u8>, String> {
    let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(body.to_string()));

    for path in attachment_paths {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let filename = std::path::Path::new(path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());
        let content_type = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let content_type = lettre::message::header::ContentType::parse(&content_type)
            .map_err(|e| e.to_string())?;
        multipart = multipart.singlepart(Attachment::new(filename).body(bytes, content_type));
    }

    let mut builder = Message::builder()
        .from(account.username.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
        .to(to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject(subject);
    // Cc, unlike Bcc, is a real header — every recipient is meant to see it.
    for mbox in parse_addresses(cc)? {
        builder = builder.cc(mbox);
    }

    let message = builder.multipart(multipart).map_err(|e| e.to_string())?;

    Ok(message.formatted())
}

pub fn send_email(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
    attachment_paths: &[String],
) -> Result<(), String> {
    let raw = build_message(account, to, cc, subject, body, attachment_paths)?;

    let creds = Credentials::new(account.username.clone(), password.to_string());
    let transport = SmtpTransport::relay(&account.smtp_host)
        .map_err(|e| e.to_string())?
        .port(account.smtp_port)
        .credentials(creds)
        .build();

    // The envelope's recipient list is what actually determines delivery
    // (SMTP RCPT TO), separately from the To/Cc headers baked into the raw
    // message above. Bcc recipients go only here — never into a header —
    // since a Bcc header in the raw bytes would leak them to everyone else
    // who received the mail.
    let mut recipients = vec![to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?];
    recipients.extend(parse_addresses(cc)?.into_iter().map(|m| m.email));
    recipients.extend(parse_addresses(bcc)?.into_iter().map(|m| m.email));

    transport.send_raw(
        &lettre::address::Envelope::new(
            Some(account.username.parse().map_err(|e: lettre::address::AddressError| e.to_string())?),
            recipients,
        )
        .map_err(|e| e.to_string())?,
        &raw,
    )
    .map_err(|e| e.to_string())?;

    // Best-effort copy into Sent — many custom IMAP servers don't auto-populate it.
    // ponytail: no dedup against providers that DO auto-copy; harmless duplicate in that case, fine for v1.
    let _ = with_session(cache, account, password, |session| {
        session.append(&account.sent_folder, &raw).map_err(|e| e.to_string())
    });

    Ok(())
}

pub fn save_draft(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    to: &str,
    cc: &str,
    subject: &str,
    body: &str,
    attachment_paths: &[String],
) -> Result<(), String> {
    let raw = build_message(account, to, cc, subject, body, attachment_paths)?;
    with_session(cache, account, password, |session| {
        session.append(&account.drafts_folder, &raw).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_account() -> Account {
        Account {
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            username: "test@example.com".into(),
            sent_folder: "Sent".into(),
            drafts_folder: "Drafts".into(),
            archive_folder: "Archive".into(),
            trash_folder: "Trash".into(),
        }
    }

    #[test]
    fn resolve_folder_maps_virtual_folders_to_account_settings() {
        let account = test_account();
        assert_eq!(resolve_folder(&account, "INBOX"), "INBOX");
        assert_eq!(resolve_folder(&account, "SENT"), "Sent");
        assert_eq!(resolve_folder(&account, "DRAFTS"), "Drafts");
        assert_eq!(resolve_folder(&account, "ARCHIVE"), "Archive");
        assert_eq!(resolve_folder(&account, "TRASH"), "Trash");
    }

    #[test]
    fn resolve_folder_maps_to_custom_account_folder_names() {
        let mut account = test_account();
        account.archive_folder = "All Mail".into();
        account.trash_folder = "Deleted Items".into();
        assert_eq!(resolve_folder(&account, "ARCHIVE"), "All Mail");
        assert_eq!(resolve_folder(&account, "TRASH"), "Deleted Items");
    }

    #[test]
    fn resolve_folder_passes_through_unknown_names() {
        let account = test_account();
        assert_eq!(resolve_folder(&account, "SomeCustomFolder"), "SomeCustomFolder");
    }

    #[test]
    fn parse_date_timestamp_orders_chronologically() {
        let earlier = parse_date_timestamp("Mon, 1 Jan 2024 00:00:00 +0000");
        let later = parse_date_timestamp("Fri, 1 Aug 2025 00:00:00 +0000");
        assert!(later > earlier);
    }

    #[test]
    fn parse_date_timestamp_falls_back_on_garbage() {
        assert_eq!(parse_date_timestamp("not a date"), i64::MIN);
        assert_eq!(parse_date_timestamp(""), i64::MIN);
    }

    #[test]
    fn date_sort_ignores_arrival_order() {
        // A message COPY'd into a mailbox lands at the end regardless of its
        // original Date header — simulate that with summaries in arrival order
        // and confirm sorting fixes it back to chronological (newest first).
        let mut summaries = vec![
            MessageSummary { uid: 1, from: "a".into(), subject: "old".into(), date: "Mon, 1 Jan 2024 00:00:00 +0000".into(), unread: false },
            MessageSummary { uid: 2, from: "b".into(), subject: "newest but arrived first".into(), date: "Fri, 1 Aug 2025 00:00:00 +0000".into(), unread: false },
            MessageSummary { uid: 3, from: "c".into(), subject: "just restored, old date".into(), date: "Tue, 2 Jan 2024 00:00:00 +0000".into(), unread: false },
        ];
        summaries.sort_by_key(|m| std::cmp::Reverse(parse_date_timestamp(&m.date)));
        let subjects: Vec<&str> = summaries.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, vec!["newest but arrived first", "just restored, old date", "old"]);
    }

    #[test]
    fn button_link_survives_sanitization() {
        let html = r#"
            <p>Hi there,</p>
            <a href="https://example.com/confirm?token=abc">
                <table><tr><td style="background:#00f">Confirm email</td></tr></table>
            </a>
        "#;
        let clean = sanitize_html(html);
        assert!(clean.contains(r#"href="https://example.com/confirm?token=abc""#));
        assert!(clean.contains("Confirm email"));
    }

    #[test]
    fn image_src_survives_sanitization() {
        let clean = sanitize_html(r#"<img src="https://example.com/pixel.png" alt="pic">"#);
        assert!(clean.contains(r#"src="https://example.com/pixel.png""#));
    }

    #[test]
    fn script_and_event_handlers_are_stripped() {
        let clean = sanitize_html(
            r#"<p onclick="alert(1)">hi</p><script>alert(1)</script><a href="javascript:alert(1)">bad</a>"#,
        );
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("javascript:"));
    }

    #[test]
    fn parse_message_detail_extracts_basic_fields() {
        let raw = "From: Alice <alice@example.com>\r\n\
                    To: Bob <bob@example.com>\r\n\
                    Subject: Hello World\r\n\
                    Date: Mon, 1 Jan 2024 12:00:00 +0000\r\n\
                    Content-Type: text/plain; charset=utf-8\r\n\
                    \r\n\
                    Hello, this is the body.\r\n";

        let detail = parse_message_detail(raw.as_bytes()).unwrap();
        assert_eq!(detail.from, "alice@example.com");
        assert_eq!(detail.to, "bob@example.com");
        assert_eq!(detail.subject, "Hello World");
        assert!(detail.date.starts_with("2024-01-01"));
        assert!(detail.body.contains("Hello, this is the body."));
        // mail_parser synthesizes an HTML view even for a plain-text body
        // (wrapping/escaping it), so body_html is Some here too — it's only
        // ever None when the message has no readable body part at all.
        assert!(detail.body_html.is_some());
        assert!(detail.attachments.is_empty());
        assert_eq!(detail.cc, "");
    }

    #[test]
    fn parse_message_detail_extracts_multiple_cc_addresses() {
        let raw = "From: alice@example.com\r\n\
                    To: bob@example.com\r\n\
                    Cc: carol@example.com, dave@example.com\r\n\
                    Subject: With Cc\r\n\
                    Content-Type: text/plain; charset=utf-8\r\n\
                    \r\n\
                    Body\r\n";

        let detail = parse_message_detail(raw.as_bytes()).unwrap();
        assert_eq!(detail.cc, "carol@example.com, dave@example.com");
    }

    #[test]
    fn parse_message_detail_falls_back_on_missing_date() {
        let raw = "From: alice@example.com\r\n\
                    To: bob@example.com\r\n\
                    Subject: No Date\r\n\
                    Content-Type: text/plain\r\n\
                    \r\n\
                    Body\r\n";

        let detail = parse_message_detail(raw.as_bytes()).unwrap();
        assert_eq!(detail.date, "");
    }

    #[test]
    fn parse_message_detail_sanitizes_html_body() {
        let raw = "From: alice@example.com\r\n\
                    To: bob@example.com\r\n\
                    Subject: HTML\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    \r\n\
                    <p onclick=\"alert(1)\">hi</p><script>alert(1)</script>\r\n";

        let detail = parse_message_detail(raw.as_bytes()).unwrap();
        let html = detail.body_html.expect("html body should be present");
        assert!(!html.contains("onclick"));
        assert!(!html.contains("<script"));
        assert!(html.contains("hi"));
    }

    #[test]
    fn parse_message_detail_extracts_attachment_metadata() {
        let raw = "From: alice@example.com\r\n\
                    To: bob@example.com\r\n\
                    Subject: With Attachment\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/mixed; boundary=\"BOUNDARY\"\r\n\
                    \r\n\
                    --BOUNDARY\r\n\
                    Content-Type: text/plain; charset=utf-8\r\n\
                    \r\n\
                    Body text here.\r\n\
                    --BOUNDARY\r\n\
                    Content-Type: text/plain; name=\"test.txt\"\r\n\
                    Content-Disposition: attachment; filename=\"test.txt\"\r\n\
                    Content-Transfer-Encoding: base64\r\n\
                    \r\n\
                    aGVsbG8=\r\n\
                    --BOUNDARY--\r\n";

        let detail = parse_message_detail(raw.as_bytes()).unwrap();
        assert!(detail.body.contains("Body text here."));
        assert_eq!(detail.attachments.len(), 1);
        assert_eq!(detail.attachments[0].filename, "test.txt");
        assert_eq!(detail.attachments[0].size, "hello".len());
    }

    #[test]
    fn cid_referenced_image_is_inlined_not_listed_as_attachment() {
        let raw = "From: alice@example.com\r\n\
                    To: bob@example.com\r\n\
                    Subject: With Logo\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/related; boundary=\"BOUNDARY\"\r\n\
                    \r\n\
                    --BOUNDARY\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    \r\n\
                    <p>Hi</p><img src=\"cid:logo123\">\r\n\
                    --BOUNDARY\r\n\
                    Content-Type: image/png\r\n\
                    Content-Disposition: inline; filename=\"logo.png\"\r\n\
                    Content-ID: <logo123>\r\n\
                    Content-Transfer-Encoding: base64\r\n\
                    \r\n\
                    aGVsbG8=\r\n\
                    --BOUNDARY--\r\n";

        let detail = parse_message_detail(raw.as_bytes()).unwrap();
        assert!(detail.attachments.is_empty());
        let html = detail.body_html.expect("html body should be present");
        assert!(!html.contains("cid:logo123"));
        assert!(html.contains("data:image/png;base64,"));
    }

    #[test]
    fn build_message_includes_headers_and_body() {
        let account = test_account();
        let raw = build_message(&account, "dest@example.com", "", "Test Subject", "Test body content", &[]).unwrap();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(raw_str.contains("Test Subject"));
        assert!(raw_str.contains("dest@example.com"));
        assert!(raw_str.contains(&account.username));
        assert!(raw_str.contains("Test body content"));
    }

    #[test]
    fn build_message_includes_cc_header_for_each_address() {
        let account = test_account();
        let raw = build_message(
            &account,
            "dest@example.com",
            "cc1@example.com, cc2@example.com",
            "Subject",
            "Body",
            &[],
        )
        .unwrap();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(raw_str.contains("Cc: cc1@example.com, cc2@example.com"));
    }

    #[test]
    fn build_message_ignores_blank_cc_field() {
        let account = test_account();
        let raw = build_message(&account, "dest@example.com", "  , ", "Subject", "Body", &[]).unwrap();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(!raw_str.contains("Cc:"));
    }

    #[test]
    fn build_message_embeds_attachment_filename() {
        let mut path = std::env::temp_dir();
        path.push(format!("tinymail_test_attachment_{}.txt", std::process::id()));
        std::fs::write(&path, b"attachment contents").unwrap();

        let account = test_account();
        let raw = build_message(
            &account,
            "dest@example.com",
            "",
            "Subject",
            "Body",
            &[path.to_string_lossy().to_string()],
        )
        .unwrap();
        let raw_str = String::from_utf8_lossy(&raw);

        std::fs::remove_file(&path).unwrap();

        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(raw_str.contains(&filename));
    }

    #[test]
    fn build_message_errors_when_attachment_path_missing() {
        let account = test_account();
        let result = build_message(
            &account,
            "dest@example.com",
            "",
            "Subject",
            "Body",
            &["/nonexistent/tinymail-test-path/file.txt".to_string()],
        );
        assert!(result.is_err());
    }
}
