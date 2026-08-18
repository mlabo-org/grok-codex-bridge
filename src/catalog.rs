use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use tempfile::Builder;
use thiserror::Error;
use tokio::sync::RwLock;

pub const OFFICIAL_MODELS_ORIGIN: &str = "https://cli-chat-proxy.grok.com/v1/models";

const CACHE_VERSION: u32 = 1;
const BOOTSTRAP_MODELS: &[&str] = &["grok-4.6", "grok-4.5"];

#[derive(Clone)]
pub struct ModelCatalog {
    models: Arc<RwLock<Vec<ModelObject>>>,
}

impl ModelCatalog {
    pub fn bootstrap() -> Result<Self, CatalogError> {
        Self::from_ids(BOOTSTRAP_MODELS.iter().copied())
    }

    pub fn from_ids<I, S>(models: I) -> Result<Self, CatalogError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let models = validate_models(models)?;
        Ok(Self {
            models: Arc::new(RwLock::new(models)),
        })
    }

    pub async fn replace<I, S>(&self, models: I) -> Result<(), CatalogError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let candidate = validate_models(models)?;
        *self.models.write().await = candidate;
        Ok(())
    }

    pub(crate) async fn response(&self) -> ModelList {
        ModelList {
            object: "list",
            data: self.models.read().await.clone(),
        }
    }

    pub(crate) async fn contains(&self, model: &str) -> bool {
        self.models
            .read()
            .await
            .iter()
            .any(|candidate| candidate.id == model)
    }

    #[cfg(test)]
    async fn ids(&self) -> Vec<String> {
        self.models
            .read()
            .await
            .iter()
            .map(|model| model.id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    version: u32,
    origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    models: Vec<String>,
}

impl CatalogSnapshot {
    pub fn new<I, S>(models: I, etag: Option<String>) -> Result<Self, CatalogError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let models = validate_model_ids(models)?;
        validate_etag(etag.as_deref())?;
        Ok(Self {
            version: CACHE_VERSION,
            origin: OFFICIAL_MODELS_ORIGIN.to_owned(),
            etag,
            models,
        })
    }

    pub fn model_ids(&self) -> &[String] {
        &self.models
    }

    pub fn models(&self) -> &[String] {
        &self.models
    }

    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    fn validate(&self) -> Result<(), CatalogError> {
        if self.version != CACHE_VERSION {
            return Err(CatalogError::UnsupportedCacheVersion(self.version));
        }
        if self.origin != OFFICIAL_MODELS_ORIGIN {
            return Err(CatalogError::WrongCacheOrigin);
        }
        validate_etag(self.etag.as_deref())?;
        validate_model_ids(self.models.iter().cloned())?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CatalogCache {
    path: PathBuf,
}

impl CatalogCache {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<CatalogSnapshot>, CatalogError> {
        let Some(mut file) = open_cache_for_read(&self.path)? else {
            return Ok(None);
        };
        validate_cache_file(&file)?;

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| CatalogError::CacheRead(error.to_string()))?;
        let snapshot: CatalogSnapshot = serde_json::from_slice(&bytes)
            .map_err(|error| CatalogError::MalformedCache(error.to_string()))?;
        snapshot.validate()?;
        Ok(Some(snapshot))
    }

    pub fn persist(&self, snapshot: &CatalogSnapshot) -> Result<(), CatalogError> {
        snapshot.validate()?;
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or(CatalogError::InvalidCachePath)?;
        let parent_metadata =
            fs::metadata(parent).map_err(|error| CatalogError::CacheWrite(error.to_string()))?;
        if !parent_metadata.is_dir() {
            return Err(CatalogError::CacheParentNotDirectory);
        }

        let encoded = serde_json::to_vec(snapshot)
            .map_err(|error| CatalogError::MalformedCache(error.to_string()))?;
        let mut temporary = Builder::new()
            .prefix(".grok-model-catalog.")
            .tempfile_in(parent)
            .map_err(|error| CatalogError::CacheWrite(error.to_string()))?;
        set_private_permissions(temporary.as_file())?;
        temporary
            .write_all(&encoded)
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| CatalogError::CacheWrite(error.to_string()))?;
        temporary
            .persist(&self.path)
            .map_err(|error| CatalogError::CacheWrite(error.error.to_string()))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| CatalogError::CacheWrite(error.to_string()))?;
        Ok(())
    }
}

fn open_cache_for_read(path: &Path) -> Result<Option<File>, CatalogError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);

    match options.open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CatalogError::CacheRead(error.to_string())),
    }
}

fn validate_cache_file(file: &File) -> Result<(), CatalogError> {
    let metadata = file
        .metadata()
        .map_err(|error| CatalogError::CacheRead(error.to_string()))?;
    if !metadata.is_file() {
        return Err(CatalogError::CacheFileNotRegular);
    }
    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(CatalogError::InsecureCachePermissions(mode));
        }
    }
    Ok(())
}

fn set_private_permissions(file: &File) -> Result<(), CatalogError> {
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| CatalogError::CacheWrite(error.to_string()))?;
    Ok(())
}

pub(crate) fn validate_model_ids<I, S>(models: I) -> Result<Vec<String>, CatalogError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    Ok(validate_models(models)?
        .into_iter()
        .map(|model| model.id)
        .collect())
}

fn validate_models<I, S>(models: I) -> Result<Vec<ModelObject>, CatalogError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut seen = HashSet::new();
    let mut admitted = Vec::new();

    for model in models {
        let id = model.into();
        if !(6..=128).contains(&id.len())
            || !id.starts_with("grok-")
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(CatalogError::InvalidModelId);
        }
        if !seen.insert(id.clone()) {
            return Err(CatalogError::DuplicateModelId);
        }
        admitted.push(ModelObject {
            id,
            object: "model",
            owned_by: "xai",
        });
    }

    if admitted.is_empty() {
        return Err(CatalogError::EmptyCatalog);
    }
    Ok(admitted)
}

fn validate_etag(etag: Option<&str>) -> Result<(), CatalogError> {
    if etag.is_some_and(|value| value.is_empty() || value.contains(['\r', '\n'])) {
        return Err(CatalogError::InvalidEtag);
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ModelList {
    object: &'static str,
    data: Vec<ModelObject>,
}

#[derive(Clone, Debug, Serialize)]
struct ModelObject {
    id: String,
    object: &'static str,
    owned_by: &'static str,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("model catalog must contain at least one admitted model")]
    EmptyCatalog,
    #[error("model catalog contains an invalid Grok model identifier")]
    InvalidModelId,
    #[error("model catalog contains a duplicate model identifier")]
    DuplicateModelId,
    #[error("model catalog cache path must have a parent directory")]
    InvalidCachePath,
    #[error("model catalog cache parent is not a directory")]
    CacheParentNotDirectory,
    #[error("model catalog cache is not a regular file")]
    CacheFileNotRegular,
    #[error("model catalog cache must have mode 0600, found {0:o}")]
    InsecureCachePermissions(u32),
    #[error("unsupported model catalog cache version {0}")]
    UnsupportedCacheVersion(u32),
    #[error("model catalog cache origin does not match the official models endpoint")]
    WrongCacheOrigin,
    #[error("model catalog cache ETag is invalid")]
    InvalidEtag,
    #[error("could not read model catalog cache: {0}")]
    CacheRead(String),
    #[error("could not write model catalog cache: {0}")]
    CacheWrite(String),
    #[error("model catalog cache JSON is malformed: {0}")]
    MalformedCache(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn future_official_model_can_replace_the_bootstrap_catalog() {
        let catalog = ModelCatalog::bootstrap().unwrap();
        catalog.replace(["grok-4.7", "grok-4.6"]).await.unwrap();
        assert_eq!(catalog.ids().await, ["grok-4.7", "grok-4.6"]);
    }

    #[tokio::test]
    async fn invalid_replacement_preserves_last_known_good_catalog() {
        let catalog = ModelCatalog::bootstrap().unwrap();
        let before = catalog.ids().await;
        assert_eq!(
            catalog.replace(Vec::<String>::new()).await,
            Err(CatalogError::EmptyCatalog)
        );
        assert_eq!(catalog.ids().await, before);
    }

    #[test]
    #[cfg(unix)]
    fn future_model_cache_round_trips_with_private_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let cache = CatalogCache::new(directory.path().join("catalog.json"));
        let snapshot =
            CatalogSnapshot::new(["grok-4.7", "grok-4.6"], Some("\"catalog-v47\"".to_owned()))
                .unwrap();

        cache.persist(&snapshot).unwrap();

        assert_eq!(cache.load().unwrap(), Some(snapshot));
        assert_eq!(
            fs::metadata(cache.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn missing_cache_is_not_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let cache = CatalogCache::new(directory.path().join("missing.json"));
        assert_eq!(cache.load().unwrap(), None);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn untrusted_cache_data_is_rejected_without_mutating_the_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("catalog.json");
        let cache = CatalogCache::new(&cache_path);
        let catalog = ModelCatalog::bootstrap().unwrap();
        let before = catalog.ids().await;

        for untrusted in [
            serde_json::json!({
                "version": 1,
                "origin": "https://example.invalid/v1/models",
                "models": ["grok-4.7"]
            }),
            serde_json::json!({
                "version": 1,
                "origin": OFFICIAL_MODELS_ORIGIN,
                "models": ["grok-4.7", "grok-4.7"]
            }),
        ] {
            fs::write(&cache_path, serde_json::to_vec(&untrusted).unwrap()).unwrap();
            fs::set_permissions(&cache_path, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(cache.load().is_err());
            assert_eq!(catalog.ids().await, before);
        }
    }

    #[test]
    #[cfg(unix)]
    fn symlink_cache_is_not_followed() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.json");
        let link = directory.path().join("catalog.json");
        fs::write(&target, b"{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &link).unwrap();

        assert!(CatalogCache::new(link).load().is_err());
    }
}
