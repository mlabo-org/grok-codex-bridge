use std::fmt;
use std::fs;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use zeroize::Zeroize;

const CONFIG_VERSION: u32 = 1;
const MIN_CAPABILITY_BYTES: usize = 32;
const MAX_CAPABILITY_BYTES: usize = 128;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    version: u32,
    server: ServerConfig,
    grok: GrokFileConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    bind: SocketAddr,
    capability_token_file: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrokFileConfig {
    catalog_cache_file: PathBuf,
    #[serde(default = "default_refresh_on_start")]
    refresh_on_start: bool,
}

fn default_refresh_on_start() -> bool {
    true
}

pub struct RuntimeConfig {
    bind: SocketAddr,
    capability: CapabilityToken,
    grok: GrokConfig,
}

#[derive(Clone, Debug)]
pub struct GrokConfig {
    catalog_cache_file: PathBuf,
    refresh_on_start: bool,
}

impl RuntimeConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let source = fs::read_to_string(path).map_err(ConfigError::ReadConfig)?;
        let file: FileConfig = toml::from_str(&source).map_err(ConfigError::ParseConfig)?;

        if file.version != CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion(file.version));
        }
        if !file.server.bind.ip().is_loopback() {
            return Err(ConfigError::NonLoopbackBind);
        }
        if file.server.bind.port() == 0 {
            return Err(ConfigError::ZeroPort);
        }
        if !file.server.capability_token_file.is_absolute() {
            return Err(ConfigError::RelativeCapabilityPath);
        }
        if !file.grok.catalog_cache_file.is_absolute() {
            return Err(ConfigError::RelativeCatalogCachePath);
        }

        let capability = CapabilityToken::load(&file.server.capability_token_file)?;
        Ok(Self {
            bind: file.server.bind,
            capability,
            grok: GrokConfig {
                catalog_cache_file: file.grok.catalog_cache_file,
                refresh_on_start: file.grok.refresh_on_start,
            },
        })
    }

    #[must_use]
    pub fn bind(&self) -> SocketAddr {
        self.bind
    }

    #[must_use]
    pub fn grok(&self) -> &GrokConfig {
        &self.grok
    }

    pub(crate) fn into_server_parts(self) -> (SocketAddr, CapabilityToken) {
        (self.bind, self.capability)
    }
}

impl GrokConfig {
    #[must_use]
    pub fn catalog_cache_file(&self) -> &Path {
        &self.catalog_cache_file
    }

    #[must_use]
    pub fn refresh_on_start(&self) -> bool {
        self.refresh_on_start
    }

    /// The merged picker publishes Native routing state beside the Grok catalog so the
    /// runtime config remains a single source-owned V1 schema.
    #[must_use]
    pub fn native_route_file(&self) -> PathBuf {
        self.catalog_cache_file
            .parent()
            .expect("validated absolute catalog path must have a parent")
            .join("picker-native-route.json")
    }
}

pub(crate) struct CapabilityToken(String);

impl CapabilityToken {
    fn load(path: &Path) -> Result<Self, ConfigError> {
        let mut file = open_capability_file(path)?;
        let metadata = file.metadata().map_err(ConfigError::ReadCapability)?;
        if !metadata.is_file() {
            return Err(ConfigError::UnsafeCapabilityFileType);
        }

        validate_private_permissions(&metadata)?;

        let size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        if !(MIN_CAPABILITY_BYTES..=MAX_CAPABILITY_BYTES + 2).contains(&size) {
            return Err(ConfigError::InvalidCapability);
        }

        let mut raw = String::with_capacity(size);
        file.read_to_string(&mut raw)
            .map_err(ConfigError::ReadCapability)?;
        let parsed = Self::parse(raw.trim());
        raw.zeroize();
        parsed
    }

    pub(crate) fn parse(value: &str) -> Result<Self, ConfigError> {
        if !(MIN_CAPABILITY_BYTES..=MAX_CAPABILITY_BYTES).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ConfigError::InvalidCapability);
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let candidate = candidate.as_bytes();
        let mut difference = expected.len() ^ candidate.len();

        for (index, expected_byte) in expected.iter().enumerate() {
            difference |= usize::from(*expected_byte ^ candidate.get(index).copied().unwrap_or(0));
        }

        difference == 0
    }
}

#[cfg(unix)]
fn open_capability_file(path: &Path) -> Result<fs::File, ConfigError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(ConfigError::ReadCapability)
}

#[cfg(not(unix))]
fn open_capability_file(path: &Path) -> Result<fs::File, ConfigError> {
    fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(ConfigError::ReadCapability)
}

impl fmt::Debug for CapabilityToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CapabilityToken([REDACTED])")
    }
}

impl Drop for CapabilityToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(unix)]
fn validate_private_permissions(metadata: &fs::Metadata) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(ConfigError::UnsafeCapabilityPermissions);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_permissions(_metadata: &fs::Metadata) -> Result<(), ConfigError> {
    Err(ConfigError::UnsupportedPermissionPlatform)
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read bridge configuration")]
    ReadConfig(#[source] std::io::Error),
    #[error("bridge configuration is not valid TOML")]
    ParseConfig(#[source] toml::de::Error),
    #[error("unsupported bridge configuration version {0}")]
    UnsupportedVersion(u32),
    #[error("server bind address must be loopback")]
    NonLoopbackBind,
    #[error("server bind port must be non-zero")]
    ZeroPort,
    #[error("capability token file path must be absolute")]
    RelativeCapabilityPath,
    #[error("model catalog cache file path must be absolute")]
    RelativeCatalogCachePath,
    #[error("failed to read capability token file")]
    ReadCapability(#[source] std::io::Error),
    #[error("capability token file must be a regular non-symlink file")]
    UnsafeCapabilityFileType,
    #[error("capability token file permissions must be exactly 0600")]
    UnsafeCapabilityPermissions,
    #[error("capability token must be 32-128 URL-safe ASCII bytes")]
    InvalidCapability,
    #[cfg(not(unix))]
    #[error("capability permission validation is unsupported on this platform")]
    UnsupportedPermissionPlatform,
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyz_12345";

    fn write_config(bind: &str, token_mode: u32) -> (tempfile::TempDir, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let token_path = temporary.path().join("caller-token");
        fs::write(&token_path, TOKEN).unwrap();
        fs::set_permissions(&token_path, fs::Permissions::from_mode(token_mode)).unwrap();
        let config_path = temporary.path().join("bridge.toml");
        fs::write(
            &config_path,
            format!(
                "version = 1\n\n[server]\nbind = \"{bind}\"\ncapability_token_file = {:?}\n\n[grok]\ncatalog_cache_file = {:?}\nrefresh_on_start = false\n",
                token_path.display().to_string(),
                temporary.path().join("models.json").display().to_string()
            ),
        )
        .unwrap();
        (temporary, config_path)
    }

    #[test]
    fn capability_comparison_matches_only_the_complete_value() {
        let token = CapabilityToken::parse(TOKEN).unwrap();
        assert!(token.matches(TOKEN));
        assert!(!token.matches("abcdefghijklmnopqrstuvwxyz_12346"));
        assert!(!token.matches("abcdefghijklmnopqrstuvwxyz_12345_extra"));
    }

    #[test]
    fn valid_config_loads_only_private_capability_material() {
        let (_temporary, config_path) = write_config("127.0.0.1:4545", 0o600);
        let config = RuntimeConfig::load(&config_path).unwrap();
        assert_eq!(config.bind(), "127.0.0.1:4545".parse().unwrap());
    }

    #[test]
    fn non_loopback_bind_is_rejected() {
        let (_temporary, config_path) = write_config("0.0.0.0:4545", 0o600);
        assert!(matches!(
            RuntimeConfig::load(&config_path),
            Err(ConfigError::NonLoopbackBind)
        ));
    }

    #[test]
    fn group_readable_capability_file_is_rejected() {
        let (_temporary, config_path) = write_config("127.0.0.1:4545", 0o640);
        assert!(matches!(
            RuntimeConfig::load(&config_path),
            Err(ConfigError::UnsafeCapabilityPermissions)
        ));
    }
}
