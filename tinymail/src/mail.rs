use crate::account::Account;
use base64::Engine;
use lettre::message::{Attachment, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use mail_parser::{MessageParser, MimeHeaders};
use serde::Serialize;
use std::net::TcpStream;
use std::sync::Mutex;

pub type ImapSession = imap::Session<native_tls::TlsStream<TcpStream>>;

/// Caches one authenticated IMAP session per app run so folder navigation only
/// pays for a SELECT, not a fresh TCP connect + TLS handshake + LOGIN each click.
pub struct SessionCache(Mutex<Option<ImapSession>>);

impl SessionCache {
    pub fn new() -> Self {
        Self(Mutex::new(None))
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
fn with_connected<F, R>(cache: &SessionCache, account: &Account, password: &str, f: F) -> Result<R, String>
where
    F: Fn(&mut ImapSession) -> Result<R, String>,
{
    let mut guard = cache.0.lock().map_err(|_| "session lock poisoned".to_string())?;
    if guard.is_none() {
        *guard = Some(imap_connect(account, password)?);
    }
    match f(guard.as_mut().unwrap()) {
        Ok(v) => Ok(v),
        Err(_) => {
            let mut fresh = imap_connect(account, password)?;
            let retry = f(&mut fresh);
            *guard = Some(fresh);
            retry
        }
    }
}

fn resolve_folder(account: &Account, folder: &str) -> String {
    match folder {
        "INBOX" => "INBOX".to_string(),
        "SENT" => account.sent_folder.clone(),
        "DRAFTS" => account.drafts_folder.clone(),
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
    pub subject: String,
    pub date: String,
    pub body: String,
    pub attachments: Vec<AttachmentInfo>,
}

pub fn list_messages(cache: &SessionCache, account: &Account, password: &str, folder: &str) -> Result<Vec<MessageSummary>, String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, |session| {
        let mailbox = session.select(&mailbox_name).map_err(|e| e.to_string())?;
        if mailbox.exists == 0 {
            return Ok(vec![]);
        }

        // last 50 messages, newest first
        let start = mailbox.exists.saturating_sub(49).max(1);
        let range = format!("{}:{}", start, mailbox.exists);
        let fetches = session
            .fetch(range, "(UID ENVELOPE FLAGS)")
            .map_err(|e| e.to_string())?;

        let mut summaries: Vec<MessageSummary> = fetches
            .iter()
            .filter_map(|f| {
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
            })
            .collect();

        summaries.reverse();
        Ok(summaries)
    })
}

/// mail_parser's own HTML-to-text conversion drops `href` attributes entirely,
/// keeping only the visible label — so "button" links (an <a> wrapping a
/// styled table/image with no visible URL) become plain, unclickable text.
/// Rewrite anchors as markdown links first so the URL survives, then let
/// mail_parser strip the remaining tags as usual; the frontend already
/// linkifies `[label](url)` markdown in message bodies.
fn html_to_text_with_links(html: &str) -> String {
    static ANCHOR: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let anchor = ANCHOR.get_or_init(|| {
        regex::Regex::new(r#"(?is)<a\s+[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap()
    });

    let rewritten = anchor.replace_all(html, |caps: &regex::Captures| {
        let url = &caps[1];
        let label = mail_parser::decoders::html::html_to_text(&caps[2]);
        let label = label.trim().replace('\n', " ");
        let label = if label.is_empty() { url.to_string() } else { label };
        format!("[{}]({})", label, url)
    });

    mail_parser::decoders::html::html_to_text(&rewritten)
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
    let subject = parsed.subject().unwrap_or_default().to_string();
    let date = parsed
        .date()
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    let body = match parsed.body_html(0) {
        Some(html) => html_to_text_with_links(&html),
        None => parsed.body_text(0).map(|b| b.to_string()).unwrap_or_default(),
    };

    let attachments = parsed
        .attachments()
        .map(|a| {
            let filename = a.attachment_name().unwrap_or("attachment").to_string();
            let content_type = mime_guess::from_path(&filename)
                .first_or_octet_stream()
                .to_string();
            AttachmentInfo { filename, size: a.contents().len(), content_type }
        })
        .collect();

    Ok(MessageDetail { from, to, subject, date, body, attachments })
}

pub fn get_message(cache: &SessionCache, account: &Account, password: &str, folder: &str, uid: u32) -> Result<MessageDetail, String> {
    let mailbox_name = resolve_folder(account, folder);
    with_connected(cache, account, password, |session| {
        session.select(&mailbox_name).map_err(|e| e.to_string())?;
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
    with_connected(cache, account, password, |session| {
        session.select(&mailbox_name).map_err(|e| e.to_string())?;
        session
            .uid_store(uid.to_string(), "+FLAGS (\\Seen)")
            .map_err(|e| e.to_string())?;
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
    with_connected(cache, account, password, |session| {
        session.select(&mailbox_name).map_err(|e| e.to_string())?;
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
    with_connected(cache, account, password, |session| {
        session.select(&mailbox_name).map_err(|e| e.to_string())?;
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

fn build_message(
    account: &Account,
    to: &str,
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

    let message = Message::builder()
        .from(account.username.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
        .to(to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject(subject)
        .multipart(multipart)
        .map_err(|e| e.to_string())?;

    Ok(message.formatted())
}

pub fn send_email(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    to: &str,
    subject: &str,
    body: &str,
    attachment_paths: &[String],
) -> Result<(), String> {
    let raw = build_message(account, to, subject, body, attachment_paths)?;

    let creds = Credentials::new(account.username.clone(), password.to_string());
    let transport = SmtpTransport::relay(&account.smtp_host)
        .map_err(|e| e.to_string())?
        .port(account.smtp_port)
        .credentials(creds)
        .build();

    transport.send_raw(
        &lettre::address::Envelope::new(
            Some(account.username.parse().map_err(|e: lettre::address::AddressError| e.to_string())?),
            vec![to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?],
        )
        .map_err(|e| e.to_string())?,
        &raw,
    )
    .map_err(|e| e.to_string())?;

    // Best-effort copy into Sent — many custom IMAP servers don't auto-populate it.
    // ponytail: no dedup against providers that DO auto-copy; harmless duplicate in that case, fine for v1.
    let _ = with_connected(cache, account, password, |session| {
        session.append(&account.sent_folder, &raw).map_err(|e| e.to_string())
    });

    Ok(())
}

pub fn save_draft(
    cache: &SessionCache,
    account: &Account,
    password: &str,
    to: &str,
    subject: &str,
    body: &str,
    attachment_paths: &[String],
) -> Result<(), String> {
    let raw = build_message(account, to, subject, body, attachment_paths)?;
    with_connected(cache, account, password, |session| {
        session.append(&account.drafts_folder, &raw).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_link_survives_as_markdown_link() {
        let html = r#"
            <p>Hi there,</p>
            <a href="https://example.com/confirm?token=abc">
                <table><tr><td style="background:#00f">Confirm email</td></tr></table>
            </a>
        "#;
        let text = html_to_text_with_links(html);
        assert!(
            text.contains("[Confirm email](https://example.com/confirm?token=abc)"),
            "expected link markdown in: {text}"
        );
    }

    #[test]
    fn plain_html_without_links_still_converts() {
        let text = html_to_text_with_links("<p>Hello <b>world</b></p>");
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
    }
}
