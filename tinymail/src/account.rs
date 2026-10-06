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

impl StoredAccount {
    fn is_expired(&self) -> bool {
        self.encrypted_password.is_none() || now().saturating_sub(self.saved_at) > SESSION_LIFETIME_SECS
    }
}

/// On-disk shape of `~/tinymail.yml`: a list of accounts, in the order the
/// user added them (which is also their order in the account switcher).
#[derive(Serialize, Deserialize, Default)]
struct StoredConfig {
    accounts: Vec<StoredAccount>,
}

/// Configs written before multi-account support are a single flat account
/// at the top level — still accepted, and rewritten in the list form the
/// next time anything is saved.
#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigFile {
    Multi(StoredConfig),
    Single(StoredAccount),
}

/// What the frontend needs to render an account in the switcher: its
/// settings plus whether it must go back through setup for a fresh password.
#[derive(Serialize)]
pub struct AccountStatus {
    #[serde(flatten)]
    pub account: Account,
    pub expired: bool,
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

fn write_config(path: &PathBuf, config: &StoredConfig) -> Result<(), String> {
    let yaml = serde_yaml::to_string(config).map_err(|e| e.to_string())?;
    fs::write(path, yaml).map_err(|e| e.to_string())
}

fn read_config(path: &PathBuf) -> Result<StoredConfig, String> {
    if !path.exists() {
        migrate_old_config_if_present(path)?;
    }
    if !path.exists() {
        return Ok(StoredConfig::default());
    }
    let yaml = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if yaml.trim().is_empty() {
        return Ok(StoredConfig::default());
    }
    match serde_yaml::from_str(&yaml).map_err(|e| e.to_string())? {
        ConfigFile::Multi(config) => Ok(config),
        ConfigFile::Single(stored) => Ok(StoredConfig { accounts: vec![stored] }),
    }
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

/// Adds the account, or replaces the existing one with the same username in
/// place (e.g. reconnecting after the session expired), keeping its position.
pub fn save(account: &Account, password: &str) -> Result<(), String> {
    let path = config_path()?;
    let mut config = read_config(&path)?;
    let stored = StoredAccount {
        account: account.clone(),
        encrypted_password: Some(encrypt_password(password)?),
        saved_at: now(),
    };
    match config.accounts.iter_mut().find(|a| a.account.username == account.username) {
        Some(existing) => *existing = stored,
        None => config.accounts.push(stored),
    }
    write_config(&path, &config)
}

/// Every configured account, with `expired` set once SESSION_LIFETIME_SECS
/// has passed since its password was last saved, or if there's no usable
/// stored password at all (e.g. a hand-written config) — either way the
/// caller should send the user back through setup to re-enter it.
pub fn load_all() -> Result<Vec<AccountStatus>, String> {
    let config = read_config(&config_path()?)?;
    Ok(config
        .accounts
        .into_iter()
        .map(|stored| AccountStatus { expired: stored.is_expired(), account: stored.account })
        .collect())
}

pub fn remove(username: &str) -> Result<(), String> {
    let path = config_path()?;
    let mut config = read_config(&path)?;
    config.accounts.retain(|a| a.account.username != username);
    write_config(&path, &config)
}

/// Decrypts the stored password using the local key file — no user input
/// needed, unless the session has expired (reported separately via `load_all`).
pub fn credentials(username: &str) -> Result<(Account, String), String> {
    let config = read_config(&config_path()?)?;
    let stored = config
        .accounts
        .into_iter()
        .find(|a| a.account.username == username)
        .ok_or_else(|| format!("no account configured for {username}"))?;
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

    /// Runs `f` against the real config path, restoring whatever was there
    /// before afterward so tests never clobber a developer's own account.
    fn with_config_backup(f: impl FnOnce(&PathBuf)) {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = config_path().unwrap();
        let backup = fs::read(&path).ok();
        let _ = fs::remove_file(&path);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&path)));
        match backup {
            Some(bytes) => fs::write(&path, bytes).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    fn test_account(username: &str) -> Account {
        Account {
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            username: username.into(),
            sent_folder: "Sent".into(),
            drafts_folder: "Drafts".into(),
            archive_folder: "Archive".into(),
            trash_folder: "Trash".into(),
        }
    }

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
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let secret = encrypt_password("hunter2").unwrap();
        assert_eq!(decrypt_password(&secret).unwrap(), "hunter2");
    }

    #[test]
    fn save_load_credentials_round_trip() {
        with_config_backup(|_| {
            save(&test_account("test@example.com"), "hunter2").unwrap();

            let accounts = load_all().unwrap();
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0].account.username, "test@example.com");
            assert!(!accounts[0].expired);

            let (creds_account, password) = credentials("test@example.com").unwrap();
            assert_eq!(creds_account.username, "test@example.com");
            assert_eq!(password, "hunter2");
        });
    }

    #[test]
    fn multiple_accounts_keep_separate_credentials() {
        with_config_backup(|_| {
            save(&test_account("one@example.com"), "pw-one").unwrap();
            save(&test_account("two@example.com"), "pw-two").unwrap();

            let usernames: Vec<String> = load_all().unwrap().into_iter().map(|a| a.account.username).collect();
            assert_eq!(usernames, vec!["one@example.com", "two@example.com"]);
            assert_eq!(credentials("one@example.com").unwrap().1, "pw-one");
            assert_eq!(credentials("two@example.com").unwrap().1, "pw-two");
        });
    }

    #[test]
    fn saving_existing_username_replaces_it_in_place() {
        with_config_backup(|_| {
            save(&test_account("one@example.com"), "old").unwrap();
            save(&test_account("two@example.com"), "pw-two").unwrap();
            let mut updated = test_account("one@example.com");
            updated.imap_host = "imap.new.example.com".into();
            save(&updated, "new").unwrap();

            let accounts = load_all().unwrap();
            assert_eq!(accounts.len(), 2);
            assert_eq!(accounts[0].account.username, "one@example.com");
            assert_eq!(accounts[0].account.imap_host, "imap.new.example.com");
            assert_eq!(credentials("one@example.com").unwrap().1, "new");
        });
    }

    #[test]
    fn remove_drops_only_that_account() {
        with_config_backup(|_| {
            save(&test_account("one@example.com"), "pw-one").unwrap();
            save(&test_account("two@example.com"), "pw-two").unwrap();
            remove("one@example.com").unwrap();

            let usernames: Vec<String> = load_all().unwrap().into_iter().map(|a| a.account.username).collect();
            assert_eq!(usernames, vec!["two@example.com"]);
            assert!(credentials("one@example.com").is_err());
        });
    }

    #[test]
    fn legacy_single_account_config_still_loads() {
        // Configs written before multi-account support are one flat account.
        with_config_backup(|path| {
            let legacy = StoredAccount {
                account: test_account("legacy@example.com"),
                encrypted_password: Some(encrypt_password("hunter2").unwrap()),
                saved_at: now(),
            };
            fs::write(path, serde_yaml::to_string(&legacy).unwrap()).unwrap();

            let accounts = load_all().unwrap();
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0].account.username, "legacy@example.com");
            assert!(!accounts[0].expired);
            assert_eq!(credentials("legacy@example.com").unwrap().1, "hunter2");

            // ...and gets upgraded to the list form once another is added.
            save(&test_account("new@example.com"), "pw").unwrap();
            assert_eq!(load_all().unwrap().len(), 2);
            assert!(fs::read_to_string(path).unwrap().starts_with("accounts:"));
        });
    }

    #[test]
    fn hand_written_readme_config_loads_as_expired() {
        // The README's example config has no stored password yet.
        with_config_backup(|path| {
            fs::write(path, "imap_host: imap.example.com\nimap_port: 993\nsmtp_host: smtp.example.com\nsmtp_port: 465\nusername: me@example.com\n").unwrap();
            let accounts = load_all().unwrap();
            assert_eq!(accounts.len(), 1);
            assert!(accounts[0].expired);
        });
    }

    #[test]
    fn stale_saved_at_counts_as_expired() {
        with_config_backup(|path| {
            let stored = StoredAccount {
                account: test_account("old@example.com"),
                encrypted_password: Some(encrypt_password("hunter2").unwrap()),
                saved_at: now() - SESSION_LIFETIME_SECS - 1,
            };
            write_config(path, &StoredConfig { accounts: vec![stored] }).unwrap();
            assert!(load_all().unwrap()[0].expired);
        });
    }

    #[test]
    fn missing_password_counts_as_expired_and_has_no_credentials() {
        // A config left over from a version that didn't store a password at
        // all (or one that failed to save it) must count as expired, since
        // there's nothing to decrypt and log back in with.
        with_config_backup(|path| {
            let stored = StoredAccount { account: test_account("old@example.com"), encrypted_password: None, saved_at: now() };
            write_config(path, &StoredConfig { accounts: vec![stored] }).unwrap();
            assert!(load_all().unwrap()[0].expired);
            assert!(credentials("old@example.com").is_err());
        });
    }

    #[test]
    fn no_config_file_means_no_accounts() {
        with_config_backup(|_| {
            assert!(load_all().unwrap().is_empty());
            assert!(credentials("anyone@example.com").is_err());
        });
    }
}
