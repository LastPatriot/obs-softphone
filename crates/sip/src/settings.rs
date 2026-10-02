// SPDX-License-Identifier: GPL-2.0-or-later
//! Plugin settings, stored as JSON in the module's global config directory
//! (never in the scene collection; DESIGN.md §6), written by the settings
//! dialog. The password lives in the OS secret store (macOS Keychain,
//! Windows Credential Manager, Linux Secret Service), not in the file,
//! unless the store isn't available.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use softphone_core::LineConfig;

use crate::SipConfig;

/// Which kind of server the settings were filled in for. Only changes
/// defaults and hints in the settings dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    TreasureStream,
    Generic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Udp,
    Tcp,
    Tls,
}

impl Transport {
    pub fn default_port(self) -> u16 {
        if self == Transport::Tls { 5061 } else { 5060 }
    }
}

/// Encrypted media (SRTP, SDES keys in the SDP).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Srtp {
    Off,
    Optional,
    Required,
}

// Files written before these fields existed were Treasure Stream over TLS.
fn legacy_preset() -> Preset {
    Preset::TreasureStream
}
fn legacy_transport() -> Transport {
    Transport::Tls
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(default = "legacy_preset")]
    pub preset: Preset,
    pub enabled: bool,
    /// Registrar host, e.g. `sip.example.com`.
    pub server: String,
    pub port: u16,
    #[serde(default = "legacy_transport")]
    pub transport: Transport,
    /// SIP user (the extension or account name).
    pub username: String,
    /// Authentication user, if different from `username`.
    pub auth_username: String,
    /// The account's domain (`sip:user@domain`), if different from `server`.
    pub domain: String,
    /// Outbound proxy, `host[:port]`, if the provider needs one.
    pub outbound_proxy: String,
    pub srtp: Srtp,
    /// STUN server, `host[:port]`, if the network needs one for media.
    pub stun_server: String,
    /// Kept in the [`SecretStore`]; only written to the file as a fallback.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
    pub verify_tls: bool,
    /// CA bundle; defaults to the system bundle.
    pub ca_file: Option<PathBuf>,
    pub auto_answer: bool,
    pub auto_answer_delay_ms: u64,
    /// Warn when another device takes the extension (§4.4). The bindings
    /// query it relies on was verified against the station's Asterisk.
    pub replaced_detection: bool,
    /// pjlib log level 0..=6.
    pub log_level: i32,
    /// OBS audio track (1–6) the caller hears: everything except the caller
    /// (mix-minus, DESIGN.md §3.3).
    pub return_track: u8,
    /// A quiet chime on the computer (never on air) when a call rings.
    pub ring_chime: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preset: Preset::Generic,
            enabled: true,
            server: String::new(),
            port: 5060,
            transport: Transport::Udp,
            username: String::new(),
            auth_username: String::new(),
            domain: String::new(),
            outbound_proxy: String::new(),
            srtp: Srtp::Off,
            stun_server: String::new(),
            password: String::new(),
            verify_tls: true,
            ca_file: None,
            auto_answer: true,
            auto_answer_delay_ms: 300,
            replaced_detection: true,
            log_level: 3,
            return_track: 2,
            ring_chime: true,
        }
    }
}

/// Where the password is kept. [`OsKeyring`] in the plugin; tests use a
/// fake.
pub trait SecretStore {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The OS secret store, under the service name `obs-softphone`.
pub struct OsKeyring;

const KEYRING_SERVICE: &str = "obs-softphone";

impl SecretStore for OsKeyring {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())?;
        entry.set_password(secret).map_err(|e| e.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl Settings {
    /// The secret store's account name, e.g. `101@radio.example.org`.
    pub fn account(&self) -> String {
        format!("{}@{}", self.username.trim(), self.server.trim())
    }

    /// Loads the file (writing defaults if it doesn't exist) and the
    /// password from `store`. A password still in the file is moved into the
    /// store. Returns notes worth logging (e.g. why the store wasn't used).
    pub fn load(path: &Path, store: &dyn SecretStore) -> Result<(Self, Vec<String>), String> {
        let mut settings: Settings = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let settings = Self::default();
                settings.write_file(path, false)?;
                return Ok((settings, Vec::new()));
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut notes = Vec::new();
        if !settings.password.is_empty() {
            // Older file (or a store that failed before): move it over.
            if let Some(note) = settings.save(path, store, None)? {
                notes.push(note);
            } else {
                notes.push("moved the password from config.json to the system's secret store".into());
            }
        } else if !settings.server.trim().is_empty() {
            match store.get(&settings.account()) {
                Ok(Some(p)) => settings.password = p,
                Ok(None) => {}
                Err(e) => notes.push(format!("can't read the password from the secret store: {e}")),
            }
        }
        Ok((settings, notes))
    }

    /// Saves the file and the password. `previous` is what was saved before,
    /// so a changed server or extension doesn't leave an old entry behind.
    /// Returns a note if the password had to stay in the file.
    pub fn save(&self, path: &Path, store: &dyn SecretStore, previous: Option<&Settings>) -> Result<Option<String>, String> {
        if let Some(prev) = previous.filter(|p| p.account() != self.account() && !p.server.trim().is_empty()) {
            let _ = store.delete(&prev.account());
        }
        let in_store = if self.password.is_empty() {
            let _ = store.delete(&self.account());
            Ok(())
        } else {
            store.set(&self.account(), &self.password)
        };
        match in_store {
            Ok(()) => {
                self.write_file(path, false)?;
                Ok(None)
            }
            Err(e) => {
                self.write_file(path, true)?;
                Ok(Some(format!("secret store unavailable ({e}); the password is kept in config.json")))
            }
        }
    }

    /// Writes the file (readable by the user only), with or without the
    /// password.
    fn write_file(&self, path: &Path, with_password: bool) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let mut copy = self.clone();
        if !with_password {
            copy.password.clear();
        }
        let json = serde_json::to_string_pretty(&copy).expect("settings serialize");
        std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        restrict_permissions(path);
        Ok(())
    }

    /// 0-based mix index for libobs; out-of-range values fall back to Track 2.
    pub fn return_mix_index(&self) -> usize {
        match self.return_track {
            1..=6 => usize::from(self.return_track) - 1,
            _ => 1,
        }
    }

    /// True once there is enough to try registering.
    pub fn is_configured(&self) -> bool {
        !self.server.trim().is_empty() && !self.username.trim().is_empty() && !self.password.is_empty()
    }

    pub fn sip_config(&self) -> SipConfig {
        let or = |v: &str, fallback: &str| if v.trim().is_empty() { fallback.trim().to_string() } else { v.trim().to_string() };
        SipConfig {
            server: self.server.trim().to_string(),
            port: self.port,
            transport: self.transport,
            username: self.username.trim().to_string(),
            auth_username: or(&self.auth_username, &self.username),
            domain: or(&self.domain, &self.server),
            outbound_proxy: self.outbound_proxy.trim().to_string(),
            srtp: self.srtp,
            stun_server: self.stun_server.trim().to_string(),
            password: self.password.clone(),
            verify_tls: self.verify_tls,
            ca_file: self.ca_file.clone(),
            log_level: self.log_level,
        }
    }

    pub fn line_config(&self) -> LineConfig {
        LineConfig {
            auto_answer: self.auto_answer,
            auto_answer_delay: Duration::from_millis(self.auto_answer_delay_ms),
            replaced_detection: self.replaced_detection,
            ..LineConfig::default()
        }
    }
}

/// The file holds the SIP password: readable by the user only.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_are_generic_udp() {
        let s = Settings::default();
        assert_eq!((s.preset, s.transport, s.port), (Preset::Generic, Transport::Udp, 5060));
        assert!(!s.is_configured());
    }

    #[test]
    fn old_files_keep_meaning_treasure_stream_over_tls() {
        let s: Settings =
            serde_json::from_str(r#"{"server":"radio.example","port":5061,"username":"101","password":"x"}"#).unwrap();
        assert_eq!((s.preset, s.transport), (Preset::TreasureStream, Transport::Tls));
        assert!(s.is_configured());
        let c = s.sip_config();
        assert_eq!((c.domain.as_str(), c.auth_username.as_str()), ("radio.example", "101"));
    }

    #[test]
    fn optional_fields_override_defaults() {
        let s = Settings {
            server: " sip.provider.net ".into(),
            username: "4415550100".into(),
            auth_username: "acct77".into(),
            domain: "provider.net".into(),
            ..Settings::default()
        };
        let c = s.sip_config();
        assert_eq!(c.server, "sip.provider.net");
        assert_eq!(c.auth_username, "acct77");
        assert_eq!(c.domain, "provider.net");
    }

    #[test]
    fn enums_use_readable_names() {
        let s = Settings { transport: Transport::Tcp, srtp: Srtp::Required, ..Settings::default() };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!((v["transport"].as_str(), v["srtp"].as_str(), v["preset"].as_str()), (Some("tcp"), Some("required"), Some("generic")));
    }

    #[test]
    fn partial_file_uses_defaults() {
        let s: Settings = serde_json::from_str(r#"{"server":"radio.example","password":"x","username":"101"}"#).unwrap();
        assert_eq!(s.port, 5060, "port: new default (old files always wrote it)");
        assert_eq!(s.username, "101");
        assert!(s.verify_tls && s.auto_answer && s.enabled);
        assert!(s.replaced_detection);
        assert_eq!(s.return_track, 2);
        assert_eq!(s.return_mix_index(), 1);
        assert_eq!(s.line_config().auto_answer_delay, Duration::from_millis(300));
    }

    #[test]
    fn bad_return_track_falls_back_to_track_2() {
        for t in [0, 7, 255] {
            let s = Settings { return_track: t, ..Settings::default() };
            assert_eq!(s.return_mix_index(), 1);
        }
        let s = Settings { return_track: 6, ..Settings::default() };
        assert_eq!(s.return_mix_index(), 5);
    }

    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemStore(RefCell<HashMap<String, String>>);
    impl SecretStore for MemStore {
        fn get(&self, a: &str) -> Result<Option<String>, String> {
            Ok(self.0.borrow().get(a).cloned())
        }
        fn set(&self, a: &str, s: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(a.into(), s.into());
            Ok(())
        }
        fn delete(&self, a: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(a);
            Ok(())
        }
    }

    struct BrokenStore;
    impl SecretStore for BrokenStore {
        fn get(&self, _: &str) -> Result<Option<String>, String> {
            Err("no keyring".into())
        }
        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            Err("no keyring".into())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Err("no keyring".into())
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("sp-settings-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            Self(d)
        }
        fn file(&self) -> PathBuf {
            self.0.join("nested/config.json")
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn file_json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn configured() -> Settings {
        Settings { server: "radio.example".into(), username: "101".into(), password: "s3cret".into(), ..Settings::default() }
    }

    #[test]
    fn creates_template_when_missing() {
        let dir = TempDir::new("template");
        let (s, notes) = Settings::load(&dir.file(), &MemStore::default()).unwrap();
        assert_eq!(s, Settings::default());
        assert!(notes.is_empty());
        assert!(dir.file().is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.file()).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn save_keeps_the_password_out_of_the_file() {
        let dir = TempDir::new("save");
        let store = MemStore::default();
        assert_eq!(configured().save(&dir.file(), &store, None).unwrap(), None);
        assert!(file_json(&dir.file()).get("password").is_none());
        assert_eq!(store.get("101@radio.example").unwrap().as_deref(), Some("s3cret"));

        let (loaded, notes) = Settings::load(&dir.file(), &store).unwrap();
        assert_eq!(loaded, configured());
        assert!(notes.is_empty());
    }

    #[test]
    fn password_in_an_old_file_is_moved_to_the_store() {
        let dir = TempDir::new("migrate");
        std::fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
        std::fs::write(dir.file(), r#"{"server":"radio.example","username":"101","password":"s3cret"}"#).unwrap();
        let store = MemStore::default();
        let (loaded, notes) = Settings::load(&dir.file(), &store).unwrap();
        assert_eq!(loaded.password, "s3cret");
        assert!(notes[0].contains("moved"));
        assert!(file_json(&dir.file()).get("password").is_none());
        assert_eq!(store.get("101@radio.example").unwrap().as_deref(), Some("s3cret"));
    }

    #[test]
    fn broken_store_falls_back_to_the_file() {
        let dir = TempDir::new("broken");
        let note = configured().save(&dir.file(), &BrokenStore, None).unwrap();
        assert!(note.unwrap().contains("kept in config.json"));
        assert_eq!(file_json(&dir.file())["password"], "s3cret");

        let (loaded, notes) = Settings::load(&dir.file(), &BrokenStore).unwrap();
        assert_eq!(loaded.password, "s3cret");
        assert!(notes[0].contains("kept in config.json"));
    }

    #[test]
    fn changing_server_or_extension_removes_the_old_entry() {
        let dir = TempDir::new("rename");
        let store = MemStore::default();
        let old = configured();
        old.save(&dir.file(), &store, None).unwrap();
        let new = Settings { username: "102".into(), ..configured() };
        new.save(&dir.file(), &store, Some(&old)).unwrap();
        assert_eq!(store.get("101@radio.example").unwrap(), None);
        assert_eq!(store.get("102@radio.example").unwrap().as_deref(), Some("s3cret"));
    }

    #[test]
    fn clearing_the_password_deletes_it() {
        let dir = TempDir::new("clear");
        let store = MemStore::default();
        configured().save(&dir.file(), &store, None).unwrap();
        let cleared = Settings { password: String::new(), ..configured() };
        cleared.save(&dir.file(), &store, Some(&configured())).unwrap();
        assert_eq!(store.get("101@radio.example").unwrap(), None);
    }
}
