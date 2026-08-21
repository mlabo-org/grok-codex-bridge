use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration as StdDuration, Instant, SystemTime};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use thiserror::Error;
use zeroize::Zeroize;

const XAI_SCOPE_PREFIX: &str = "https://auth.x.ai::";
const XAI_ISSUER: &str = "https://auth.x.ai";
const TOKEN_TTL: Duration = Duration::days(30);
const MAX_AUTH_FILE_BYTES: u64 = 1024 * 1024;
const RENEWAL_POLL_INTERVAL: StdDuration = StdDuration::from_millis(250);
const OFFICIAL_REFRESH_TIMEOUT: StdDuration = StdDuration::from_secs(7);

pub struct CredentialStore {
    path: PathBuf,
    official_cli: PathBuf,
    cached: Mutex<Option<CachedCredential>>,
}

impl CredentialStore {
    pub fn from_environment() -> Result<Self, CredentialError> {
        let path = resolve_auth_path()?;
        let official_cli = resolve_official_cli_path(&path)?;
        Self::with_official_cli(path, official_cli)
    }

    pub fn new(path: PathBuf) -> Result<Self, CredentialError> {
        let official_cli = path
            .parent()
            .ok_or(CredentialError::RelativeAuthPath)?
            .join("bin/grok");
        Self::with_official_cli(path, official_cli)
    }

    fn with_official_cli(path: PathBuf, official_cli: PathBuf) -> Result<Self, CredentialError> {
        if !path.is_absolute() {
            return Err(CredentialError::RelativeAuthPath);
        }
        Ok(Self {
            path,
            official_cli,
            cached: Mutex::new(None),
        })
    }

    pub fn load(&self) -> Result<Arc<SessionCredential>, CredentialError> {
        let mut file = open_read_only(&self.path)?;
        let metadata = file.metadata().map_err(CredentialError::ReadAuth)?;
        validate_auth_metadata(&metadata)?;
        let fingerprint = FileFingerprint {
            len: metadata.len(),
            modified: metadata.modified().map_err(CredentialError::ReadAuth)?,
        };

        let mut cached = self
            .cached
            .lock()
            .map_err(|_| CredentialError::CacheUnavailable)?;
        if let Some(current) = cached
            .as_ref()
            .filter(|entry| entry.fingerprint == fingerprint)
        {
            current.credential.ensure_current()?;
            return Ok(Arc::clone(&current.credential));
        }

        let capacity = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        let mut raw = String::with_capacity(capacity);
        file.read_to_string(&mut raw)
            .map_err(CredentialError::ReadAuth)?;
        let parsed = parse_auth_map(&raw);
        raw.zeroize();
        let credential = Arc::new(parsed?);
        credential.ensure_current()?;
        *cached = Some(CachedCredential {
            fingerprint,
            credential: Arc::clone(&credential),
        });
        Ok(credential)
    }

    /// Waits briefly for the official Grok flow to replace a credential that
    /// expired while this long-lived bridge process was asleep.
    ///
    /// This method never refreshes, rewrites, or sends the expired credential
    /// itself. It delegates one headless refresh attempt to the official Grok
    /// helper, then repeats the same read-only load until the authoritative
    /// file is replaced or the bounded grace period ends.
    pub fn load_with_renewal_grace(
        &self,
        grace_period: StdDuration,
    ) -> Result<Arc<SessionCredential>, CredentialError> {
        let deadline = Instant::now() + grace_period;
        let mut refresh_attempted = false;
        loop {
            match self.load() {
                Ok(credential) => return Ok(credential),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if !matches!(error, CredentialError::ExpiredSessionCredential) {
                        return Err(error);
                    }
                    if remaining.is_zero() {
                        return Err(error);
                    }
                    if !refresh_attempted {
                        refresh_attempted = true;
                        let _ = run_official_refresh(&self.official_cli);
                        continue;
                    }
                    thread::sleep(RENEWAL_POLL_INTERVAL.min(remaining));
                }
            }
        }
    }
}

fn run_official_refresh(helper: &Path) -> bool {
    let Ok(mut child) = Command::new(helper)
        .arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    let deadline = Instant::now() + OFFICIAL_REFRESH_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(
                    RENEWAL_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn resolve_auth_path() -> Result<PathBuf, CredentialError> {
    if let Some(path) = nonempty_env("GROK_AUTH_PATH") {
        return absolute_path(path);
    }
    if let Some(home) = nonempty_env("GROK_HOME") {
        return Ok(absolute_path(home)?.join("auth.json"));
    }
    let home = nonempty_env("HOME").ok_or(CredentialError::HomeUnavailable)?;
    Ok(absolute_path(home)?.join(".grok/auth.json"))
}

fn resolve_official_cli_path(auth_path: &Path) -> Result<PathBuf, CredentialError> {
    if let Some(home) = nonempty_env("GROK_HOME") {
        return Ok(absolute_path(home)?.join("bin/grok"));
    }
    if let Some(home) = nonempty_env("HOME") {
        return Ok(absolute_path(home)?.join(".grok/bin/grok"));
    }
    auth_path
        .parent()
        .map(|parent| parent.join("bin/grok"))
        .ok_or(CredentialError::RelativeAuthPath)
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn absolute_path(value: String) -> Result<PathBuf, CredentialError> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(CredentialError::RelativeAuthPath);
    }
    Ok(path)
}

#[cfg(unix)]
fn open_read_only(path: &Path) -> Result<fs::File, CredentialError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(CredentialError::ReadAuth)
}

#[cfg(not(unix))]
fn open_read_only(_path: &Path) -> Result<fs::File, CredentialError> {
    Err(CredentialError::UnsupportedPermissionPlatform)
}

#[cfg(unix)]
fn validate_auth_metadata(metadata: &fs::Metadata) -> Result<(), CredentialError> {
    use std::os::unix::fs::PermissionsExt;

    if !metadata.is_file() {
        return Err(CredentialError::UnsafeAuthFileType);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(CredentialError::UnsafeAuthPermissions);
    }
    if metadata.len() == 0 || metadata.len() > MAX_AUTH_FILE_BYTES {
        return Err(CredentialError::InvalidAuthFileSize);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_auth_metadata(_metadata: &fs::Metadata) -> Result<(), CredentialError> {
    Err(CredentialError::UnsupportedPermissionPlatform)
}

fn parse_auth_map(source: &str) -> Result<SessionCredential, CredentialError> {
    let entries: BTreeMap<String, RawAuthRecord> =
        serde_json::from_str(source).map_err(CredentialError::ParseAuth)?;
    let candidates: Vec<_> = entries
        .iter()
        .filter(|(scope, record)| record.is_current_xai_session(scope))
        .collect();

    let [(scope, selected)] = candidates.as_slice() else {
        return Err(if candidates.is_empty() {
            CredentialError::SessionCredentialMissing
        } else {
            CredentialError::AmbiguousSessionCredential
        });
    };

    if selected.key.trim().is_empty() || selected.user_id.trim().is_empty() {
        return Err(CredentialError::InvalidSessionCredential);
    }
    let expires_at = selected
        .expires_at
        .unwrap_or(selected.create_time + TOKEN_TTL);
    let credential = SessionCredential {
        token: SecretString::new(selected.key.clone()),
        user_id: selected.user_id.clone(),
        scope: (*scope).clone(),
        expires_at,
    };
    credential.ensure_current()?;
    Ok(credential)
}

#[derive(Deserialize)]
struct RawAuthRecord {
    key: String,
    auth_mode: AuthMode,
    create_time: DateTime<Utc>,
    user_id: String,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    oidc_issuer: Option<String>,
}

impl RawAuthRecord {
    fn is_current_xai_session(&self, scope: &str) -> bool {
        scope.starts_with(XAI_SCOPE_PREFIX)
            && matches!(self.auth_mode, AuthMode::Oidc | AuthMode::External)
            && self
                .oidc_issuer
                .as_deref()
                .is_none_or(|issuer| issuer == XAI_ISSUER)
    }
}

impl Drop for RawAuthRecord {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AuthMode {
    Oidc,
    External,
    #[serde(other)]
    Unsupported,
}

pub struct SessionCredential {
    token: SecretString,
    user_id: String,
    scope: String,
    expires_at: DateTime<Utc>,
}

impl SessionCredential {
    pub(crate) fn token(&self) -> &str {
        self.token.expose()
    }

    pub(crate) fn user_id(&self) -> &str {
        &self.user_id
    }

    fn ensure_current(&self) -> Result<(), CredentialError> {
        if self.expires_at <= Utc::now() {
            return Err(CredentialError::ExpiredSessionCredential);
        }
        Ok(())
    }
}

impl fmt::Debug for SessionCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCredential")
            .field("token", &"[REDACTED]")
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl Drop for SessionCredential {
    fn drop(&mut self) {
        self.user_id.zeroize();
        self.scope.zeroize();
    }
}

struct SecretString(String);

impl SecretString {
    fn new(value: String) -> Self {
        Self(value)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileFingerprint {
    len: u64,
    modified: SystemTime,
}

struct CachedCredential {
    fingerprint: FileFingerprint,
    credential: Arc<SessionCredential>,
}

#[cfg(test)]
pub(crate) fn test_session_credential(token: &str, user_id: &str) -> SessionCredential {
    SessionCredential {
        token: SecretString::new(token.to_owned()),
        user_id: user_id.to_owned(),
        scope: format!("{XAI_SCOPE_PREFIX}test-client"),
        expires_at: DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    }
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("Grok auth file path must be absolute")]
    RelativeAuthPath,
    #[error("HOME is unavailable and no Grok auth path was configured")]
    HomeUnavailable,
    #[error("failed to read Grok auth file")]
    ReadAuth(#[source] std::io::Error),
    #[error("Grok auth file must be a regular non-symlink file")]
    UnsafeAuthFileType,
    #[error("Grok auth file must not be accessible by group or other users")]
    UnsafeAuthPermissions,
    #[error("Grok auth file size is invalid")]
    InvalidAuthFileSize,
    #[error("Grok auth file is not valid JSON")]
    ParseAuth(#[source] serde_json::Error),
    #[error("one current xAI session credential was not found; run the official Grok login flow")]
    SessionCredentialMissing,
    #[error(
        "multiple current xAI session credentials were found; select one official Grok auth file"
    )]
    AmbiguousSessionCredential,
    #[error("the selected xAI session credential is incomplete")]
    InvalidSessionCredential,
    #[error("the selected xAI session credential is expired; run the official Grok login flow")]
    ExpiredSessionCredential,
    #[error("credential memory cache is unavailable")]
    CacheUnavailable,
    #[cfg(not(unix))]
    #[error("credential permission validation is unsupported on this platform")]
    UnsupportedPermissionPlatform,
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn write_auth(path: &Path, records: &str, mode: u32) {
        fs::write(path, records).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn write_helper(path: &Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn record(token: &str, expires_at: &str) -> String {
        format!(
            r#"{{"https://auth.x.ai::current-client":{{"key":"{token}","auth_mode":"oidc","create_time":"2026-08-01T00:00:00Z","user_id":"user-1","expires_at":"{expires_at}","oidc_issuer":"https://auth.x.ai"}}}}"#
        )
    }

    #[test]
    fn reads_one_private_current_session_and_reloads_changed_file() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("auth.json");
        write_auth(
            &path,
            &record("first-secret", "2099-01-01T00:00:00Z"),
            0o600,
        );
        let store = CredentialStore::new(path.clone()).unwrap();
        let first = store.load().unwrap();
        assert_eq!(first.token(), "first-secret");

        write_auth(
            &path,
            &record("second-secret-longer", "2099-01-01T00:00:00Z"),
            0o600,
        );
        let second = store.load().unwrap();
        assert_eq!(second.token(), "second-secret-longer");
    }

    #[test]
    fn rejects_group_readable_auth_file() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("auth.json");
        write_auth(&path, &record("secret", "2099-01-01T00:00:00Z"), 0o640);
        let store = CredentialStore::new(path).unwrap();
        assert!(matches!(
            store.load(),
            Err(CredentialError::UnsafeAuthPermissions)
        ));
    }

    #[test]
    fn rejects_expired_and_ambiguous_session_credentials() {
        assert!(matches!(
            parse_auth_map(&record("secret", "2020-01-01T00:00:00Z")),
            Err(CredentialError::ExpiredSessionCredential)
        ));
        let ambiguous = r#"{
            "https://auth.x.ai::client-a":{"key":"a","auth_mode":"oidc","create_time":"2026-01-01T00:00:00Z","user_id":"user-a","expires_at":"2099-01-01T00:00:00Z"},
            "https://auth.x.ai::client-b":{"key":"b","auth_mode":"external","create_time":"2026-01-01T00:00:00Z","user_id":"user-b","expires_at":"2099-01-01T00:00:00Z","oidc_issuer":"https://auth.x.ai"}
        }"#;
        assert!(matches!(
            parse_auth_map(ambiguous),
            Err(CredentialError::AmbiguousSessionCredential)
        ));
    }

    #[test]
    fn expired_credential_waits_for_official_file_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("auth.json");
        write_auth(&path, &record("expired", "2020-01-01T00:00:00Z"), 0o600);
        let store = CredentialStore::new(path.clone()).unwrap();

        let replacement_path = temporary.path().join("auth.json.renewed");
        let renew = thread::spawn(move || {
            thread::sleep(StdDuration::from_millis(25));
            write_auth(
                &replacement_path,
                &record("renewed", "2099-01-01T00:00:00Z"),
                0o600,
            );
            fs::rename(replacement_path, path).unwrap();
        });

        let credential = store
            .load_with_renewal_grace(StdDuration::from_secs(1))
            .unwrap();
        renew.join().unwrap();
        assert_eq!(credential.token(), "renewed");
    }

    #[test]
    fn expired_credential_adopts_official_helper_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let auth_home = temporary.path();
        let path = auth_home.join("auth.json");
        let helper = auth_home.join("bin/grok");
        let invocation = auth_home.join("invocation");
        fs::create_dir(auth_home.join("bin")).unwrap();
        write_auth(&path, &record("expired", "2020-01-01T00:00:00Z"), 0o600);
        write_helper(
            &helper,
            &format!(
                "helper_dir=$(dirname \"$0\")\nauth_path=\"$helper_dir/../auth.json\"\nprintf '%s' \"$1\" > \"{}\"\nprintf '%s' '{}' > \"$auth_path.tmp\"\nchmod 600 \"$auth_path.tmp\"\nmv \"$auth_path.tmp\" \"$auth_path\"",
                invocation.display(),
                record("renewed", "2099-01-01T00:00:00Z")
            ),
        );

        let credential = CredentialStore::new(path)
            .unwrap()
            .load_with_renewal_grace(StdDuration::from_secs(1))
            .unwrap();
        assert_eq!(credential.token(), "renewed");
        assert_eq!(fs::read_to_string(invocation).unwrap(), "models");
    }

    #[test]
    fn failed_official_helper_preserves_expired_error() {
        let temporary = tempfile::tempdir().unwrap();
        let auth_home = temporary.path();
        let path = auth_home.join("auth.json");
        let helper = auth_home.join("bin/grok");
        fs::create_dir(auth_home.join("bin")).unwrap();
        write_auth(&path, &record("expired", "2020-01-01T00:00:00Z"), 0o600);
        write_helper(&helper, "exit 7");

        assert!(matches!(
            CredentialStore::new(path)
                .unwrap()
                .load_with_renewal_grace(StdDuration::from_secs(1)),
            Err(CredentialError::ExpiredSessionCredential)
        ));
    }

    #[test]
    fn official_helper_is_not_attempted_for_non_expiry_errors() {
        let current_record = record("current", "2099-01-01T00:00:00Z");
        let cases = [
            ("missing", None, None),
            ("malformed", Some("not-json"), None),
            ("current", Some(current_record.as_str()), Some("current")),
        ];

        for (name, contents, expected_token) in cases {
            let temporary = tempfile::tempdir().unwrap();
            let auth_home = temporary.path();
            let path = auth_home.join("auth.json");
            let helper = auth_home.join("bin/grok");
            let marker = auth_home.join("attempted");
            fs::create_dir(auth_home.join("bin")).unwrap();
            write_helper(&helper, &format!("touch \"{}\"", marker.display()));
            if let Some(contents) = contents {
                write_auth(&path, contents, 0o600);
            }

            let result = CredentialStore::new(path)
                .unwrap()
                .load_with_renewal_grace(StdDuration::from_millis(1));
            match expected_token {
                Some(token) => assert_eq!(result.unwrap().token(), token, "{name}"),
                None => assert!(result.is_err(), "{name}"),
            }
            assert!(!marker.exists(), "{name}");
        }
    }
}
