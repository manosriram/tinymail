use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION_LIFETIME_SECS: u64 = 128 * 24 * 60 * 60;

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    #[serde(default = "default_sent")]
    pub sent_folder: String,
    #[serde(default = "default_drafts")]
    pub drafts_folder: String,
    #[serde(default = "default_archive")]
    pub archive_folder: String,
    #[serde(default = "default_trash")]
    pub trash_folder: String,
}

fn default_sent() -> String {
    "Sent".into()
}
fn default_drafts() -> String {
    "Drafts".into()
}
fn default_archive() -> String {
    "Archive".into()
}
fn default_trash() -> String {
    "Trash".into()
}

fn config_path() -> Result<PathBuf, String> {
    let mut path = dirs::home_dir().ok_or("could not resolve home dir")?;
    path.push("tinymail.yml");
    Ok(path)
}

fn key_path() -> Result<PathBuf, String> {
    let mut path = dirs::home_dir().ok_or("could not resolve home dir")?;
    path.push(".tinymail.key");
    Ok(path)
}

fn old_named_path() -> Result<PathBuf, String> {
    let mut path = dirs::home_dir().ok_or("could not resolve home dir")?;
    path.push("email-client.yml");
    Ok(path)
}

fn old_dotfile_path() -> Result<PathBuf, String> {
    let mut path = dirs::home_dir().ok_or("could not resolve home dir")?;
    path.push(".email-client.yml");
    Ok(path)
}

// The account password is encrypted at rest with a random key generated on
// first run and stored in `~/.tinymail.key` (0600) — no master password to
// type, matching how most desktop mail clients (Thunderbird, Mail.app-without-
// keychain) just keep the credential locally. `saved_at` lets the app force a
// fresh password entry after SESSION_LIFETIME_SECS instead of trusting the
// local key forever.
#[derive(Serialize, Deserialize, Clone)]
struct EncryptedSecret {
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct StoredAccount {
    #[serde(flatten)]
    account: Account,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    encrypted_password: Option<EncryptedSecret>,
    #[serde(default)]
    saved_at: u64,
}

/// One-time migration from older config locations/names to the current
/// `~/tinymail.yml`, preserving the existing account.
fn migrate_old_config_if_present(path: &PathBuf) -> Result<(), String> {
    for renamed in [old_named_path()?, old_dotfile_path()?] {
        if renamed.exists() {
            fs::rename(&renamed, path).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Ok(())
}

fn write_stored(path: &PathBuf, stored: &StoredAccount) -> Result<(), String> {
    let yaml = serde_yaml::to_string(stored).map_err(|e| e.to_string())?;
    fs::write(path, yaml).map_err(|e| e.to_string())
}

fn read_stored(path: &PathBuf) -> Result<StoredAccount, String> {
    let yaml = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_yaml::from_str(&yaml).map_err(|e| e.to_string())
}

fn local_key() -> Result<[u8; 32], String> {
    let path = key_path()?;
    if let Ok(bytes) = fs::read(&path) {
        let encoded = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        let decoded = STANDARD.decode(encoded.trim()).map_err(|e| e.to_string())?;
        let mut key = [0u8; 32];
        if decoded.len() == 32 {
            key.copy_from_slice(&decoded);
            return Ok(key);
        }
    }

    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    fs::write(&path, STANDARD.encode(key)).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn encrypt_password(password: &str) -> Result<EncryptedSecret, String> {
    let key = local_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, password.as_bytes())
        .map_err(|e| e.to_string())?;

    Ok(EncryptedSecret {
        nonce: STANDARD.encode(nonce_bytes),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

fn decrypt_password(secret: &EncryptedSecret) -> Result<String, String> {
    let key = local_key()?;
    let nonce_bytes = STANDARD.decode(&secret.nonce).map_err(|e| e.to_string())?;
    let ciphertext = STANDARD.decode(&secret.ciphertext).map_err(|e| e.to_string())?;

    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| "could not decrypt stored password".to_string())?;
    String::from_utf8(plaintext).map_err(|e| e.to_string())
}

pub fn save(account: &Account, password: &str) -> Result<(), String> {
    let encrypted_password = Some(encrypt_password(password)?);
    let path = config_path()?;
    write_stored(&path, &StoredAccount { account: account.clone(), encrypted_password, saved_at: now() })
}

pub fn load() -> Result<Option<Account>, String> {
    let path = config_path()?;
    if !path.exists() {
        migrate_old_config_if_present(&path)?;
    }
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_stored(&path)?.account))
}

/// True once SESSION_LIFETIME_SECS has passed since the password was last
/// saved, or if there's no usable stored password at all (e.g. a config left
/// over from an older version) — either way the caller should send the user
/// back through account setup to re-enter their password.
pub fn is_expired() -> Result<bool, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let stored = read_stored(&path)?;
    if stored.encrypted_password.is_none() {
        return Ok(true);
    }
    Ok(now().saturating_sub(stored.saved_at) > SESSION_LIFETIME_SECS)
}

/// Decrypts the stored password using the local key file — no user input
/// needed, unless the session has expired (checked separately via `is_expired`).
pub fn credentials() -> Result<(Account, String), String> {
    let path = config_path()?;
    if !path.exists() {
        return Err("no account configured".into());
    }
    let stored = read_stored(&path)?;
    let secret = stored.encrypted_password.ok_or("account has no stored password; reconnect")?;
    let password = decrypt_password(&secret)?;
    Ok((stored.account, password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests below share ~/tinymail.yml (config_path() is not test-overridable),
    // so they must not run concurrently or they'll clobber each other's file.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn account_defaults_archive_and_trash_folders_when_missing() {
        // Configs written before archive/trash support existed won't have
        // these keys — must not fail to load, must fall back sensibly.
        let yaml = "imap_host: imap.example.com\nimap_port: 993\nsmtp_host: smtp.example.com\nsmtp_port: 465\nusername: old@example.com\n";
        let account: Account = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(account.sent_folder, "Sent");
        assert_eq!(account.drafts_folder, "Drafts");
        assert_eq!(account.archive_folder, "Archive");
        assert_eq!(account.trash_folder, "Trash");
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let _guard = TEST_LOCK.lock().unwrap();
        let secret = encrypt_password("hunter2").unwrap();
        assert_eq!(decrypt_password(&secret).unwrap(), "hunter2");
    }

    #[test]
    fn save_load_credentials_round_trip() {
        let _guard = TEST_LOCK.lock().unwrap();
        let path = config_path().unwrap();
        let backup = fs::read(&path).ok();

        let account = Account {
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            username: "test@example.com".into(),
            sent_folder: "Sent".into(),
            drafts_folder: "Drafts".into(),
            archive_folder: "Archive".into(),
            trash_folder: "Trash".into(),
        };
        save(&account, "hunter2").unwrap();

        let loaded = load().unwrap().expect("account should load");
        assert_eq!(loaded.username, "test@example.com");
        assert!(!is_expired().unwrap());

        let (creds_account, password) = credentials().unwrap();
        assert_eq!(creds_account.username, "test@example.com");
        assert_eq!(password, "hunter2");

        match backup {
            Some(bytes) => fs::write(&path, bytes).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    #[test]
    fn stale_saved_at_counts_as_expired() {
        let _guard = TEST_LOCK.lock().unwrap();
        let path = config_path().unwrap();
        let backup = fs::read(&path).ok();

        let account = Account {
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            username: "old@example.com".into(),
            sent_folder: "Sent".into(),
            drafts_folder: "Drafts".into(),
            archive_folder: "Archive".into(),
            trash_folder: "Trash".into(),
        };
        let stored = StoredAccount {
            account,
            encrypted_password: Some(encrypt_password("hunter2").unwrap()),
            saved_at: now() - SESSION_LIFETIME_SECS - 1,
        };
        write_stored(&path, &stored).unwrap();

        assert!(is_expired().unwrap());

        match backup {
            Some(bytes) => fs::write(&path, bytes).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }
}
