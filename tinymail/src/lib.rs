mod account;
mod mail;

use account::{Account, AccountStatus};
use mail::{MessageDetail, MessageSummary, SessionCache};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::State;

/// Per-account runtime state, keyed by account username. Each account gets
/// its own IMAP session (so polling one inbox never blocks reading another)
/// and its own in-memory copy of the decrypted password — passwords are
/// encrypted at rest with a local key file (see account.rs), and caching
/// here avoids hitting disk + decrypting on every single call.
#[derive(Default)]
struct AppState {
    credentials: Mutex<HashMap<String, (Account, String)>>,
    sessions: Mutex<HashMap<String, Arc<SessionCache>>>,
}

impl AppState {
    fn resolve(&self, username: &str) -> Result<(Arc<SessionCache>, Account, String), String> {
        let cached = self.credentials.lock().unwrap().get(username).cloned();
        let (account, password) = match cached {
            Some(creds) => creds,
            None => {
                let creds = account::credentials(username)?;
                self.credentials.lock().unwrap().insert(username.to_string(), creds.clone());
                creds
            }
        };
        let session = self
            .sessions
            .lock()
            .unwrap()
            .entry(username.to_string())
            .or_insert_with(|| Arc::new(SessionCache::new()))
            .clone();
        Ok((session, account, password))
    }

    /// Drops the cached password and IMAP session, so the next command
    /// reconnects with whatever is now on disk (or fails if it was removed).
    fn forget(&self, username: &str) {
        self.credentials.lock().unwrap().remove(username);
        self.sessions.lock().unwrap().remove(username);
    }
}

async fn blocking<R, F>(f: F) -> Result<R, String>
where
    R: Send + 'static,
    F: FnOnce() -> Result<R, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_account(account: Account, password: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        account::save(&account, &password)?;
        state.forget(&account.username);
        state.credentials.lock().unwrap().insert(account.username.clone(), (account, password));
        Ok(())
    })
    .await
}

#[tauri::command]
async fn list_accounts() -> Result<Vec<AccountStatus>, String> {
    blocking(account::load_all).await
}

#[tauri::command]
async fn remove_account(account: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        account::remove(&account)?;
        state.forget(&account);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn list_messages(account: String, folder: String, state: State<'_, Arc<AppState>>) -> Result<Vec<MessageSummary>, String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::list_messages(&cache, &acct, &pw, &folder)
    })
    .await
}

#[tauri::command]
async fn get_message(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<MessageDetail, String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::get_message(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

/// Warms the backend body cache for messages the user is likely to open next.
#[tauri::command]
async fn prefetch_messages(account: String, folder: String, uids: Vec<u32>, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::prefetch_messages(&cache, &acct, &pw, &folder, &uids)
    })
    .await
}

#[tauri::command]
async fn mark_read(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::mark_read(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

#[tauri::command]
async fn delete_message(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::delete_message(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

#[tauri::command]
async fn archive_message(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::archive_message(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

#[tauri::command]
async fn unarchive_message(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::unarchive_message(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

#[tauri::command]
async fn restore_message(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::restore_message(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

#[tauri::command]
async fn permanently_delete_message(account: String, folder: String, uid: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::permanently_delete_message(&cache, &acct, &pw, &folder, uid)
    })
    .await
}

#[tauri::command]
async fn empty_trash(account: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::empty_trash(&cache, &acct, &pw)
    })
    .await
}

/// Opens a link from mail content in the default browser / mail app. Done in
/// Rust (not via the JS opener plugin) so it doesn't depend on the webview's
/// global plugin object or URL scope, and only ever opens http(s)/mailto.
#[tauri::command]
fn open_link(url: String, app: tauri::AppHandle) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
        return Err(format!("unsupported link: {url}"));
    }
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_attachment(account: String, folder: String, uid: u32, filename: String, dest_path: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::save_attachment(&cache, &acct, &pw, &folder, uid, &filename, &dest_path)
    })
    .await
}

#[tauri::command]
async fn get_attachment_data(account: String, folder: String, uid: u32, filename: String, state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::get_attachment_data(&cache, &acct, &pw, &folder, uid, &filename)
    })
    .await
}

#[tauri::command]
async fn read_file_base64(path: String) -> Result<String, String> {
    blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        Ok(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes))
    })
    .await
}

#[tauri::command]
async fn send_email(account: String, to: String, cc: String, bcc: String, subject: String, body: String, attachment_paths: Vec<String>, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::send_email(&cache, &acct, &pw, &to, &cc, &bcc, &subject, &body, &attachment_paths)
    })
    .await
}

#[tauri::command]
async fn save_draft(account: String, to: String, cc: String, subject: String, body: String, attachment_paths: Vec<String>, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || {
        let (cache, acct, pw) = state.resolve(&account)?;
        mail::save_draft(&cache, &acct, &pw, &to, &cc, &subject, &body, &attachment_paths)
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Arc::new(AppState::default()))
        .invoke_handler(tauri::generate_handler![
            save_account,
            list_accounts,
            remove_account,
            list_messages,
            get_message,
            prefetch_messages,
            mark_read,
            delete_message,
            archive_message,
            unarchive_message,
            restore_message,
            permanently_delete_message,
            empty_trash,
            open_link,
            save_attachment,
            get_attachment_data,
            read_file_base64,
            send_email,
            save_draft
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
