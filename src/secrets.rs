//! Secure storage for user secrets (currently just the Anthropic API
//! key) backed by the OS keychain. On macOS this is the login
//! Keychain via Security.framework; the `keyring` crate is configured
//! with the `apple-native` feature in Cargo.toml.
//!
//! Previously the API key lived in plaintext at
//! `~/.launchpad/settings.json`. That file is scrubbed on load (see
//! `settings::AppSettings::load_or_default`) — users re-enter the key
//! once, and it's persisted here instead.

use std::io;

const SERVICE: &str = "dev.launchpad.Launchpad";
const ACCOUNT: &str = "anthropic_api_key";

fn entry() -> Option<keyring::Entry> {
    match keyring::Entry::new(SERVICE, ACCOUNT) {
        Ok(e) => Some(e),
        Err(err) => {
            eprintln!("[launchpad] keychain entry unavailable: {err}");
            None
        }
    }
}

/// Read the stored API key, or `None` if no entry exists or the
/// keychain is unavailable. A "not found" result is indistinguishable
/// from a hard error at the caller — both mean "we don't have a key."
pub fn get_api_key() -> Option<String> {
    let entry = entry()?;
    match entry.get_password() {
        Ok(key) => Some(key),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            eprintln!("[launchpad] keychain read failed: {err}");
            None
        }
    }
}

/// Store (or overwrite) the API key in the keychain.
pub fn set_api_key(key: &str) -> io::Result<()> {
    let entry = entry().ok_or_else(|| io::Error::other("keychain unavailable"))?;
    entry
        .set_password(key)
        .map_err(|e| io::Error::other(e.to_string()))
}

/// Remove the stored API key. A missing entry is treated as success
/// — the post-condition "there is no stored key" is the same either
/// way.
pub fn delete_api_key() -> io::Result<()> {
    let entry = entry().ok_or_else(|| io::Error::other("keychain unavailable"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(io::Error::other(err.to_string())),
    }
}
