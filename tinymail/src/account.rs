use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "tinymail";
const OLD_KEYRING_SERVICE: &str = "email-client";

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
}

fn default_sent() -> String {
    "Sent".into()
}
fn default_drafts() -> String {
    "Drafts".into()
}

fn config_path() -> Result<PathBuf, String> {
    let mut path = dirs::home_dir().ok_or("could not resolve home dir")?;
    path.push("tinymail.yml");
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

fn old_json_path() -> Result<PathBuf, String> {
    let mut dir = dirs::config_dir().ok_or("could not resolve config dir")?;
    dir.push("email-client");
    Ok(dir.join("account.json"))
}

// The stored file never keeps a plaintext password on disk under normal
// operation — `password` is only ever populated when a user hand-writes the
// file to bootstrap an account (see `save`, which always writes it back out
// as None once the real secret has been moved into the keychain).
#[derive(Serialize, Deserialize)]
struct StoredAccount {
    #[serde(flatten)]
    account: Account,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    password: Option<String>,
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

    let json_path = old_json_path()?;
    if json_path.exists() {
        let json = fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
        let account: Account = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        write_stored(path, &StoredAccount { account, password: None })?;
        let _ = fs::remove_file(&json_path);
    }
    Ok(())
}

fn write_stored(path: &PathBuf, stored: &StoredAccount) -> Result<(), String> {
    let yaml = serde_yaml::to_string(stored).map_err(|e| e.to_string())?;
    fs::write(path, yaml).map_err(|e| e.to_string())
}

pub fn save(account: &Account, password: &str) -> Result<(), String> {
    // Write the password first: if the keychain rejects it, we must not persist
    // a config that get_account() would return forever with no matching secret.
    keyring::Entry::new(KEYRING_SERVICE, &account.username)
        .map_err(|e| e.to_string())?
        .set_password(password)
        .map_err(|e| e.to_string())?;

    let path = config_path()?;
    write_stored(&path, &StoredAccount { account: account.clone(), password: None })
}

pub fn load() -> Result<Option<Account>, String> {
    let path = config_path()?;
    if !path.exists() {
        migrate_old_config_if_present(&path)?;
    }
    if !path.exists() {
        return Ok(None);
    }

    let yaml = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let stored: StoredAccount = serde_yaml::from_str(&yaml).map_err(|e| e.to_string())?;

    // Hand-written bootstrap file: move the password into the keychain and
    // rewrite the file without it before returning.
    if let Some(password) = stored.password {
        save(&stored.account, &password)?;
        return Ok(Some(stored.account));
    }

    // Self-heal only when the keychain has genuinely no entry for this account
    // (e.g. left over from an old failed save). A transient error — the keychain
    // being locked, or a still-pending "Allow access" prompt — must NOT wipe out
    // an otherwise-valid config; that would look like the account vanished.
    match raw_password(&stored.account.username) {
        Err(keyring::Error::NoEntry) => {
            let _ = fs::remove_file(&path);
            Ok(None)
        }
        Err(e) => Err(e.to_string()),
        Ok(_) => Ok(Some(stored.account)),
    }
}

fn raw_password(username: &str) -> Result<String, keyring::Error> {
    match keyring::Entry::new(KEYRING_SERVICE, username)?.get_password() {
        Ok(password) => Ok(password),
        // Migrate a credential saved under the old service name (pre-rename)
        // into the new one, then clean up the old entry.
        Err(keyring::Error::NoEntry) => {
            let old_entry = keyring::Entry::new(OLD_KEYRING_SERVICE, username)?;
            let password = old_entry.get_password()?;
            keyring::Entry::new(KEYRING_SERVICE, username)?.set_password(&password)?;
            let _ = old_entry.delete_credential();
            Ok(password)
        }
        Err(e) => Err(e),
    }
}

pub fn password(username: &str) -> Result<String, String> {
    raw_password(username).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_bootstrap_round_trip() {
        let path = config_path().unwrap();
        let _ = fs::remove_file(&path);
        let _ = keyring::Entry::new(KEYRING_SERVICE, "test@example.com")
            .unwrap()
            .delete_credential();

        fs::write(
            &path,
            "username: test@example.com\nimap_host: imap.example.com\nimap_port: 993\nsmtp_host: smtp.example.com\nsmtp_port: 465\npassword: hunter2\n",
        )
        .unwrap();

        let account = load().expect("load should not error").expect("account should be imported");
        assert_eq!(account.username, "test@example.com");
        assert_eq!(password("test@example.com").unwrap(), "hunter2");

        // password must have been stripped from disk after import
        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("hunter2"));

        let _ = fs::remove_file(&path);
        let _ = keyring::Entry::new(KEYRING_SERVICE, "test@example.com")
            .unwrap()
            .delete_credential();
    }

    #[test]
    fn keyring_service_migration() {
        let _ = keyring::Entry::new(KEYRING_SERVICE, "migrate-svc@example.com")
            .unwrap()
            .delete_credential();
        keyring::Entry::new(OLD_KEYRING_SERVICE, "migrate-svc@example.com")
            .unwrap()
            .set_password("oldsecret")
            .unwrap();

        assert_eq!(password("migrate-svc@example.com").unwrap(), "oldsecret");
        // now readable under the new service, and gone from the old one
        assert_eq!(
            keyring::Entry::new(KEYRING_SERVICE, "migrate-svc@example.com")
                .unwrap()
                .get_password()
                .unwrap(),
            "oldsecret"
        );
        assert!(keyring::Entry::new(OLD_KEYRING_SERVICE, "migrate-svc@example.com")
            .unwrap()
            .get_password()
            .is_err());

        let _ = keyring::Entry::new(KEYRING_SERVICE, "migrate-svc@example.com")
            .unwrap()
            .delete_credential();
    }
}
