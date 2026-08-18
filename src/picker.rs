use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::catalog::{CatalogError, CatalogSnapshot, OFFICIAL_MODELS_ORIGIN, validate_model_ids};

const PICKER_POLICY_VERSION: u32 = 2;
const MANAGED_STATE_VERSION: u32 = 2;
const GROK_ENTRY_DESCRIPTION: &str = "Grok model served through Grok Codex Bridge.";
const GROK_BASE_INSTRUCTIONS: &str = "You are Codex, a coding agent using the selected Grok model through Grok Codex Bridge. Follow the developer and user instructions supplied by Codex, and use Codex tools when needed.";

/// A generated complete-replacement Codex catalog.
///
/// The input catalog is borrowed and never edited. Native entries and unknown
/// fields are retained as JSON values; only policy-owned Grok entries are
/// appended to the generated copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedPickerCatalog {
    bytes: Vec<u8>,
    native_model_slugs: Vec<String>,
    native_model_count: usize,
    grok_model_count: usize,
}

impl GeneratedPickerCatalog {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    #[must_use]
    pub fn native_model_count(&self) -> usize {
        self.native_model_count
    }

    #[must_use]
    pub fn native_model_slugs(&self) -> &[String] {
        &self.native_model_slugs
    }

    #[must_use]
    pub fn grok_model_count(&self) -> usize {
        self.grok_model_count
    }
}

/// Generates the one bridge-owned catalog consumed later by Phase J.
///
/// `grok_catalog` is deliberately a [`CatalogSnapshot`], so picker admission
/// cannot bypass the refreshable Grok catalog owner in `catalog.rs`.
pub fn generate_picker_catalog(
    native_catalog: &[u8],
    grok_catalog: &CatalogSnapshot,
) -> Result<GeneratedPickerCatalog, PickerError> {
    let mut root: Value = serde_json::from_slice(native_catalog)
        .map_err(|error| PickerError::MalformedNativeCatalog(error.to_string()))?;
    let root_object = root
        .as_object_mut()
        .ok_or(PickerError::InvalidNativeCatalog(
            "catalog root must be an object",
        ))?;
    let native_models = root_object
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or(PickerError::InvalidNativeCatalog(
            "catalog must contain a models array",
        ))?;
    if native_models.is_empty() {
        return Err(PickerError::InvalidNativeCatalog(
            "native models array must not be empty",
        ));
    }

    let mut seen = HashSet::with_capacity(native_models.len() + grok_catalog.models().len());
    let mut native_model_slugs = Vec::with_capacity(native_models.len());
    for (index, model) in native_models.iter().enumerate() {
        let slug = validate_model_entry(model, index)?;
        if !seen.insert(slug.to_owned()) {
            return Err(PickerError::DuplicateSlug(slug.to_owned()));
        }
        native_model_slugs.push(slug.to_owned());
    }

    let native_model_count = native_models.len();
    let mut grok_ids = grok_catalog.models().to_vec();
    grok_ids.sort_unstable();
    for (index, id) in grok_ids.iter().enumerate() {
        if !seen.insert(id.clone()) {
            return Err(PickerError::DuplicateSlug(id.clone()));
        }
        let priority = 1_000_i32
            .checked_add(i32::try_from(index).map_err(|_| PickerError::TooManyModels)?)
            .ok_or(PickerError::TooManyModels)?;
        let entry = grok_picker_entry(id, priority);
        validate_model_entry(&entry, native_model_count + index)?;
        native_models.push(entry);
    }

    let bytes = serde_json::to_vec_pretty(&root)
        .map_err(|error| PickerError::SerializeCatalog(error.to_string()))?;
    Ok(GeneratedPickerCatalog {
        bytes,
        native_model_slugs,
        native_model_count,
        grok_model_count: grok_ids.len(),
    })
}

fn grok_picker_entry(id: &str, priority: i32) -> Value {
    let mut entry = json!({
        "slug": id,
        "display_name": id,
        "description": GROK_ENTRY_DESCRIPTION,
        "default_reasoning_level": "high",
        "supported_reasoning_levels": [
            {
                "effort": "low",
                "description": "Faster, lighter reasoning"
            },
            {
                "effort": "medium",
                "description": "Balanced reasoning"
            },
            {
                "effort": "high",
                "description": "Heavy reasoning"
            }
        ],
        "shell_type": "default",
        "visibility": "list",
        "supported_in_api": true,
        "priority": priority,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "default_service_tier": null,
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": GROK_BASE_INSTRUCTIONS,
        "model_messages": null,
        "include_skills_usage_instructions": true,
        "include_plugin_usage_instructions": true,
        "include_apps_usage_instructions": true,
        "supports_reasoning_summary_parameter": false,
        "default_reasoning_summary": "auto",
        "support_verbosity": false,
        "default_verbosity": null,
    });
    let remaining = json!({
        "apply_patch_tool_type": null,
        "web_search_tool_type": "text",
        "truncation_policy": {
            "mode": "bytes",
            "limit": 10_000
        },
        "supports_parallel_tool_calls": true,
        "supports_image_detail_original": false,
        "context_window": null,
        "max_context_window": null,
        "auto_compact_token_limit": null,
        "comp_hash": null,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": true,
        "use_responses_lite": false,
        "node_repl_auto_review_required": false,
        "node_repl_disabled": false,
        "auto_review_model_override": null,
        "model_specialty": null,
        "tool_mode": null,
        "multi_agent_version": null
    });
    entry
        .as_object_mut()
        .expect("Grok picker policy must be an object")
        .extend(
            remaining
                .as_object()
                .expect("Grok picker policy continuation must be an object")
                .clone(),
        );
    entry
}

fn validate_model_entry(model: &Value, index: usize) -> Result<&str, PickerError> {
    let object = model.as_object().ok_or(PickerError::InvalidModelSchema {
        index,
        field: "model entry",
    })?;
    let slug = required_non_empty_string(object, "slug", index)?;
    required_non_empty_string(object, "display_name", index)?;
    required_array(object, "supported_reasoning_levels", index)?;
    required_enum(
        object,
        "shell_type",
        &[
            "default",
            "local",
            "unified_exec",
            "disabled",
            "shell_command",
        ],
        index,
    )?;
    required_enum(object, "visibility", &["list", "hide", "none"], index)?;
    required_bool(object, "supported_in_api", index)?;
    let priority =
        object
            .get("priority")
            .and_then(Value::as_i64)
            .ok_or(PickerError::InvalidModelSchema {
                index,
                field: "priority",
            })?;
    i32::try_from(priority).map_err(|_| PickerError::InvalidModelSchema {
        index,
        field: "priority",
    })?;
    required_bool(object, "support_verbosity", index)?;
    let truncation = object
        .get("truncation_policy")
        .and_then(Value::as_object)
        .ok_or(PickerError::InvalidModelSchema {
            index,
            field: "truncation_policy",
        })?;
    required_enum(truncation, "mode", &["bytes", "tokens"], index)?;
    if truncation.get("limit").and_then(Value::as_i64).is_none() {
        return Err(PickerError::InvalidModelSchema {
            index,
            field: "truncation_policy.limit",
        });
    }
    required_string_array(object, "experimental_supported_tools", index)?;
    if let Some(modalities) = object.get("input_modalities") {
        let modalities = modalities
            .as_array()
            .ok_or(PickerError::InvalidModelSchema {
                index,
                field: "input_modalities",
            })?;
        if modalities
            .iter()
            .any(|modality| !matches!(modality.as_str(), Some("text" | "image" | "audio")))
        {
            return Err(PickerError::InvalidModelSchema {
                index,
                field: "input_modalities",
            });
        }
    }
    validate_reasoning_levels(object, index)?;
    validate_instructions(object, index)?;
    Ok(slug)
}

fn validate_reasoning_levels(object: &Map<String, Value>, index: usize) -> Result<(), PickerError> {
    for level in required_array(object, "supported_reasoning_levels", index)? {
        let level = level.as_object().ok_or(PickerError::InvalidModelSchema {
            index,
            field: "supported_reasoning_levels",
        })?;
        required_non_empty_string(level, "effort", index)?;
        required_non_empty_string(level, "description", index)?;
    }
    Ok(())
}

fn validate_instructions(object: &Map<String, Value>, index: usize) -> Result<(), PickerError> {
    let has_legacy = object
        .get("base_instructions")
        .is_some_and(Value::is_string);
    let has_current = object
        .get("model_messages")
        .and_then(Value::as_object)
        .and_then(|messages| messages.get("instructions_template"))
        .is_some_and(Value::is_string);
    if has_legacy || has_current {
        Ok(())
    } else {
        Err(PickerError::InvalidModelSchema {
            index,
            field: "base_instructions or model_messages.instructions_template",
        })
    }
}

fn required_non_empty_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
    index: usize,
) -> Result<&'a str, PickerError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(PickerError::InvalidModelSchema { index, field })
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
    index: usize,
) -> Result<&'a [Value], PickerError> {
    object
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or(PickerError::InvalidModelSchema { index, field })
}

fn required_string_array(
    object: &Map<String, Value>,
    field: &'static str,
    index: usize,
) -> Result<(), PickerError> {
    if required_array(object, field, index)?
        .iter()
        .any(|value| !value.is_string())
    {
        return Err(PickerError::InvalidModelSchema { index, field });
    }
    Ok(())
}

fn required_bool(
    object: &Map<String, Value>,
    field: &'static str,
    index: usize,
) -> Result<(), PickerError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .map(|_| ())
        .ok_or(PickerError::InvalidModelSchema { index, field })
}

fn required_enum(
    object: &Map<String, Value>,
    field: &'static str,
    admitted: &[&str],
    index: usize,
) -> Result<(), PickerError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| admitted.contains(value))
        .map(|_| ())
        .ok_or(PickerError::InvalidModelSchema { index, field })
}

/// Exact, non-secret identity supplied by the Phase J lifecycle owner.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    path: PathBuf,
    byte_len: u64,
    sha256: String,
}

impl ArtifactIdentity {
    pub fn new(
        path: impl Into<PathBuf>,
        byte_len: u64,
        sha256: impl Into<String>,
    ) -> Result<Self, PickerError> {
        let identity = Self {
            path: path.into(),
            byte_len,
            sha256: sha256.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    fn validate(&self) -> Result<(), PickerError> {
        validate_absolute_clean(&self.path)?;
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(PickerError::InvalidManagedState(
                "artifact sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedGrokCatalogState {
    origin: String,
    etag: Option<String>,
    models: Vec<String>,
}

impl AdmittedGrokCatalogState {
    fn from_snapshot(snapshot: &CatalogSnapshot) -> Self {
        let mut models = snapshot.models().to_vec();
        models.sort_unstable();
        Self {
            origin: OFFICIAL_MODELS_ORIGIN.to_owned(),
            etag: snapshot.etag().map(str::to_owned),
            models,
        }
    }

    fn validate(&self) -> Result<(), PickerError> {
        if self.origin != OFFICIAL_MODELS_ORIGIN {
            return Err(PickerError::InvalidManagedState(
                "Grok catalog origin is not authoritative",
            ));
        }
        validate_model_ids(self.models.iter().cloned())?;
        if self.models.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PickerError::InvalidManagedState(
                "Grok model identifiers must be strictly sorted",
            ));
        }
        if self
            .etag
            .as_deref()
            .is_some_and(|etag| etag.is_empty() || etag.contains(['\r', '\n']))
        {
            return Err(PickerError::InvalidManagedState(
                "Grok catalog ETag is invalid",
            ));
        }
        Ok(())
    }
}

/// Exact rollback receipt for the Codex config target that Phase J mutates.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum ConfigRollbackOwnership {
    RemoveCreated,
    RestoreExactBackup {
        backup: ArtifactIdentity,
        original_mode: u32,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum GeneratedCatalogRollbackOwnership {
    RemoveIfIdentityMatches,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NativeRouteRollbackOwnership {
    RemoveIfIdentityMatches,
}

/// Versioned metadata-only state. It contains no credential, capability, or
/// request content and performs no filesystem mutation by itself.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PickerManagedState {
    version: u32,
    policy_version: u32,
    native_catalog: ArtifactIdentity,
    grok_catalog: AdmittedGrokCatalogState,
    generated_catalog: ArtifactIdentity,
    generated_catalog_rollback: GeneratedCatalogRollbackOwnership,
    native_route: ArtifactIdentity,
    native_route_rollback: NativeRouteRollbackOwnership,
    managed_config: ArtifactIdentity,
    config_rollback: ConfigRollbackOwnership,
}

impl PickerManagedState {
    pub fn new(
        native_catalog: ArtifactIdentity,
        grok_catalog: &CatalogSnapshot,
        generated_catalog: ArtifactIdentity,
        native_route: ArtifactIdentity,
        managed_config: ArtifactIdentity,
        config_rollback: ConfigRollbackOwnership,
    ) -> Result<Self, PickerError> {
        let state = Self {
            version: MANAGED_STATE_VERSION,
            policy_version: PICKER_POLICY_VERSION,
            native_catalog,
            grok_catalog: AdmittedGrokCatalogState::from_snapshot(grok_catalog),
            generated_catalog,
            generated_catalog_rollback: GeneratedCatalogRollbackOwnership::RemoveIfIdentityMatches,
            native_route,
            native_route_rollback: NativeRouteRollbackOwnership::RemoveIfIdentityMatches,
            managed_config,
            config_rollback,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, PickerError> {
        let state: Self = serde_json::from_slice(bytes)
            .map_err(|error| PickerError::MalformedManagedState(error.to_string()))?;
        state.validate()?;
        Ok(state)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, PickerError> {
        self.validate()?;
        serde_json::to_vec_pretty(self)
            .map_err(|error| PickerError::SerializeManagedState(error.to_string()))
    }

    #[must_use]
    pub fn native_catalog(&self) -> &ArtifactIdentity {
        &self.native_catalog
    }

    #[must_use]
    pub fn generated_catalog(&self) -> &ArtifactIdentity {
        &self.generated_catalog
    }

    #[must_use]
    pub fn native_route(&self) -> &ArtifactIdentity {
        &self.native_route
    }

    #[must_use]
    pub fn managed_config(&self) -> &ArtifactIdentity {
        &self.managed_config
    }

    #[must_use]
    pub fn config_rollback(&self) -> &ConfigRollbackOwnership {
        &self.config_rollback
    }

    fn validate(&self) -> Result<(), PickerError> {
        if self.version != MANAGED_STATE_VERSION || self.policy_version != PICKER_POLICY_VERSION {
            return Err(PickerError::InvalidManagedState(
                "managed state or picker policy version is unsupported",
            ));
        }
        self.native_catalog.validate()?;
        self.grok_catalog.validate()?;
        self.generated_catalog.validate()?;
        self.native_route.validate()?;
        self.managed_config.validate()?;

        let paths = [
            self.native_catalog.path(),
            self.generated_catalog.path(),
            self.native_route.path(),
            self.managed_config.path(),
        ];
        for (index, path) in paths.iter().enumerate() {
            if paths.iter().skip(index + 1).any(|other| path == other) {
                return Err(PickerError::InvalidManagedState(
                    "managed artifact paths must be distinct",
                ));
            }
        }

        if let ConfigRollbackOwnership::RestoreExactBackup {
            backup,
            original_mode,
        } = &self.config_rollback
        {
            backup.validate()?;
            if !matches!(*original_mode, 0o600 | 0o644)
                || backup.path() == self.managed_config.path()
                || backup.path().parent() != self.managed_config.path().parent()
                || !backup
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".grok-codex-bridge-backup-"))
            {
                return Err(PickerError::InvalidManagedState(
                    "config backup ownership is not an exact private sibling backup",
                ));
            }
        }
        Ok(())
    }
}

fn validate_absolute_clean(path: &Path) -> Result<(), PickerError> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
        || path.parent().is_none()
    {
        return Err(PickerError::InvalidManagedState(
            "managed artifact path must be absolute and normalized",
        ));
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PickerError {
    #[error("native Codex model catalog JSON is malformed: {0}")]
    MalformedNativeCatalog(String),
    #[error("native Codex model catalog is invalid: {0}")]
    InvalidNativeCatalog(&'static str),
    #[error("model catalog entry {index} has invalid field {field}")]
    InvalidModelSchema { index: usize, field: &'static str },
    #[error("model catalog contains duplicate slug {0}")]
    DuplicateSlug(String),
    #[error("model catalog contains too many models")]
    TooManyModels,
    #[error("could not serialize generated picker catalog: {0}")]
    SerializeCatalog(String),
    #[error("picker managed-state JSON is malformed: {0}")]
    MalformedManagedState(String),
    #[error("picker managed state is invalid: {0}")]
    InvalidManagedState(&'static str),
    #[error("could not serialize picker managed state: {0}")]
    SerializeManagedState(String),
    #[error(transparent)]
    GrokCatalog(#[from] CatalogError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_catalog() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "models": [native_model("gpt-native", 1)],
            "future_top_level": {
                "opaque": [1, {"kept": true}]
            }
        }))
        .unwrap()
    }

    fn native_model(slug: &str, priority: i32) -> Value {
        json!({
            "slug": slug,
            "display_name": "Native GPT",
            "description": "native",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {"effort": "medium", "description": "Balanced"}
            ],
            "shell_type": "unified_exec",
            "visibility": "list",
            "supported_in_api": true,
            "priority": priority,
            "base_instructions": "native instructions",
            "support_verbosity": true,
            "truncation_policy": {"mode": "tokens", "limit": 12345},
            "experimental_supported_tools": ["future_tool"],
            "input_modalities": ["text", "image"],
            "future_native_field": {
                "nested": ["preserved", 42]
            }
        })
    }

    #[test]
    fn generated_catalog_preserves_native_entries_and_unknown_fields() {
        let source = native_catalog();
        let before: Value = serde_json::from_slice(&source).unwrap();
        let grok = CatalogSnapshot::new(["grok-4.6"], Some("\"grok-v1\"".to_owned())).unwrap();

        let generated = generate_picker_catalog(&source, &grok).unwrap();
        let after: Value = serde_json::from_slice(generated.bytes()).unwrap();

        assert_eq!(generated.native_model_count(), 1);
        assert_eq!(generated.grok_model_count(), 1);
        assert_eq!(generated.native_model_slugs(), &["gpt-native"]);
        assert_eq!(after["models"][0], before["models"][0]);
        assert_eq!(after["future_top_level"], before["future_top_level"]);
        assert_eq!(after["models"][1]["slug"], "grok-4.6");
        assert!(after["models"][1].get("model_provider").is_none());
        assert_eq!(after["models"][1]["display_name"], "grok-4.6");
        assert_eq!(after["models"][1]["context_window"], Value::Null);
        assert_eq!(after["models"][1]["default_reasoning_level"], "high");
        assert_eq!(
            after["models"][1]["supported_reasoning_levels"],
            json!([
                {"effort": "low", "description": "Faster, lighter reasoning"},
                {"effort": "medium", "description": "Balanced reasoning"},
                {"effort": "high", "description": "Heavy reasoning"}
            ])
        );
        assert_eq!(
            after["models"][1]["include_skills_usage_instructions"],
            true
        );
        assert_eq!(
            after["models"][1]["include_plugin_usage_instructions"],
            true
        );
        assert_eq!(after["models"][1]["include_apps_usage_instructions"], true);
        assert_eq!(after["models"][1]["supports_parallel_tool_calls"], true);
        assert_eq!(serde_json::from_slice::<Value>(&source).unwrap(), before);
    }

    #[test]
    #[ignore = "requires the accepted Codex consumer binary and its bundled catalog"]
    fn generated_catalog_is_accepted_by_codex_consumer() {
        let codex_binary = std::env::var_os("CODEX_CONSUMER_BINARY")
            .expect("CODEX_CONSUMER_BINARY must name the accepted Codex executable");
        let native_catalog = std::env::var_os("CODEX_NATIVE_CATALOG")
            .expect("CODEX_NATIVE_CATALOG must name that consumer's bundled catalog");
        let native_bytes = std::fs::read(native_catalog).unwrap();
        let grok = CatalogSnapshot::new(["grok-4.6"], Some("\"grok-v1\"".to_owned())).unwrap();
        let generated = generate_picker_catalog(&native_bytes, &grok).unwrap();
        let isolated_home = tempfile::tempdir().unwrap();
        let generated_path = isolated_home.path().join("picker-models.json");
        std::fs::write(&generated_path, generated.bytes()).unwrap();
        let catalog_path = serde_json::to_string(generated_path.to_str().unwrap()).unwrap();
        std::fs::write(
            isolated_home.path().join("config.toml"),
            format!(
                "openai_base_url = \"http://127.0.0.1:9/v1\"\nmodel_catalog_json = {catalog_path}\n"
            ),
        )
        .unwrap();

        let output = std::process::Command::new(codex_binary)
            .env("CODEX_HOME", isolated_home.path())
            .args(["debug", "models"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Codex rejected generated catalog: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
        let grok_entry = parsed["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["slug"] == "grok-4.6")
            .expect("Codex consumer output must retain the generated Grok row");
        assert!(grok_entry.get("model_provider").is_none());
        assert_eq!(grok_entry["supports_parallel_tool_calls"], true);
    }

    #[test]
    fn future_admitted_grok_model_flows_without_a_picker_registry_change() {
        let grok = CatalogSnapshot::new(["grok-4.7", "grok-4.6"], None).unwrap();
        let generated = generate_picker_catalog(&native_catalog(), &grok).unwrap();
        let value: Value = serde_json::from_slice(generated.bytes()).unwrap();
        let slugs: Vec<&str> = value["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["slug"].as_str().unwrap())
            .collect();

        assert_eq!(slugs, ["gpt-native", "grok-4.6", "grok-4.7"]);
    }

    #[test]
    fn grok_slug_collision_is_rejected_without_overwrite_or_alias() {
        let native = serde_json::to_vec(&json!({
            "models": [native_model("grok-4.6", 1)]
        }))
        .unwrap();
        let grok = CatalogSnapshot::new(["grok-4.6"], None).unwrap();

        assert_eq!(
            generate_picker_catalog(&native, &grok),
            Err(PickerError::DuplicateSlug("grok-4.6".to_owned()))
        );
    }

    #[test]
    fn malformed_or_schema_invalid_native_catalog_is_rejected() {
        let grok = CatalogSnapshot::new(["grok-4.6"], None).unwrap();
        assert!(matches!(
            generate_picker_catalog(b"{", &grok),
            Err(PickerError::MalformedNativeCatalog(_))
        ));
        assert_eq!(
            generate_picker_catalog(br#"{"models":[]}"#, &grok),
            Err(PickerError::InvalidNativeCatalog(
                "native models array must not be empty"
            ))
        );
        let mut missing_instructions = native_model("gpt-native", 1);
        missing_instructions
            .as_object_mut()
            .unwrap()
            .remove("base_instructions");
        let invalid = serde_json::to_vec(&json!({"models": [missing_instructions]})).unwrap();
        assert_eq!(
            generate_picker_catalog(&invalid, &grok),
            Err(PickerError::InvalidModelSchema {
                index: 0,
                field: "base_instructions or model_messages.instructions_template"
            })
        );
    }

    #[test]
    fn generated_output_is_deterministic_for_the_same_admitted_set() {
        let first = CatalogSnapshot::new(["grok-4.7", "grok-4.6"], None).unwrap();
        let second = CatalogSnapshot::new(["grok-4.6", "grok-4.7"], None).unwrap();

        assert_eq!(
            generate_picker_catalog(&native_catalog(), &first)
                .unwrap()
                .bytes(),
            generate_picker_catalog(&native_catalog(), &second)
                .unwrap()
                .bytes()
        );
    }

    #[test]
    fn managed_state_serializes_input_output_and_exact_rollback_ownership() {
        let grok =
            CatalogSnapshot::new(["grok-4.7", "grok-4.6"], Some("\"v47\"".to_owned())).unwrap();
        let state = PickerManagedState::new(
            identity("/opt/bridge/native-models.json", 100, 'a'),
            &grok,
            identity("/opt/bridge/generated-models.json", 200, 'b'),
            identity("/opt/bridge/picker-native-route.json", 175, 'e'),
            identity("/Users/test/.codex/config.toml", 300, 'c'),
            ConfigRollbackOwnership::RestoreExactBackup {
                backup: identity(
                    "/Users/test/.codex/config.toml.grok-codex-bridge-backup-20260818",
                    250,
                    'd',
                ),
                original_mode: 0o600,
            },
        )
        .unwrap();

        let encoded = state.to_json().unwrap();
        let decoded = PickerManagedState::from_json(&encoded).unwrap();
        assert_eq!(decoded, state);
        let value: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["policy_version"], 2);
        assert_eq!(
            value["grok_catalog"]["models"],
            json!(["grok-4.6", "grok-4.7"])
        );
        assert_eq!(value["config_rollback"]["strategy"], "restore_exact_backup");
        assert_eq!(
            value["generated_catalog_rollback"],
            "remove_if_identity_matches"
        );
        assert_eq!(value["native_route_rollback"], "remove_if_identity_matches");
        assert!(value.get("credential").is_none());
        assert!(value.get("capability").is_none());
    }

    fn identity(path: &str, byte_len: u64, fill: char) -> ArtifactIdentity {
        ArtifactIdentity::new(path, byte_len, fill.to_string().repeat(64)).unwrap()
    }
}
