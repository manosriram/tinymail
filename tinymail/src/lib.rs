mod account;
mod mail;

use account::Account;
use mail::{MessageDetail, MessageSummary, SessionCache};
use std::sync::{Arc, Mutex};
use tauri::State;

// The account password is encrypted at rest with a local key file (see
// account.rs) instead of the OS keychain or a master password. The decrypted
// password is cached here, in memory, after the first command that needs it
// so we don't hit disk + decrypt on every single call.
static CREDENTIALS_CACHE: Mutex<Option<(Account, String)>> = Mutex::new(None);

#[tauri::command]
async fn save_account(account: Account, password: String) -> Result<(), String> {
    let account_for_cache = account.clone();
    let password_for_cache = password.clone();
    tauri::async_runtime::spawn_blocking(move || account::save(&account, &password))
        .await
        .map_err(|e| e.to_string())??;
    *CREDENTIALS_CACHE.lock().unwrap() = Some((account_for_cache, password_for_cache));
    Ok(())
}

#[tauri::command]
async fn get_account() -> Result<Option<Account>, String> {
    tauri::async_runtime::spawn_blocking(account::load)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn is_expired() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(account::is_expired)
        .await
        .map_err(|e| e.to_string())?
}

fn current_account() -> Result<(Account, String), String> {
    if let Some(cached) = CREDENTIALS_CACHE.lock().unwrap().clone() {
        return Ok(cached);
    }
    let (account, password) = account::credentials()?;
    *CREDENTIALS_CACHE.lock().unwrap() = Some((account.clone(), password.clone()));
    Ok((account, password))
}

#[tauri::command]
async fn list_messages(folder: String, cache: State<'_, Arc<SessionCache>>) -> Result<Vec<MessageSummary>, String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::list_messages(&cache, &account, &password, &folder)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_message(folder: String, uid: u32, cache: State<'_, Arc<SessionCache>>) -> Result<MessageDetail, String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::get_message(&cache, &account, &password, &folder, uid)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn mark_read(folder: String, uid: u32, cache: State<'_, Arc<SessionCache>>) -> Result<(), String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::mark_read(&cache, &account, &password, &folder, uid)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_attachment(
    folder: String,
    uid: u32,
    filename: String,
    dest_path: String,
    cache: State<'_, Arc<SessionCache>>,
) -> Result<(), String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::save_attachment(&cache, &account, &password, &folder, uid, &filename, &dest_path)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_attachment_data(
    folder: String,
    uid: u32,
    filename: String,
    cache: State<'_, Arc<SessionCache>>,
) -> Result<String, String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::get_attachment_data(&cache, &account, &password, &folder, uid, &filename)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn read_file_base64(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        Ok(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn send_email(
    to: String,
    subject: String,
    body: String,
    attachment_paths: Vec<String>,
    cache: State<'_, Arc<SessionCache>>,
) -> Result<(), String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::send_email(&cache, &account, &password, &to, &subject, &body, &attachment_paths)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_draft(
    to: String,
    subject: String,
    body: String,
    attachment_paths: Vec<String>,
    cache: State<'_, Arc<SessionCache>>,
) -> Result<(), String> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (account, password) = current_account()?;
        mail::save_draft(&cache, &account, &password, &to, &subject, &body, &attachment_paths)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Arc::new(SessionCache::new()))
        .invoke_handler(tauri::generate_handler![
            save_account,
            get_account,
            is_expired,
            list_messages,
            get_message,
            mark_read,
            save_attachment,
            get_attachment_data,
            read_file_base64,
            send_email,
            save_draft
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
