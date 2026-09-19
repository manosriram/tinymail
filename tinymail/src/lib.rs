mod account;
mod mail;

use account::Account;
use mail::{MessageDetail, MessageSummary, SessionCache};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
async fn save_account(account: Account, password: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || account::save(&account, &password))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_account() -> Result<Option<Account>, String> {
    tauri::async_runtime::spawn_blocking(account::load)
        .await
        .map_err(|e| e.to_string())?
}

fn current_account() -> Result<(Account, String), String> {
    let account = account::load()?.ok_or("no account configured")?;
    let password = account::password(&account.username)?;
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
            list_messages,
            get_message,
            mark_read,
            save_attachment,
            send_email,
            save_draft
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
