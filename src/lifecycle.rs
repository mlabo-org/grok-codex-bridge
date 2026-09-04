use std::fs;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::catalog::{CatalogCache, CatalogSnapshot};
use crate::credential::CredentialStore;
use crate::native::{NativeRouteState, NativeUpstream};
use crate::picker::{
    ArtifactIdentity, ConfigRollbackOwnership, PickerManagedState,
    generate_native_compatibility_catalog, generate_picker_catalog,
};

const MANIFEST_VERSION: u32 = 1;
const CONFIG_VERSION: u32 = 1;
const MANIFEST_FILE_NAME: &str = "install-manifest.json";
const PROFILE_FILE_NAME: &str = "grok-bridge.config.toml";
const PROFILE_PROVIDER_NAME: &str = "Grok Codex Bridge";
const BINARY_FILE_NAME: &str = "grok-codex-bridge";
const LAUNCHER_APP_PATH: &str = "bin/Grok Codex Switch.app";
const LAUNCHER_EXECUTABLE_PATH: &str = "Contents/MacOS/Grok Codex Switch";
const LAUNCHER_OVERLAY_PATH: &str = "Contents/Resources/grok-codex-bridge-overlay.md";
const MAX_CONTROL_FILE_BYTES: u64 = 1024 * 1024;
const CAPABILITY_LENGTH: usize = 64;
const PICKER_CATALOG_FILE_NAME: &str = "picker-models.json";
const PICKER_NATIVE_ROUTE_FILE_NAME: &str = "picker-native-route.json";
const PICKER_STATE_FILE_NAME: &str = "picker-managed-state.json";
const PICKER_BEGIN: &str = "# >>> grok-codex-bridge picker begin >>>";
const PICKER_END: &str = "# <<< grok-codex-bridge picker end <<<";
const PICKER_PROVIDER_ID: &str = "grok_codex_picker";
const PICKER_PROVIDER_NAME: &str = "Grok Codex Picker Bridge";
pub(crate) const PICKER_CALLER_HEADER: &str = "x-grok-caller-capability";

/// Complete, explicit inputs owned by the filesystem lifecycle producer.
pub struct InstallRequest {
    pub source_binary: PathBuf,
    pub source_launcher: PathBuf,
    pub install_root: PathBuf,
    pub codex_home: PathBuf,
    pub launch_agent_path: PathBuf,
    pub launch_agent_label: String,
    pub launch_agent_contents: String,
    pub bind: SocketAddr,
    pub initial_model: String,
}

/// Non-secret paths materialized by a successful install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReceipt {
    pub install_root: PathBuf,
    pub binary_path: PathBuf,
    pub launcher_path: PathBuf,
    pub config_path: PathBuf,
    pub profile_path: PathBuf,
    pub launch_agent_path: PathBuf,
    pub manifest_path: PathBuf,
    pub profile_replaced: bool,
    pub launch_agent_replaced: bool,
}

pub struct UninstallRequest {
    pub install_root: PathBuf,
    pub codex_home: PathBuf,
    pub launch_agent_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UninstallReceipt {
    pub install_root: PathBuf,
    pub profile_restored: bool,
    pub launch_agent_restored: bool,
}

/// Inputs for the merged-picker generated-catalog lifecycle. The caller supplies
/// the authoritative local native catalog; it is copied, never edited.
pub struct PickerInstallRequest {
    pub install_root: PathBuf,
    pub codex_home: PathBuf,
    pub native_catalog_path: PathBuf,
    pub grok_overlay_path: PathBuf,
    pub native_upstream: NativeUpstream,
    pub bind: SocketAddr,
    pub native_compatibility: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerInstallReceipt {
    pub generated_catalog_path: PathBuf,
    pub native_route_path: PathBuf,
    pub managed_state_path: PathBuf,
    pub config_path: PathBuf,
    pub native_model_count: usize,
    pub grok_model_count: usize,
}

/// Result of reconciling the installed merged picker with its two authoritative
/// catalog inputs. The Native catalog remains read-only; only bridge-owned
/// generated state is replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerCatalogSyncReceipt {
    pub changed: bool,
    pub native_catalog: ArtifactIdentity,
    pub grok_catalog: ArtifactIdentity,
    pub native_route: NativeRouteState,
    pub native_model_count: usize,
    pub grok_model_ids: Vec<String>,
}

pub struct DoctorRequest {
    pub install_root: PathBuf,
    pub codex_home: PathBuf,
    pub launch_agent_path: PathBuf,
    pub credential_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub id: &'static str,
    pub status: DoctorCheckStatus,
    pub message: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.checks
            .iter()
            .all(|check| check.status == DoctorCheckStatus::Passed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAvailability {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AuthStatus {
    pub availability: AuthAvailability,
    pub message: &'static str,
}

/// Validates the configured credential in place without returning identity,
/// token, or path material.
#[must_use]
pub fn auth_status(store: &CredentialStore) -> AuthStatus {
    match store.load() {
        Ok(_) => AuthStatus {
            availability: AuthAvailability::Available,
            message: "Grok session credential is available",
        },
        Err(_) => AuthStatus {
            availability: AuthAvailability::Unavailable,
            message: "Grok session credential is unavailable or invalid",
        },
    }
}

pub fn install(request: &InstallRequest) -> Result<InstallReceipt, LifecycleError> {
    install_inner(request, InstallFailpoint::None)
}

fn install_inner(
    request: &InstallRequest,
    failpoint: InstallFailpoint,
) -> Result<InstallReceipt, LifecycleError> {
    let paths = InstallPaths::validate(request)?;
    let source = read_source_binary(&request.source_binary)?;
    validate_launcher_bundle(&request.source_launcher)?;
    let capability = generate_capability();
    if request.launch_agent_contents.contains(&capability) {
        return Err(LifecycleError::LaunchAgentContainsCapability);
    }

    let profile_contents = render_profile(&request.initial_model, request.bind, &capability)?;
    let config_contents = render_runtime_config(&paths, request.bind)?;

    let stage_root = unique_sibling(&request.install_root, "stage")?;
    create_stage_tree(&stage_root)?;
    let stage_paths = StagePaths::new(&stage_root);
    let stage_result = (|| {
        write_new_file(&stage_paths.binary, &source, 0o755)?;
        copy_launcher_bundle(&request.source_launcher, &stage_paths.launcher)?;
        write_new_file(&stage_paths.config, config_contents.as_bytes(), 0o600)?;
        write_new_file(&stage_paths.capability, capability.as_bytes(), 0o600)?;
        CatalogCache::new(&stage_paths.catalog).persist(&CatalogSnapshot::bootstrap()?)?;
        Ok::<(), LifecycleError>(())
    })();
    if let Err(error) = stage_result {
        let _ = fs::remove_dir_all(&stage_root);
        return Err(error);
    }

    let profile_plan = match prepare_external_target(&paths.profile, 0o600, false) {
        Ok(plan) => plan,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage_root);
            return Err(error);
        }
    };
    let launch_plan = match prepare_external_target(&request.launch_agent_path, 0o644, true) {
        Ok(plan) => plan,
        Err(error) => {
            cleanup_prepared_backup(&profile_plan);
            let _ = fs::remove_dir_all(&stage_root);
            return Err(error);
        }
    };

    let manifest = InstallManifest {
        version: MANIFEST_VERSION,
        install_root: request.install_root.clone(),
        binary_path: paths.binary.clone(),
        launcher_path: paths.launcher.clone(),
        config_path: paths.config.clone(),
        catalog_cache_path: paths.catalog.clone(),
        caller_token_path: paths.capability.clone(),
        logs_dir: paths.logs.clone(),
        profile_path: paths.profile.clone(),
        profile_backup: profile_plan.backup.clone(),
        profile_created: profile_plan.created,
        launch_agent_path: request.launch_agent_path.clone(),
        launch_agent_label: request.launch_agent_label.clone(),
        launch_agent_contents: request.launch_agent_contents.clone(),
        launch_agent_mode: 0o644,
        launch_agent_backup: launch_plan.backup.clone(),
        launch_agent_created: launch_plan.created,
        bind: request.bind.to_string(),
        initial_model: request.initial_model.clone(),
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(LifecycleError::SerializeManifest)?;
    if let Err(error) = write_new_file(&stage_paths.manifest, &manifest_bytes, 0o600) {
        cleanup_prepared_backup(&profile_plan);
        cleanup_prepared_backup(&launch_plan);
        let _ = fs::remove_dir_all(&stage_root);
        return Err(error);
    }

    let mut profile_mutated = false;
    let mut launch_mutated = false;
    let mut root_moved = false;
    let result = (|| {
        atomic_write(&paths.profile, profile_contents.as_bytes(), 0o600)?;
        profile_mutated = true;
        if failpoint == InstallFailpoint::AfterProfile {
            return Err(LifecycleError::InjectedInstallFailure);
        }
        atomic_write(
            &request.launch_agent_path,
            request.launch_agent_contents.as_bytes(),
            0o644,
        )?;
        launch_mutated = true;
        fs::rename(&stage_root, &request.install_root)
            .map_err(LifecycleError::InstallRootRename)?;
        root_moved = true;
        sync_parent(&request.install_root)?;
        Ok(())
    })();

    if let Err(error) = result {
        let rollback = rollback_install(
            &profile_plan,
            profile_mutated,
            &launch_plan,
            launch_mutated,
            &stage_root,
            &request.install_root,
            root_moved,
        );
        return match rollback {
            Ok(()) => Err(error),
            Err(_) => Err(LifecycleError::InstallRollbackFailed),
        };
    }

    Ok(InstallReceipt {
        install_root: request.install_root.clone(),
        binary_path: paths.binary,
        launcher_path: paths.launcher,
        config_path: paths.config,
        profile_path: paths.profile,
        launch_agent_path: request.launch_agent_path.clone(),
        manifest_path: request.install_root.join(MANIFEST_FILE_NAME),
        profile_replaced: !profile_plan.created,
        launch_agent_replaced: !launch_plan.created,
    })
}

pub fn uninstall(request: &UninstallRequest) -> Result<UninstallReceipt, LifecycleError> {
    preflight_uninstall(request)?;

    let (manifest, raw_manifest) = read_manifest(&request.install_root)?;
    let managed = validate_managed_install(&manifest, &raw_manifest)?;

    // A picker install has its own exact rollback state, but full bridge
    // uninstall is the terminal owner and must restore it first.
    uninstall_picker_if_present(&request.install_root, &request.codex_home)?;

    let profile_restore = RestorePlan::new(
        &manifest.profile_path,
        manifest.profile_backup.as_ref(),
        manifest.profile_created,
        &managed.profile_contents,
    )?;
    let launch_restore = RestorePlan::new(
        &manifest.launch_agent_path,
        manifest.launch_agent_backup.as_ref(),
        manifest.launch_agent_created,
        manifest.launch_agent_contents.as_bytes(),
    )?;

    let mut profile_changed = false;
    let mut launch_changed = false;
    let result = (|| {
        profile_restore.apply()?;
        profile_changed = true;
        launch_restore.apply()?;
        launch_changed = true;
        remove_tree_without_symlinks(&request.install_root)?;
        Ok(())
    })();

    if let Err(error) = result {
        let mut rollback_failed = false;
        if launch_changed && launch_restore.rollback().is_err() {
            rollback_failed = true;
        }
        if profile_changed && profile_restore.rollback().is_err() {
            rollback_failed = true;
        }
        return if rollback_failed {
            Err(LifecycleError::UninstallRollbackFailed)
        } else {
            Err(error)
        };
    }

    profile_restore.finish()?;
    launch_restore.finish()?;

    Ok(UninstallReceipt {
        install_root: request.install_root.clone(),
        profile_restored: !manifest.profile_created,
        launch_agent_restored: !manifest.launch_agent_created,
    })
}

/// Proves that every manifest-owned target and picker rollback receipt needed
/// by a full uninstall is currently readable and valid, without mutating the
/// service, Codex configuration, or bridge installation.
pub fn preflight_uninstall(request: &UninstallRequest) -> Result<(), LifecycleError> {
    validate_absolute_clean(&request.install_root)?;
    validate_absolute_clean(&request.codex_home)?;
    validate_absolute_clean(&request.launch_agent_path)?;
    validate_safe_root(&request.install_root, &request.codex_home)?;
    validate_existing_directory(&request.install_root, || LifecycleError::UnsafeInstallRoot)?;

    let (manifest, raw_manifest) = read_manifest(&request.install_root)?;
    validate_manifest(&manifest, request)?;
    validate_managed_install(&manifest, &raw_manifest)?;
    preflight_uninstall_picker_if_present(&request.install_root, &request.codex_home)
}

/// Creates or updates the non-secret merged-picker state. It does
/// not start a service or reload Codex; callers must cross that boundary
/// explicitly with the exact accepted Codex binary.
pub fn install_picker(
    request: &PickerInstallRequest,
) -> Result<PickerInstallReceipt, LifecycleError> {
    validate_absolute_clean(&request.install_root)?;
    validate_absolute_clean(&request.codex_home)?;
    validate_absolute_clean(&request.native_catalog_path)?;
    validate_absolute_clean(&request.grok_overlay_path)?;
    validate_safe_root(&request.install_root, &request.codex_home)?;
    if !request.bind.ip().is_loopback() || request.bind.port() == 0 {
        return Err(LifecycleError::NonLoopbackBind);
    }
    let manifest_request = UninstallRequest {
        install_root: request.install_root.clone(),
        codex_home: request.codex_home.clone(),
        launch_agent_path: request.install_root.join(".picker-unused-launch-agent"),
    };
    let (install_manifest, _) = read_manifest(&request.install_root)?;
    // Picker ownership is possible only on a valid managed bridge root. The
    // launch-agent path is checked separately by the root manifest, so use
    // the deterministic manifest fields here rather than an external target.
    if install_manifest.install_root != request.install_root
        || install_manifest.profile_path != request.codex_home.join(PROFILE_FILE_NAME)
        || install_manifest.catalog_cache_path != request.install_root.join("state/models.json")
        || install_manifest.caller_token_path
            != request.install_root.join("secrets/caller-capability")
    {
        return Err(LifecycleError::InvalidManifest);
    }
    let _ = manifest_request;

    let native_bytes = read_regular_file(&request.native_catalog_path, MAX_CONTROL_FILE_BYTES)?;
    let grok_catalog = CatalogCache::new(&install_manifest.catalog_cache_path)
        .load()
        .map_err(LifecycleError::Catalog)?
        .ok_or(LifecycleError::PickerCatalogUnavailable)?;
    let generated = if request.native_compatibility {
        generate_native_compatibility_catalog(&native_bytes).map_err(LifecycleError::Picker)?
    } else {
        let overlay = crate::picker::load_grok_overlay(&request.grok_overlay_path)
            .map_err(LifecycleError::Picker)?;
        generate_picker_catalog(&native_bytes, &grok_catalog, &overlay)
            .map_err(LifecycleError::Picker)?
    };
    let capability = read_private_control_file(&install_manifest.caller_token_path)?;
    let capability = validate_capability_bytes(&capability)?;
    let config_path = request.codex_home.join("config.toml");
    let generated_path = request
        .install_root
        .join("state")
        .join(PICKER_CATALOG_FILE_NAME);
    let native_route_path = request
        .install_root
        .join("state")
        .join(PICKER_NATIVE_ROUTE_FILE_NAME);
    let state_path = request
        .install_root
        .join("state")
        .join(PICKER_STATE_FILE_NAME);

    let prior_state = read_picker_state(&state_path)?;
    let previous_generated = read_optional_control_file(&generated_path, &[0o600])?;
    let previous_native_route = read_optional_control_file(&native_route_path, &[0o600])?;
    let previous_state = read_optional_control_file(&state_path, &[0o600])?;
    let config_plan = prepare_picker_config(
        &config_path,
        prior_state.as_ref(),
        &generated_path,
        request.bind,
        capability,
    )?;
    let (generated, native_route) = if request.native_compatibility {
        let fallback = native_compatibility_fallback(
            &config_plan.render_base,
            generated.native_model_slugs(),
        )?;
        let native_route = NativeRouteState::new_native_compatibility(
            request.native_upstream,
            generated.native_model_slugs().iter().cloned(),
            fallback.clone(),
        )?;
        let generated = generated
            .with_hidden_grok_compatibility(&grok_catalog, &fallback)
            .map_err(LifecycleError::Picker)?;
        (generated, native_route)
    } else {
        let native_route = NativeRouteState::new(
            request.native_upstream,
            generated.native_model_slugs().iter().cloned(),
        )?;
        (generated, native_route)
    };
    let native_route_bytes = native_route.to_json().map_err(LifecycleError::Native)?;
    let prepared = (|| {
        let config_contents = render_picker_config(
            &config_plan.render_base,
            &generated_path,
            request.bind,
            capability,
        )?;
        let generated_identity = artifact_identity(&generated_path, generated.bytes())?;
        let native_route_identity = artifact_identity(&native_route_path, &native_route_bytes)?;
        let config_identity = artifact_identity(&config_path, config_contents.as_bytes())?;
        let state = PickerManagedState::new(
            artifact_identity(&request.native_catalog_path, &native_bytes)?,
            &grok_catalog,
            generated_identity,
            native_route_identity,
            config_identity,
            config_plan.rollback.clone(),
        )
        .map_err(LifecycleError::Picker)?;
        let state_bytes = state.to_json().map_err(LifecycleError::Picker)?;
        Ok::<_, LifecycleError>((config_contents, state_bytes))
    })();
    let (config_contents, state_bytes) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            config_plan.abort()?;
            return Err(error);
        }
    };

    if let Err(error) = atomic_write(&generated_path, generated.bytes(), 0o600) {
        config_plan.abort()?;
        return Err(error);
    }
    if let Err(error) = atomic_write(&native_route_path, &native_route_bytes, 0o600) {
        restore_optional_file(&generated_path, previous_generated.as_deref(), 0o600)?;
        config_plan.abort()?;
        return Err(error);
    }
    if let Err(error) = atomic_write(&config_path, config_contents.as_bytes(), 0o600) {
        restore_optional_file(&native_route_path, previous_native_route.as_deref(), 0o600)?;
        restore_optional_file(&generated_path, previous_generated.as_deref(), 0o600)?;
        config_plan.abort()?;
        return Err(error);
    }

    if let Err(error) = atomic_write(&state_path, &state_bytes, 0o600) {
        restore_optional_file(
            &config_path,
            config_plan.current.as_deref(),
            config_plan.previous_mode,
        )?;
        restore_optional_file(&generated_path, previous_generated.as_deref(), 0o600)?;
        restore_optional_file(&native_route_path, previous_native_route.as_deref(), 0o600)?;
        restore_optional_file(&state_path, previous_state.as_deref(), 0o600)?;
        config_plan.abort()?;
        return Err(error);
    }

    config_plan.finish()?;
    Ok(PickerInstallReceipt {
        generated_catalog_path: generated_path,
        native_route_path,
        managed_state_path: state_path,
        config_path,
        native_model_count: generated.native_model_count(),
        grok_model_count: generated.grok_model_count(),
    })
}

/// Reconciles an already-installed picker after Codex refreshes its Native
/// `models_cache.json` or the bridge refreshes its admitted Grok catalog.
///
/// This path intentionally leaves Codex configuration and both authoritative
/// inputs untouched. It updates the generated picker, Native routing state,
/// and their rollback identities as one recoverable operation.
pub fn sync_installed_picker_catalog(
    grok_catalog_path: &Path,
) -> Result<Option<PickerCatalogSyncReceipt>, LifecycleError> {
    validate_absolute_clean(grok_catalog_path)?;
    let state_dir = grok_catalog_path
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "state"))
        .ok_or(LifecycleError::InvalidManifest)?;
    let install_root = state_dir.parent().ok_or(LifecycleError::InvalidManifest)?;
    validate_absolute_clean(install_root)?;

    let (install_manifest, _) = read_manifest(install_root)?;
    if install_manifest.install_root != install_root
        || install_manifest.catalog_cache_path != grok_catalog_path
    {
        return Err(LifecycleError::InvalidManifest);
    }

    let generated_path = state_dir.join(PICKER_CATALOG_FILE_NAME);
    let native_route_path = state_dir.join(PICKER_NATIVE_ROUTE_FILE_NAME);
    let state_path = state_dir.join(PICKER_STATE_FILE_NAME);
    let Some(previous_state_bytes) = read_optional_control_file(&state_path, &[0o600])? else {
        return Ok(None);
    };
    let previous_state =
        PickerManagedState::from_json(&previous_state_bytes).map_err(LifecycleError::Picker)?;
    if previous_state.generated_catalog().path() != generated_path
        || previous_state.native_route().path() != native_route_path
    {
        return Err(LifecycleError::InvalidPickerState);
    }
    validate_identity(previous_state.generated_catalog())?;
    validate_identity(previous_state.native_route())?;

    let native_catalog_path = previous_state.native_catalog().path().to_path_buf();
    let native_bytes = read_regular_file(&native_catalog_path, MAX_CONTROL_FILE_BYTES)?;
    let (grok_catalog, grok_catalog_bytes) = CatalogCache::new(grok_catalog_path)
        .load_with_bytes()
        .map_err(LifecycleError::Catalog)?
        .ok_or(LifecycleError::PickerCatalogUnavailable)?;
    let previous_generated = read_private_control_file(&generated_path)?;
    let previous_native_route = read_private_control_file(&native_route_path)?;
    let current_native_route =
        NativeRouteState::from_json(&previous_native_route).map_err(LifecycleError::Native)?;

    let capability = read_private_control_file(&install_manifest.caller_token_path)?;
    let capability = validate_capability_bytes(&capability)?;
    let bind = install_manifest
        .bind
        .parse::<SocketAddr>()
        .map_err(|_| LifecycleError::InvalidManifest)?;
    let config_path = previous_state.managed_config().path();
    let config_plan = prepare_picker_config(
        config_path,
        Some(&previous_state),
        &generated_path,
        bind,
        capability,
    )?;

    let prepared = (|| {
        let native_catalog_identity = artifact_identity(&native_catalog_path, &native_bytes)?;
        let grok_catalog_identity = artifact_identity(grok_catalog_path, &grok_catalog_bytes)?;
        let native_compatibility = current_native_route
            .native_compatibility_fallback()
            .is_some();
        let generated = if native_compatibility {
            generate_native_compatibility_catalog(&native_bytes).map_err(LifecycleError::Picker)?
        } else {
            let overlay = crate::picker::load_grok_overlay(
                &install_manifest.launcher_path.join(LAUNCHER_OVERLAY_PATH),
            )
            .map_err(LifecycleError::Picker)?;
            generate_picker_catalog(&native_bytes, &grok_catalog, &overlay)
                .map_err(LifecycleError::Picker)?
        };
        let (generated, native_route) = if native_compatibility {
            let fallback = native_compatibility_fallback(
                &config_plan.render_base,
                generated.native_model_slugs(),
            )?;
            let native_route = NativeRouteState::new_native_compatibility(
                current_native_route.upstream(),
                generated.native_model_slugs().iter().cloned(),
                fallback.clone(),
            )?;
            let generated = generated
                .with_hidden_grok_compatibility(&grok_catalog, &fallback)
                .map_err(LifecycleError::Picker)?;
            (generated, native_route)
        } else {
            let native_route = NativeRouteState::new(
                current_native_route.upstream(),
                generated.native_model_slugs().iter().cloned(),
            )?;
            (generated, native_route)
        };
        let native_route_bytes = native_route.to_json().map_err(LifecycleError::Native)?;
        let current_config = config_plan
            .current
            .as_deref()
            .ok_or(LifecycleError::InvalidPickerState)?;
        let state = PickerManagedState::new(
            native_catalog_identity.clone(),
            &grok_catalog,
            artifact_identity(&generated_path, generated.bytes())?,
            artifact_identity(&native_route_path, &native_route_bytes)?,
            artifact_identity(config_path, current_config)?,
            config_plan.rollback.clone(),
        )
        .map_err(LifecycleError::Picker)?;
        let state_bytes = state.to_json().map_err(LifecycleError::Picker)?;
        Ok::<_, LifecycleError>((
            generated,
            native_route,
            native_route_bytes,
            state_bytes,
            native_catalog_identity,
            grok_catalog_identity,
        ))
    })();
    let (
        generated,
        native_route,
        native_route_bytes,
        state_bytes,
        native_catalog_identity,
        grok_catalog_identity,
    ) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            config_plan.abort()?;
            return Err(error);
        }
    };

    let generated_changed = generated.bytes() != previous_generated;
    let native_route_changed = native_route_bytes != previous_native_route;
    let state_changed = state_bytes != previous_state_bytes;
    if generated_changed {
        if let Err(error) = atomic_write(&generated_path, generated.bytes(), 0o600) {
            config_plan.abort()?;
            return Err(error);
        }
    }
    if native_route_changed {
        if let Err(error) = atomic_write(&native_route_path, &native_route_bytes, 0o600) {
            if generated_changed {
                restore_optional_file(&generated_path, Some(&previous_generated), 0o600)?;
            }
            config_plan.abort()?;
            return Err(error);
        }
    }
    if state_changed {
        if let Err(error) = atomic_write(&state_path, &state_bytes, 0o600) {
            if native_route_changed {
                restore_optional_file(&native_route_path, Some(&previous_native_route), 0o600)?;
            }
            if generated_changed {
                restore_optional_file(&generated_path, Some(&previous_generated), 0o600)?;
            }
            config_plan.abort()?;
            return Err(error);
        }
    }

    config_plan.finish()?;
    Ok(Some(PickerCatalogSyncReceipt {
        changed: generated_changed || native_route_changed,
        native_catalog: native_catalog_identity,
        grok_catalog: grok_catalog_identity,
        native_route,
        native_model_count: generated.native_model_count(),
        grok_model_ids: grok_catalog.model_ids().to_vec(),
    }))
}

pub fn uninstall_picker(install_root: &Path, codex_home: &Path) -> Result<bool, LifecycleError> {
    validate_absolute_clean(install_root)?;
    validate_absolute_clean(codex_home)?;
    uninstall_picker_if_present(install_root, codex_home)
}

fn preflight_uninstall_picker_if_present(
    install_root: &Path,
    codex_home: &Path,
) -> Result<(), LifecycleError> {
    let state_path = install_root.join("state").join(PICKER_STATE_FILE_NAME);
    let Some(bytes) = read_optional_control_file(&state_path, &[0o600])? else {
        return Ok(());
    };
    let state = PickerManagedState::from_json(&bytes).map_err(LifecycleError::Picker)?;
    let config_path = codex_home.join("config.toml");
    validate_picker_uninstall_inputs(install_root, &config_path, &state)?;
    preflight_picker_config_rollback(install_root, &config_path, &state)
}

fn uninstall_picker_if_present(
    install_root: &Path,
    codex_home: &Path,
) -> Result<bool, LifecycleError> {
    let state_path = install_root.join("state").join(PICKER_STATE_FILE_NAME);
    let Some(bytes) = read_optional_control_file(&state_path, &[0o600])? else {
        return Ok(false);
    };
    let state = PickerManagedState::from_json(&bytes).map_err(LifecycleError::Picker)?;
    let config_path = codex_home.join("config.toml");
    validate_picker_uninstall_inputs(install_root, &config_path, &state)?;
    rollback_picker_config(install_root, &config_path, &state)?;
    fs::remove_file(state.generated_catalog().path()).map_err(LifecycleError::RemoveInstallRoot)?;
    fs::remove_file(state.native_route().path()).map_err(LifecycleError::RemoveInstallRoot)?;
    fs::remove_file(&state_path).map_err(LifecycleError::RemoveInstallRoot)?;
    sync_parent(&state_path)?;
    Ok(true)
}

fn validate_picker_uninstall_inputs(
    install_root: &Path,
    config_path: &Path,
    state: &PickerManagedState,
) -> Result<(), LifecycleError> {
    if state.managed_config().path() != config_path
        || state.generated_catalog().path()
            != install_root.join("state").join(PICKER_CATALOG_FILE_NAME)
        || state.native_route().path()
            != install_root
                .join("state")
                .join(PICKER_NATIVE_ROUTE_FILE_NAME)
    {
        return Err(LifecycleError::InvalidPickerState);
    }
    validate_read_only_picker_input(state.native_catalog())?;
    validate_identity(state.generated_catalog())?;
    validate_identity(state.native_route())?;
    Ok(())
}

fn preflight_picker_config_rollback(
    install_root: &Path,
    config_path: &Path,
    state: &PickerManagedState,
) -> Result<(), LifecycleError> {
    let metadata =
        fs::symlink_metadata(config_path).map_err(LifecycleError::InspectExternalTarget)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LifecycleError::UnsafeExternalTarget);
    }
    let mode = file_mode(&metadata)?;
    if !matches!(mode, 0o600 | 0o644) {
        return Err(LifecycleError::UnsafeExternalPermissions);
    }
    let current = read_control_file_with_modes(config_path, &[mode], MAX_CONTROL_FILE_BYTES)?;
    if artifact_identity(config_path, &current)? == *state.managed_config() {
        if let ConfigRollbackOwnership::RestoreExactBackup { backup, .. } = state.config_rollback()
        {
            validate_identity(backup)?;
        }
        return Ok(());
    }

    let expectation = picker_config_expectation(install_root)?;
    remove_rewritten_picker_config(
        &current,
        state.generated_catalog().path(),
        expectation.bind,
        &expectation.capability,
    )?;
    if let ConfigRollbackOwnership::RestoreExactBackup { backup, .. } = state.config_rollback() {
        validate_identity(backup)?;
    }
    Ok(())
}

fn rollback_picker_config(
    install_root: &Path,
    config_path: &Path,
    state: &PickerManagedState,
) -> Result<(), LifecycleError> {
    let metadata =
        fs::symlink_metadata(config_path).map_err(LifecycleError::InspectExternalTarget)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LifecycleError::UnsafeExternalTarget);
    }
    let mode = file_mode(&metadata)?;
    if !matches!(mode, 0o600 | 0o644) {
        return Err(LifecycleError::UnsafeExternalPermissions);
    }
    let current = read_control_file_with_modes(config_path, &[mode], MAX_CONTROL_FILE_BYTES)?;
    if artifact_identity(config_path, &current)? == *state.managed_config() {
        return restore_exact_picker_config(config_path, state.config_rollback());
    }

    let expectation = picker_config_expectation(install_root)?;
    let recovered = remove_rewritten_picker_config(
        &current,
        &state.generated_catalog().path(),
        expectation.bind,
        &expectation.capability,
    )?;
    atomic_write(config_path, &recovered, mode)?;
    remove_picker_config_backup(state.config_rollback())
}

struct PickerConfigExpectation {
    bind: SocketAddr,
    capability: String,
}

fn picker_config_expectation(
    install_root: &Path,
) -> Result<PickerConfigExpectation, LifecycleError> {
    let (manifest, raw_manifest) = read_manifest(install_root)?;
    if manifest.version != MANIFEST_VERSION
        || manifest.install_root != install_root
        || manifest.caller_token_path != install_root.join("secrets").join("caller-capability")
    {
        return Err(LifecycleError::InvalidPickerState);
    }
    let bind: SocketAddr = manifest
        .bind
        .parse()
        .map_err(|_| LifecycleError::InvalidPickerState)?;
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Err(LifecycleError::InvalidPickerState);
    }
    let capability = read_private_control_file(&manifest.caller_token_path)?;
    let capability = validate_capability_bytes(&capability)?;
    if raw_manifest.contains(capability) {
        return Err(LifecycleError::ManifestContainsCapability);
    }
    Ok(PickerConfigExpectation {
        bind,
        capability: capability.to_owned(),
    })
}

fn restore_exact_picker_config(
    config_path: &Path,
    rollback: &ConfigRollbackOwnership,
) -> Result<(), LifecycleError> {
    match rollback {
        ConfigRollbackOwnership::RemoveCreated => {
            fs::remove_file(config_path).map_err(LifecycleError::RestoreExternalTarget)?;
            sync_parent(config_path)
        }
        ConfigRollbackOwnership::RestoreExactBackup {
            backup,
            original_mode,
        } => {
            validate_identity(backup)?;
            let original = read_private_control_file(backup.path())?;
            atomic_write(config_path, &original, *original_mode)?;
            fs::remove_file(backup.path()).map_err(LifecycleError::RemoveBackup)?;
            sync_parent(backup.path())
        }
    }
}

fn remove_picker_config_backup(rollback: &ConfigRollbackOwnership) -> Result<(), LifecycleError> {
    if let ConfigRollbackOwnership::RestoreExactBackup { backup, .. } = rollback {
        validate_identity(backup)?;
        fs::remove_file(backup.path()).map_err(LifecycleError::RemoveBackup)?;
        sync_parent(backup.path())?;
    }
    Ok(())
}

pub fn doctor(request: &DoctorRequest) -> Result<DoctorReport, LifecycleError> {
    validate_absolute_clean(&request.install_root)?;
    validate_absolute_clean(&request.codex_home)?;
    validate_absolute_clean(&request.launch_agent_path)?;
    validate_absolute_clean(&request.credential_path)?;

    let mut checks = Vec::new();
    let manifest_result = read_manifest(&request.install_root).and_then(|(manifest, raw)| {
        validate_manifest(
            &manifest,
            &UninstallRequest {
                install_root: request.install_root.clone(),
                codex_home: request.codex_home.clone(),
                launch_agent_path: request.launch_agent_path.clone(),
            },
        )?;
        Ok((manifest, raw))
    });
    let (manifest, raw_manifest) = match manifest_result {
        Ok(value) => {
            checks.push(passed("manifest", "install manifest is valid"));
            value
        }
        Err(_) => {
            checks.push(failed("manifest", "install manifest is missing or invalid"));
            return Ok(DoctorReport { checks });
        }
    };

    check(
        &mut checks,
        "binary",
        "installed binary is a regular executable",
        "installed binary is missing, unsafe, or not executable",
        validate_installed_binary(&manifest.binary_path),
    );

    let token = read_private_control_file(&manifest.caller_token_path)
        .and_then(|bytes| validate_capability_bytes(&bytes).map(|value| value.to_owned()));
    check(
        &mut checks,
        "caller_capability",
        "caller capability is private and valid",
        "caller capability is missing, unsafe, or invalid",
        token.as_ref().map(|_| ()).map_err(|_| ()),
    );

    let runtime = validate_runtime_config(&manifest);
    check(
        &mut checks,
        "runtime_config",
        "runtime configuration is valid and loopback-only",
        "runtime configuration is invalid or is not loopback-only",
        runtime.map_err(|_| ()),
    );

    check(
        &mut checks,
        "catalog_path",
        "catalog cache remains inside bridge-owned state",
        "catalog cache path or file ownership is unsafe",
        validate_catalog_path(&manifest).map_err(|_| ()),
    );

    let profile_result = token.as_ref().map_err(|_| ()).and_then(|capability| {
        validate_current_profile(&manifest, capability)
            .map(|_| ())
            .map_err(|_| ())
    });
    check(
        &mut checks,
        "codex_profile",
        "isolated Codex provider profile is exact and valid",
        "isolated Codex provider profile is missing, unsafe, or altered",
        profile_result,
    );

    check(
        &mut checks,
        "backups",
        "recorded external backups are present and private",
        "a recorded external backup is missing or unsafe",
        validate_recorded_backups(&manifest).map_err(|_| ()),
    );

    check(
        &mut checks,
        "launch_agent",
        "LaunchAgent file matches the installed manifest",
        "LaunchAgent file is missing, unsafe, or altered",
        validate_current_launch_agent(&manifest).map_err(|_| ()),
    );

    let manifest_has_secret = token
        .as_ref()
        .is_ok_and(|capability| raw_manifest.contains(capability));
    check(
        &mut checks,
        "manifest_secrets",
        "install manifest contains no caller capability",
        "install manifest contains caller capability material",
        if manifest_has_secret { Err(()) } else { Ok(()) },
    );

    let credential = CredentialStore::new(request.credential_path.clone())
        .ok()
        .map_or(
            AuthStatus {
                availability: AuthAvailability::Unavailable,
                message: "Grok session credential is unavailable or invalid",
            },
            |store| auth_status(&store),
        );
    check(
        &mut checks,
        "grok_credential",
        "Grok credential is available and valid",
        "Grok credential is unavailable, unsafe, expired, or invalid",
        if credential.availability == AuthAvailability::Available {
            Ok(())
        } else {
            Err(())
        },
    );

    Ok(DoctorReport { checks })
}

fn passed(id: &'static str, message: &'static str) -> DoctorCheck {
    DoctorCheck {
        id,
        status: DoctorCheckStatus::Passed,
        message,
    }
}

fn failed(id: &'static str, message: &'static str) -> DoctorCheck {
    DoctorCheck {
        id,
        status: DoctorCheckStatus::Failed,
        message,
    }
}

fn check<E>(
    checks: &mut Vec<DoctorCheck>,
    id: &'static str,
    pass_message: &'static str,
    fail_message: &'static str,
    result: Result<(), E>,
) {
    checks.push(if result.is_ok() {
        passed(id, pass_message)
    } else {
        failed(id, fail_message)
    });
}

struct PickerConfigPlan {
    current: Option<Vec<u8>>,
    render_base: Vec<u8>,
    previous_mode: u32,
    rollback: ConfigRollbackOwnership,
    fresh_backup: Option<PathBuf>,
    superseded_backup: Option<ArtifactIdentity>,
}

impl PickerConfigPlan {
    fn finish(self) -> Result<(), LifecycleError> {
        // A rewrite-resilient update snapshots the current foreign-only
        // configuration before it writes a new managed block. Once that new
        // state is published, its predecessor is no longer a rollback receipt.
        if let Some(backup) = self.superseded_backup {
            validate_identity(&backup)?;
            fs::remove_file(backup.path()).map_err(LifecycleError::RemoveBackup)?;
            sync_parent(backup.path())?;
        }
        Ok(())
    }

    fn abort(self) -> Result<(), LifecycleError> {
        if let Some(path) = self.fresh_backup {
            fs::remove_file(&path).map_err(LifecycleError::RemoveBackup)?;
            sync_parent(&path)?;
        }
        Ok(())
    }
}

fn prepare_picker_config(
    config_path: &Path,
    previous: Option<&PickerManagedState>,
    generated_catalog: &Path,
    bind: SocketAddr,
    capability: &str,
) -> Result<PickerConfigPlan, LifecycleError> {
    if let Some(previous) = previous {
        if previous.managed_config().path() != config_path {
            return Err(LifecycleError::InvalidPickerState);
        }
        let metadata =
            fs::symlink_metadata(config_path).map_err(LifecycleError::InspectExternalTarget)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(LifecycleError::UnsafeExternalTarget);
        }
        let mode = file_mode(&metadata)?;
        if !matches!(mode, 0o600 | 0o644) {
            return Err(LifecycleError::UnsafeExternalPermissions);
        }
        let current = read_control_file_with_modes(config_path, &[mode], MAX_CONTROL_FILE_BYTES)?;
        if artifact_identity(config_path, &current)? == *previous.managed_config() {
            return Ok(PickerConfigPlan {
                render_base: current.clone(),
                current: Some(current),
                previous_mode: mode,
                rollback: previous.config_rollback().clone(),
                fresh_backup: None,
                superseded_backup: None,
            });
        }

        let render_base =
            remove_rewritten_picker_config(&current, generated_catalog, bind, capability)?;
        let backup_path = unique_backup_path(config_path)?;
        write_new_file(&backup_path, &render_base, 0o600)?;
        let backup = artifact_identity(&backup_path, &render_base)?;
        let superseded_backup = match previous.config_rollback() {
            ConfigRollbackOwnership::RemoveCreated => None,
            ConfigRollbackOwnership::RestoreExactBackup { backup, .. } => {
                validate_identity(backup)?;
                Some(backup.clone())
            }
        };
        return Ok(PickerConfigPlan {
            current: Some(current),
            render_base,
            previous_mode: mode,
            rollback: ConfigRollbackOwnership::RestoreExactBackup {
                backup,
                original_mode: mode,
            },
            fresh_backup: Some(backup_path),
            superseded_backup,
        });
    }

    match fs::symlink_metadata(config_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(LifecycleError::UnsafeExternalTarget);
            }
            let mode = file_mode(&metadata)?;
            if !matches!(mode, 0o600 | 0o644) {
                return Err(LifecycleError::UnsafeExternalPermissions);
            }
            let current = read_regular_file(config_path, MAX_CONTROL_FILE_BYTES)?;
            let source =
                std::str::from_utf8(&current).map_err(|_| LifecycleError::InvalidPickerConfig)?;
            let parsed: toml::Value =
                toml::from_str(source).map_err(|_| LifecycleError::InvalidPickerConfig)?;
            if picker_marker_count(source) != 0
                || parsed.get("model_catalog_json").is_some()
                || parsed.get("openai_base_url").is_some()
                || parsed.get("model_provider").is_some()
                || parsed.get("model_providers").is_some_and(|providers| {
                    providers
                        .as_table()
                        .is_none_or(|providers| providers.contains_key(PICKER_PROVIDER_ID))
                })
            {
                return Err(LifecycleError::PickerConfigConflict);
            }
            let backup_path = unique_backup_path(config_path)?;
            write_new_file(&backup_path, &current, 0o600)?;
            let backup = artifact_identity(&backup_path, &current)?;
            Ok(PickerConfigPlan {
                render_base: current.clone(),
                current: Some(current),
                previous_mode: mode,
                rollback: ConfigRollbackOwnership::RestoreExactBackup {
                    backup,
                    original_mode: mode,
                },
                fresh_backup: Some(backup_path),
                superseded_backup: None,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            validate_existing_parent(config_path)?;
            Ok(PickerConfigPlan {
                current: None,
                render_base: Vec::new(),
                previous_mode: 0o600,
                rollback: ConfigRollbackOwnership::RemoveCreated,
                fresh_backup: None,
                superseded_backup: None,
            })
        }
        Err(error) => Err(LifecycleError::InspectExternalTarget(error)),
    }
}

fn remove_rewritten_picker_config(
    current: &[u8],
    generated_catalog: &Path,
    bind: SocketAddr,
    capability: &str,
) -> Result<Vec<u8>, LifecycleError> {
    let source = std::str::from_utf8(current).map_err(|_| LifecycleError::InvalidPickerConfig)?;
    if source.matches(PICKER_BEGIN).count() > 1 || source.matches(PICKER_END).count() > 1 {
        return Err(LifecycleError::InvalidPickerState);
    }
    let mut parsed: toml::Value =
        toml::from_str(source).map_err(|_| LifecycleError::InvalidPickerConfig)?;
    let table = parsed
        .as_table_mut()
        .ok_or(LifecycleError::InvalidPickerConfig)?;
    let expected = picker_config_value(generated_catalog, bind, capability)?;
    let expected_table = expected
        .as_table()
        .expect("generated picker TOML is always a root table");
    for key in ["model_provider", "model_catalog_json"] {
        if table.get(key) != expected_table.get(key) {
            return Err(LifecycleError::InvalidPickerState);
        }
    }
    let providers = table
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .ok_or(LifecycleError::InvalidPickerState)?;
    let provider = providers
        .get(PICKER_PROVIDER_ID)
        .ok_or(LifecycleError::InvalidPickerState)?;
    if !picker_provider_matches(provider, expected_table, bind, capability)? {
        return Err(LifecycleError::InvalidPickerState);
    }

    table.remove("model_provider");
    table.remove("model_catalog_json");
    let providers = table
        .get_mut("model_providers")
        .and_then(toml::Value::as_table_mut)
        .expect("validated picker provider table must remain a TOML table");
    providers.remove(PICKER_PROVIDER_ID);
    toml::to_string(&parsed)
        .map_err(LifecycleError::SerializeConfig)
        .map(Vec::from)
}

fn picker_config_value(
    generated_catalog: &Path,
    bind: SocketAddr,
    capability: &str,
) -> Result<toml::Value, LifecycleError> {
    let rendered = render_picker_config(&[], generated_catalog, bind, capability)?;
    toml::from_str(&rendered).map_err(|_| LifecycleError::InvalidPickerConfig)
}

fn native_compatibility_fallback(
    config: &[u8],
    native_model_slugs: &[String],
) -> Result<String, LifecycleError> {
    let source = std::str::from_utf8(config).map_err(|_| LifecycleError::InvalidPickerConfig)?;
    let parsed: toml::Value =
        toml::from_str(source).map_err(|_| LifecycleError::InvalidPickerConfig)?;
    let model = parsed
        .get("model")
        .and_then(toml::Value::as_str)
        .filter(|model| !model.is_empty())
        .ok_or(LifecycleError::NativeCompatibilityFallbackUnavailable)?;
    if !native_model_slugs
        .iter()
        .any(|candidate| candidate == model)
    {
        return Err(LifecycleError::NativeCompatibilityFallbackUnavailable);
    }
    Ok(model.to_owned())
}

fn picker_provider_matches(
    provider: &toml::Value,
    expected: &toml::map::Map<String, toml::Value>,
    bind: SocketAddr,
    capability: &str,
) -> Result<bool, LifecycleError> {
    let current_header = expected
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(PICKER_PROVIDER_ID))
        .expect("generated picker TOML must include its provider");
    if provider == current_header {
        return Ok(true);
    }

    let legacy_base_url = format!("http://{bind}/_grok/{capability}/v1");
    let legacy = format!(
        "[model_providers.{PICKER_PROVIDER_ID}]\nname = {PICKER_PROVIDER_NAME:?}\nbase_url = {legacy_base_url:?}\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\n"
    );
    let legacy: toml::Value =
        toml::from_str(&legacy).map_err(|_| LifecycleError::InvalidPickerConfig)?;
    let legacy_provider = legacy
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(PICKER_PROVIDER_ID))
        .expect("legacy picker TOML must include its provider");
    Ok(provider == legacy_provider)
}

fn render_picker_config(
    current: &[u8],
    generated_catalog: &Path,
    bind: SocketAddr,
    capability: &str,
) -> Result<String, LifecycleError> {
    let source = std::str::from_utf8(current).map_err(|_| LifecycleError::InvalidPickerConfig)?;
    let generated_catalog = path_text(generated_catalog)?;
    let base_url = format!("http://{bind}/v1");
    let block = format!(
        "{PICKER_BEGIN}\nmodel_provider = {PICKER_PROVIDER_ID:?}\nmodel_catalog_json = {generated_catalog:?}\n\n[model_providers.{PICKER_PROVIDER_ID}]\nname = {PICKER_PROVIDER_NAME:?}\nbase_url = {base_url:?}\nhttp_headers = {{ {PICKER_CALLER_HEADER:?} = {capability:?} }}\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\nrequest_max_retries = 0\nstream_max_retries = 0\n{PICKER_END}\n"
    );
    let markers = picker_marker_count(source);
    if markers > 1 {
        return Err(LifecycleError::PickerConfigConflict);
    }
    if markers == 1 {
        let begin = source
            .find(PICKER_BEGIN)
            .ok_or(LifecycleError::PickerConfigConflict)?;
        let end = source
            .find(PICKER_END)
            .ok_or(LifecycleError::PickerConfigConflict)?
            + PICKER_END.len();
        return Ok(format!("{}{}{}", &source[..begin], block, &source[end..]));
    }
    let mut insertion = source.len();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            insertion = offset;
            break;
        }
        offset += line.len();
    }
    Ok(format!(
        "{}{}{}",
        &source[..insertion],
        block,
        &source[insertion..]
    ))
}

fn picker_marker_count(source: &str) -> usize {
    match (
        source.matches(PICKER_BEGIN).count(),
        source.matches(PICKER_END).count(),
    ) {
        (0, 0) => 0,
        (1, 1) => 1,
        _ => 2,
    }
}

fn artifact_identity(path: &Path, bytes: &[u8]) -> Result<ArtifactIdentity, LifecycleError> {
    ArtifactIdentity::new(
        path.to_path_buf(),
        bytes.len() as u64,
        format!("{:x}", Sha256::digest(bytes)),
    )
    .map_err(LifecycleError::Picker)
}

fn validate_identity(identity: &ArtifactIdentity) -> Result<(), LifecycleError> {
    let bytes = read_regular_file(identity.path(), MAX_CONTROL_FILE_BYTES)?;
    let observed = artifact_identity(identity.path(), &bytes)?;
    if &observed != identity {
        return Err(LifecycleError::InvalidPickerState);
    }
    Ok(())
}

fn validate_read_only_picker_input(identity: &ArtifactIdentity) -> Result<(), LifecycleError> {
    // The native Codex model cache belongs to Codex. It is an external input,
    // so uninstall proves only that its recorded path remains safe to read;
    // Desktop is free to refresh its bytes while picker state is installed.
    let _ = read_regular_file(identity.path(), MAX_CONTROL_FILE_BYTES)?;
    Ok(())
}

fn read_picker_state(path: &Path) -> Result<Option<PickerManagedState>, LifecycleError> {
    read_optional_control_file(path, &[0o600])?
        .map(|bytes| PickerManagedState::from_json(&bytes).map_err(LifecycleError::Picker))
        .transpose()
}

fn read_optional_control_file(
    path: &Path,
    modes: &[u32],
) -> Result<Option<Vec<u8>>, LifecycleError> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_control_file_with_modes(path, modes, MAX_CONTROL_FILE_BYTES).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(LifecycleError::InspectFile(error)),
    }
}

fn restore_optional_file(
    path: &Path,
    previous: Option<&[u8]>,
    mode: u32,
) -> Result<(), LifecycleError> {
    if let Some(previous) = previous {
        atomic_write(path, previous, mode)
    } else {
        match fs::remove_file(path) {
            Ok(()) => sync_parent(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(LifecycleError::RestoreExternalTarget(error)),
        }
    }
}

struct InstallPaths {
    binary: PathBuf,
    launcher: PathBuf,
    config: PathBuf,
    catalog: PathBuf,
    capability: PathBuf,
    logs: PathBuf,
    profile: PathBuf,
}

impl InstallPaths {
    fn validate(request: &InstallRequest) -> Result<Self, LifecycleError> {
        validate_absolute_clean(&request.source_binary)?;
        validate_absolute_clean(&request.source_launcher)?;
        validate_absolute_clean(&request.install_root)?;
        validate_absolute_clean(&request.codex_home)?;
        validate_absolute_clean(&request.launch_agent_path)?;
        validate_safe_root(&request.install_root, &request.codex_home)?;
        if request.install_root.exists() {
            return Err(LifecycleError::InstallRootAlreadyExists);
        }
        validate_existing_parent(&request.install_root)?;
        validate_existing_directory(&request.codex_home, || LifecycleError::UnsafeCodexHome)?;
        validate_existing_parent(&request.launch_agent_path)?;
        if !request.bind.ip().is_loopback() {
            return Err(LifecycleError::NonLoopbackBind);
        }
        if request.bind.port() == 0 {
            return Err(LifecycleError::ZeroPort);
        }
        validate_model(&request.initial_model)?;
        validate_launch_label(&request.launch_agent_label)?;
        if request.launch_agent_contents.is_empty()
            || request.launch_agent_contents.len() as u64 > MAX_CONTROL_FILE_BYTES
        {
            return Err(LifecycleError::InvalidLaunchAgentContents);
        }
        let profile = request.codex_home.join(PROFILE_FILE_NAME);
        if request.launch_agent_path == profile
            || request.launch_agent_path.starts_with(&request.install_root)
        {
            return Err(LifecycleError::OverlappingManagedPaths);
        }
        Ok(Self {
            binary: request.install_root.join("bin").join(BINARY_FILE_NAME),
            launcher: request.install_root.join(LAUNCHER_APP_PATH),
            config: request.install_root.join("config").join("bridge.toml"),
            catalog: request.install_root.join("state").join("models.json"),
            capability: request
                .install_root
                .join("secrets")
                .join("caller-capability"),
            logs: request.install_root.join("logs"),
            profile,
        })
    }
}

struct StagePaths {
    binary: PathBuf,
    launcher: PathBuf,
    config: PathBuf,
    catalog: PathBuf,
    capability: PathBuf,
    manifest: PathBuf,
}

impl StagePaths {
    fn new(root: &Path) -> Self {
        Self {
            binary: root.join("bin").join(BINARY_FILE_NAME),
            launcher: root.join(LAUNCHER_APP_PATH),
            config: root.join("config").join("bridge.toml"),
            catalog: root.join("state").join("models.json"),
            capability: root.join("secrets").join("caller-capability"),
            manifest: root.join(MANIFEST_FILE_NAME),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupRecord {
    path: PathBuf,
    original_mode: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallManifest {
    version: u32,
    install_root: PathBuf,
    binary_path: PathBuf,
    launcher_path: PathBuf,
    config_path: PathBuf,
    catalog_cache_path: PathBuf,
    caller_token_path: PathBuf,
    logs_dir: PathBuf,
    profile_path: PathBuf,
    profile_backup: Option<BackupRecord>,
    profile_created: bool,
    launch_agent_path: PathBuf,
    launch_agent_label: String,
    launch_agent_contents: String,
    launch_agent_mode: u32,
    launch_agent_backup: Option<BackupRecord>,
    launch_agent_created: bool,
    bind: String,
    initial_model: String,
}

#[derive(Serialize)]
struct RuntimeFile<'a> {
    version: u32,
    server: RuntimeServerFile<'a>,
    grok: RuntimeGrokFile<'a>,
}

#[derive(Serialize)]
struct RuntimeServerFile<'a> {
    bind: String,
    capability_token_file: &'a str,
}

#[derive(Serialize)]
struct RuntimeGrokFile<'a> {
    catalog_cache_file: &'a str,
    refresh_on_start: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeFileRead {
    version: u32,
    server: RuntimeServerFileRead,
    grok: RuntimeGrokFileRead,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeServerFileRead {
    bind: SocketAddr,
    capability_token_file: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeGrokFileRead {
    catalog_cache_file: PathBuf,
    refresh_on_start: bool,
}

#[derive(Serialize)]
struct ProfileFile<'a> {
    model: &'a str,
    model_provider: &'static str,
    model_providers: ProfileProviders<'a>,
}

#[derive(Serialize)]
struct ProfileProviders<'a> {
    grok_bridge: ProfileProvider<'a>,
}

#[derive(Serialize)]
struct ProfileProvider<'a> {
    name: &'static str,
    base_url: &'a str,
    wire_api: &'static str,
    requires_openai_auth: bool,
    supports_websockets: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFileRead {
    model: String,
    model_provider: String,
    model_providers: ProfileProvidersRead,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileProvidersRead {
    grok_bridge: ProfileProviderRead,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileProviderRead {
    name: String,
    base_url: String,
    wire_api: String,
    requires_openai_auth: bool,
    supports_websockets: bool,
}

fn render_runtime_config(paths: &InstallPaths, bind: SocketAddr) -> Result<String, LifecycleError> {
    let capability = path_text(&paths.capability)?;
    let catalog = path_text(&paths.catalog)?;
    toml::to_string(&RuntimeFile {
        version: CONFIG_VERSION,
        server: RuntimeServerFile {
            bind: bind.to_string(),
            capability_token_file: capability,
        },
        grok: RuntimeGrokFile {
            catalog_cache_file: catalog,
            refresh_on_start: true,
        },
    })
    .map_err(LifecycleError::SerializeConfig)
}

fn render_profile(
    model: &str,
    bind: SocketAddr,
    capability: &str,
) -> Result<String, LifecycleError> {
    let base_url = format!("http://{bind}/_grok/{capability}/v1");
    toml::to_string(&ProfileFile {
        model,
        model_provider: "grok_bridge",
        model_providers: ProfileProviders {
            grok_bridge: ProfileProvider {
                name: PROFILE_PROVIDER_NAME,
                base_url: &base_url,
                wire_api: "responses",
                requires_openai_auth: false,
                supports_websockets: false,
            },
        },
    })
    .map_err(LifecycleError::SerializeProfile)
}

fn validate_runtime_config(manifest: &InstallManifest) -> Result<(), LifecycleError> {
    let bytes = read_private_control_file(&manifest.config_path)?;
    let source = std::str::from_utf8(&bytes).map_err(|_| LifecycleError::InvalidRuntimeConfig)?;
    let parsed: RuntimeFileRead =
        toml::from_str(source).map_err(|_| LifecycleError::InvalidRuntimeConfig)?;
    let bind: SocketAddr = manifest
        .bind
        .parse()
        .map_err(|_| LifecycleError::InvalidManifest)?;
    if parsed.version != CONFIG_VERSION
        || parsed.server.bind != bind
        || !parsed.server.bind.ip().is_loopback()
        || parsed.server.bind.port() == 0
        || parsed.server.capability_token_file != manifest.caller_token_path
        || parsed.grok.catalog_cache_file != manifest.catalog_cache_path
        || !parsed.grok.refresh_on_start
    {
        return Err(LifecycleError::InvalidRuntimeConfig);
    }
    Ok(())
}

fn validate_current_profile(
    manifest: &InstallManifest,
    capability: &str,
) -> Result<Vec<u8>, LifecycleError> {
    let bytes = read_private_control_file(&manifest.profile_path)?;
    let source = std::str::from_utf8(&bytes).map_err(|_| LifecycleError::InvalidProfile)?;
    let parsed: ProfileFileRead =
        toml::from_str(source).map_err(|_| LifecycleError::InvalidProfile)?;
    let bind: SocketAddr = manifest
        .bind
        .parse()
        .map_err(|_| LifecycleError::InvalidManifest)?;
    let expected_url = format!("http://{bind}/_grok/{capability}/v1");
    if parsed.model != manifest.initial_model
        || parsed.model_provider != "grok_bridge"
        || parsed.model_providers.grok_bridge.name != PROFILE_PROVIDER_NAME
        || parsed.model_providers.grok_bridge.base_url != expected_url
        || parsed.model_providers.grok_bridge.wire_api != "responses"
        || parsed.model_providers.grok_bridge.requires_openai_auth
        || parsed.model_providers.grok_bridge.supports_websockets
        || source != render_profile(&manifest.initial_model, bind, capability)?
    {
        return Err(LifecycleError::InvalidProfile);
    }
    Ok(bytes)
}

struct PreparedTarget {
    path: PathBuf,
    created: bool,
    backup: Option<BackupRecord>,
    original: Option<Vec<u8>>,
}

fn prepare_external_target(
    path: &Path,
    installed_mode: u32,
    allow_public_mode: bool,
) -> Result<PreparedTarget, LifecycleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(LifecycleError::UnsafeExternalTarget);
            }
            let mode = file_mode(&metadata)?;
            let permitted = mode == installed_mode || (allow_public_mode && mode == 0o600);
            if !permitted {
                return Err(LifecycleError::UnsafeExternalPermissions);
            }
            let original = read_regular_file(path, MAX_CONTROL_FILE_BYTES)?;
            let backup_path = unique_backup_path(path)?;
            write_new_file(&backup_path, &original, 0o600)?;
            Ok(PreparedTarget {
                path: path.to_path_buf(),
                created: false,
                backup: Some(BackupRecord {
                    path: backup_path,
                    original_mode: mode,
                }),
                original: Some(original),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PreparedTarget {
            path: path.to_path_buf(),
            created: true,
            backup: None,
            original: None,
        }),
        Err(error) => Err(LifecycleError::InspectExternalTarget(error)),
    }
}

fn cleanup_prepared_backup(plan: &PreparedTarget) {
    if let Some(backup) = &plan.backup {
        let _ = fs::remove_file(&backup.path);
    }
}

fn rollback_install(
    profile: &PreparedTarget,
    profile_mutated: bool,
    launch: &PreparedTarget,
    launch_mutated: bool,
    stage_root: &Path,
    install_root: &Path,
    root_moved: bool,
) -> Result<(), LifecycleError> {
    let mut failed = false;
    if launch_mutated && restore_prepared_target(launch).is_err() {
        failed = true;
    }
    if profile_mutated && restore_prepared_target(profile).is_err() {
        failed = true;
    }
    cleanup_prepared_backup(profile);
    cleanup_prepared_backup(launch);
    let tree = if root_moved { install_root } else { stage_root };
    if tree.exists() && fs::remove_dir_all(tree).is_err() {
        failed = true;
    }
    if failed {
        Err(LifecycleError::InstallRollbackFailed)
    } else {
        Ok(())
    }
}

fn restore_prepared_target(plan: &PreparedTarget) -> Result<(), LifecycleError> {
    if plan.created {
        match fs::remove_file(&plan.path) {
            Ok(()) => sync_parent(&plan.path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(LifecycleError::RestoreExternalTarget(error)),
        }
    } else {
        let backup = plan
            .backup
            .as_ref()
            .ok_or(LifecycleError::InvalidManifest)?;
        let original = plan
            .original
            .as_ref()
            .ok_or(LifecycleError::InvalidManifest)?;
        atomic_write(&plan.path, original, backup.original_mode)
    }
}

struct RestorePlan {
    path: PathBuf,
    installed: Vec<u8>,
    previous: Option<(Vec<u8>, u32)>,
    backup_path: Option<PathBuf>,
}

impl RestorePlan {
    fn new(
        path: &Path,
        backup: Option<&BackupRecord>,
        created: bool,
        installed: &[u8],
    ) -> Result<Self, LifecycleError> {
        if created == backup.is_some() {
            return Err(LifecycleError::InvalidManifest);
        }
        let previous = if let Some(backup) = backup {
            let bytes = read_private_control_file(&backup.path)?;
            Some((bytes, backup.original_mode))
        } else {
            None
        };
        Ok(Self {
            path: path.to_path_buf(),
            installed: installed.to_vec(),
            previous,
            backup_path: backup.map(|record| record.path.clone()),
        })
    }

    fn apply(&self) -> Result<(), LifecycleError> {
        if let Some((bytes, mode)) = &self.previous {
            atomic_write(&self.path, bytes, *mode)
        } else {
            fs::remove_file(&self.path).map_err(LifecycleError::RestoreExternalTarget)?;
            sync_parent(&self.path)
        }
    }

    fn rollback(&self) -> Result<(), LifecycleError> {
        let mode = if self.path.ends_with(PROFILE_FILE_NAME) {
            0o600
        } else {
            0o644
        };
        atomic_write(&self.path, &self.installed, mode)
    }

    fn finish(&self) -> Result<(), LifecycleError> {
        if let Some(path) = &self.backup_path {
            fs::remove_file(path).map_err(LifecycleError::RemoveBackup)?;
            sync_parent(path)?;
        }
        Ok(())
    }
}

struct ManagedInstall {
    profile_contents: Vec<u8>,
}

fn validate_managed_install(
    manifest: &InstallManifest,
    raw_manifest: &str,
) -> Result<ManagedInstall, LifecycleError> {
    validate_installed_binary(&manifest.binary_path)?;
    validate_launcher_bundle(&manifest.launcher_path)?;
    validate_runtime_config(manifest)?;
    validate_catalog_path(manifest)?;
    let token_bytes = read_private_control_file(&manifest.caller_token_path)?;
    let capability = validate_capability_bytes(&token_bytes)?;
    if raw_manifest.contains(capability) {
        return Err(LifecycleError::ManifestContainsCapability);
    }
    let profile_contents = validate_current_profile(manifest, capability)?;
    validate_current_launch_agent(manifest)?;
    validate_recorded_backups(manifest)?;
    validate_tree_has_no_symlinks(&manifest.install_root)?;
    Ok(ManagedInstall { profile_contents })
}

fn validate_manifest(
    manifest: &InstallManifest,
    request: &UninstallRequest,
) -> Result<(), LifecycleError> {
    if manifest.version != MANIFEST_VERSION
        || manifest.install_root != request.install_root
        || manifest.binary_path != request.install_root.join("bin").join(BINARY_FILE_NAME)
        || manifest.launcher_path != request.install_root.join(LAUNCHER_APP_PATH)
        || manifest.config_path != request.install_root.join("config").join("bridge.toml")
        || manifest.catalog_cache_path != request.install_root.join("state").join("models.json")
        || manifest.caller_token_path
            != request
                .install_root
                .join("secrets")
                .join("caller-capability")
        || manifest.logs_dir != request.install_root.join("logs")
        || manifest.profile_path != request.codex_home.join(PROFILE_FILE_NAME)
        || manifest.launch_agent_path != request.launch_agent_path
        || manifest.launch_agent_mode != 0o644
        || manifest.launch_agent_contents.is_empty()
        || manifest.launch_agent_contents.len() as u64 > MAX_CONTROL_FILE_BYTES
        || manifest.profile_created == manifest.profile_backup.is_some()
        || manifest.launch_agent_created == manifest.launch_agent_backup.is_some()
    {
        return Err(LifecycleError::InvalidManifest);
    }
    validate_safe_root(&manifest.install_root, &request.codex_home)?;
    validate_launch_label(&manifest.launch_agent_label)?;
    validate_model(&manifest.initial_model)?;
    let bind: SocketAddr = manifest
        .bind
        .parse()
        .map_err(|_| LifecycleError::InvalidManifest)?;
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Err(LifecycleError::InvalidManifest);
    }
    validate_backup_record(
        manifest.profile_backup.as_ref(),
        &manifest.profile_path,
        false,
    )?;
    validate_backup_record(
        manifest.launch_agent_backup.as_ref(),
        &manifest.launch_agent_path,
        true,
    )?;
    Ok(())
}

fn validate_backup_record(
    backup: Option<&BackupRecord>,
    target: &Path,
    launch_agent: bool,
) -> Result<(), LifecycleError> {
    let Some(backup) = backup else {
        return Ok(());
    };
    validate_absolute_clean(&backup.path)?;
    if backup.path.parent() != target.parent()
        || backup.path == target
        || !backup
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".grok-codex-bridge-backup-"))
        || if launch_agent {
            !matches!(backup.original_mode, 0o600 | 0o644)
        } else {
            backup.original_mode != 0o600
        }
    {
        return Err(LifecycleError::InvalidManifest);
    }
    Ok(())
}

fn read_manifest(root: &Path) -> Result<(InstallManifest, String), LifecycleError> {
    let bytes = read_private_control_file(&root.join(MANIFEST_FILE_NAME))?;
    let source = String::from_utf8(bytes).map_err(|_| LifecycleError::InvalidManifest)?;
    let manifest = serde_json::from_str(&source).map_err(LifecycleError::ParseManifest)?;
    Ok((manifest, source))
}

fn validate_recorded_backups(manifest: &InstallManifest) -> Result<(), LifecycleError> {
    for backup in [
        manifest.profile_backup.as_ref(),
        manifest.launch_agent_backup.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let _ = read_private_control_file(&backup.path)?;
    }
    Ok(())
}

fn validate_current_launch_agent(manifest: &InstallManifest) -> Result<(), LifecycleError> {
    let bytes = read_control_file_with_modes(
        &manifest.launch_agent_path,
        &[manifest.launch_agent_mode],
        MAX_CONTROL_FILE_BYTES,
    )?;
    if bytes != manifest.launch_agent_contents.as_bytes() {
        return Err(LifecycleError::InvalidLaunchAgent);
    }
    Ok(())
}

fn validate_catalog_path(manifest: &InstallManifest) -> Result<(), LifecycleError> {
    if manifest.catalog_cache_path != manifest.install_root.join("state").join("models.json") {
        return Err(LifecycleError::UnsafeCatalogPath);
    }
    validate_existing_directory(
        manifest
            .catalog_cache_path
            .parent()
            .ok_or(LifecycleError::UnsafeCatalogPath)?,
        || LifecycleError::UnsafeCatalogPath,
    )?;
    match fs::symlink_metadata(&manifest.catalog_cache_path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file()
                || metadata.file_type().is_symlink()
                || file_mode(&metadata)? != 0o600
            {
                return Err(LifecycleError::UnsafeCatalogPath);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(LifecycleError::UnsafeCatalogPath),
    }
    Ok(())
}

fn read_source_binary(path: &Path) -> Result<Vec<u8>, LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(LifecycleError::InspectSourceBinary)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LifecycleError::UnsafeSourceBinary);
    }
    if file_mode(&metadata)? & 0o111 == 0 || metadata.len() == 0 {
        return Err(LifecycleError::SourceBinaryNotExecutable);
    }
    read_regular_file(path, u64::MAX)
}

fn validate_installed_binary(path: &Path) -> Result<(), LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(LifecycleError::InspectInstalledBinary)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() == 0
        || file_mode(&metadata)? != 0o755
    {
        return Err(LifecycleError::UnsafeInstalledBinary);
    }
    Ok(())
}

fn validate_launcher_bundle(path: &Path) -> Result<(), LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(LifecycleError::InspectLauncherBundle)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_dir()
        || path.extension().and_then(|extension| extension.to_str()) != Some("app")
    {
        return Err(LifecycleError::UnsafeLauncherBundle);
    }
    validate_tree_has_no_symlinks(path)?;
    let executable = path.join(LAUNCHER_EXECUTABLE_PATH);
    validate_installed_binary(&executable).map_err(|error| match error {
        LifecycleError::InspectInstalledBinary(source) => {
            LifecycleError::InspectLauncherBundle(source)
        }
        LifecycleError::UnsafeInstalledBinary => LifecycleError::UnsafeLauncherBundle,
        LifecycleError::SourceBinaryNotExecutable => LifecycleError::UnsafeLauncherBundle,
        other => other,
    })?;
    let info = path.join("Contents/Info.plist");
    let info_bytes =
        read_regular_file(&info, MAX_CONTROL_FILE_BYTES).map_err(|error| match error {
            LifecycleError::OpenFile(source) | LifecycleError::InspectFile(source) => {
                LifecycleError::InspectLauncherBundle(source)
            }
            LifecycleError::UnsafeControlFile => LifecycleError::UnsafeLauncherBundle,
            other => other,
        })?;
    let info =
        std::str::from_utf8(&info_bytes).map_err(|_| LifecycleError::UnsafeLauncherBundle)?;
    if !info.contains("<key>CFBundlePackageType</key>")
        || !info.contains("<string>APPL</string>")
        || !info.contains("<key>CFBundleExecutable</key>")
        || !info.contains("<string>Grok Codex Switch</string>")
    {
        return Err(LifecycleError::UnsafeLauncherBundle);
    }
    let overlay = path.join(LAUNCHER_OVERLAY_PATH);
    let overlay_bytes =
        read_regular_file(&overlay, MAX_CONTROL_FILE_BYTES).map_err(|error| match error {
            LifecycleError::OpenFile(source) | LifecycleError::InspectFile(source) => {
                LifecycleError::InspectLauncherBundle(source)
            }
            LifecycleError::UnsafeControlFile => LifecycleError::UnsafeLauncherBundle,
            other => other,
        })?;
    if overlay_bytes.is_empty() || std::str::from_utf8(&overlay_bytes).is_err() {
        return Err(LifecycleError::UnsafeLauncherBundle);
    }
    Ok(())
}

fn copy_launcher_bundle(source: &Path, destination: &Path) -> Result<(), LifecycleError> {
    validate_launcher_bundle(source)?;
    copy_launcher_tree(source, destination)
}

fn copy_launcher_tree(source: &Path, destination: &Path) -> Result<(), LifecycleError> {
    let source_metadata =
        fs::symlink_metadata(source).map_err(LifecycleError::InspectLauncherBundle)?;
    if source_metadata.file_type().is_symlink() {
        return Err(LifecycleError::UnsafeLauncherBundle);
    }
    if source_metadata.is_dir() {
        fs::create_dir(destination).map_err(LifecycleError::CreateStage)?;
        set_mode(destination, file_mode(&source_metadata)?)?;
        for entry in fs::read_dir(source).map_err(LifecycleError::InspectLauncherBundle)? {
            let entry = entry.map_err(LifecycleError::InspectLauncherBundle)?;
            copy_launcher_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if !source_metadata.is_file() {
        return Err(LifecycleError::UnsafeLauncherBundle);
    }
    fs::copy(source, destination).map_err(LifecycleError::WriteFile)?;
    set_mode(destination, file_mode(&source_metadata)?)
}

fn generate_capability() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn validate_capability_bytes(bytes: &[u8]) -> Result<&str, LifecycleError> {
    let capability = std::str::from_utf8(bytes).map_err(|_| LifecycleError::InvalidCapability)?;
    if capability.len() != CAPABILITY_LENGTH
        || !capability
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(LifecycleError::InvalidCapability);
    }
    Ok(capability)
}

fn validate_model(model: &str) -> Result<(), LifecycleError> {
    if model.is_empty()
        || model.len() > 256
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(LifecycleError::InvalidInitialModel);
    }
    Ok(())
}

fn validate_launch_label(label: &str) -> Result<(), LifecycleError> {
    if label.is_empty()
        || label.len() > 255
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(LifecycleError::InvalidLaunchAgentLabel);
    }
    Ok(())
}

fn create_stage_tree(root: &Path) -> Result<(), LifecycleError> {
    fs::create_dir(root).map_err(LifecycleError::CreateStage)?;
    set_mode(root, 0o700)?;
    for (name, mode) in [
        ("bin", 0o755),
        ("config", 0o700),
        ("state", 0o700),
        ("secrets", 0o700),
        ("logs", 0o700),
    ] {
        let path = root.join(name);
        fs::create_dir(&path).map_err(LifecycleError::CreateStage)?;
        set_mode(&path, mode)?;
    }
    Ok(())
}

fn unique_sibling(path: &Path, kind: &str) -> Result<PathBuf, LifecycleError> {
    let parent = path.parent().ok_or(LifecycleError::UnsafePath)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LifecycleError::UnsafePath)?;
    Ok(parent.join(format!(".{name}.{kind}.{}", Uuid::new_v4().simple())))
}

fn unique_backup_path(target: &Path) -> Result<PathBuf, LifecycleError> {
    let parent = target.parent().ok_or(LifecycleError::UnsafePath)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LifecycleError::UnsafePath)?;
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.fZ");
    Ok(parent.join(format!(
        "{name}.grok-codex-bridge-backup-{timestamp}-{}",
        Uuid::new_v4().simple()
    )))
}

fn validate_absolute_clean(path: &Path) -> Result<(), LifecycleError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(LifecycleError::UnsafePath);
    }
    Ok(())
}

fn validate_safe_root(root: &Path, codex_home: &Path) -> Result<(), LifecycleError> {
    let inferred_home = codex_home
        .parent()
        .ok_or(LifecycleError::UnsafeInstallRoot)?;
    if root == Path::new("/")
        || root == inferred_home
        || root == codex_home
        || codex_home.starts_with(root)
        || root.starts_with(codex_home)
    {
        return Err(LifecycleError::UnsafeInstallRoot);
    }
    Ok(())
}

fn validate_existing_parent(path: &Path) -> Result<(), LifecycleError> {
    let parent = path.parent().ok_or(LifecycleError::UnsafePath)?;
    validate_existing_directory(parent, || LifecycleError::UnsafeParent)
}

fn validate_existing_directory<F>(path: &Path, error: F) -> Result<(), LifecycleError>
where
    F: Fn() -> LifecycleError,
{
    let metadata = fs::symlink_metadata(path).map_err(|_| error())?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(error());
    }
    Ok(())
}

fn validate_tree_has_no_symlinks(root: &Path) -> Result<(), LifecycleError> {
    let metadata = fs::symlink_metadata(root).map_err(LifecycleError::InspectInstallTree)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(LifecycleError::UnsafeInstallRoot);
    }
    for entry in fs::read_dir(root).map_err(LifecycleError::InspectInstallTree)? {
        let entry = entry.map_err(LifecycleError::InspectInstallTree)?;
        let metadata = entry
            .file_type()
            .map_err(LifecycleError::InspectInstallTree)?;
        if metadata.is_symlink() {
            return Err(LifecycleError::UnsafeInstallTree);
        }
        if metadata.is_dir() {
            validate_tree_has_no_symlinks(&entry.path())?;
        } else if !metadata.is_file() {
            return Err(LifecycleError::UnsafeInstallTree);
        }
    }
    Ok(())
}

fn remove_tree_without_symlinks(root: &Path) -> Result<(), LifecycleError> {
    validate_tree_has_no_symlinks(root)?;
    fs::remove_dir_all(root).map_err(LifecycleError::RemoveInstallRoot)?;
    sync_parent(root)
}

fn path_text(path: &Path) -> Result<&str, LifecycleError> {
    path.to_str().ok_or(LifecycleError::NonUtf8Path)
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> Result<u32, LifecycleError> {
    use std::os::unix::fs::PermissionsExt;
    Ok(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> Result<u32, LifecycleError> {
    Err(LifecycleError::UnsupportedPlatform)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), LifecycleError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(LifecycleError::SetPermissions)
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), LifecycleError> {
    Err(LifecycleError::UnsupportedPlatform)
}

#[cfg(unix)]
fn open_read_only(path: &Path) -> Result<fs::File, LifecycleError> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(LifecycleError::OpenFile)
}

#[cfg(not(unix))]
fn open_read_only(_path: &Path) -> Result<fs::File, LifecycleError> {
    Err(LifecycleError::UnsupportedPlatform)
}

fn read_regular_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, LifecycleError> {
    let mut file = open_read_only(path)?;
    let metadata = file.metadata().map_err(LifecycleError::InspectFile)?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(LifecycleError::UnsafeControlFile);
    }
    let capacity = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(LifecycleError::ReadFile)?;
    Ok(bytes)
}

fn read_private_control_file(path: &Path) -> Result<Vec<u8>, LifecycleError> {
    read_control_file_with_modes(path, &[0o600], MAX_CONTROL_FILE_BYTES)
}

fn read_control_file_with_modes(
    path: &Path,
    modes: &[u32],
    max_bytes: u64,
) -> Result<Vec<u8>, LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(LifecycleError::InspectFile)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || !modes.contains(&file_mode(&metadata)?)
    {
        return Err(LifecycleError::UnsafeControlFile);
    }
    read_regular_file(path, max_bytes)
}

#[cfg(unix)]
fn write_new_file(path: &Path, contents: &[u8], mode: u32) -> Result<(), LifecycleError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(LifecycleError::CreateFile)?;
    set_mode(path, mode)?;
    file.write_all(contents)
        .map_err(LifecycleError::WriteFile)?;
    file.sync_all().map_err(LifecycleError::SyncFile)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_new_file(_path: &Path, _contents: &[u8], _mode: u32) -> Result<(), LifecycleError> {
    Err(LifecycleError::UnsupportedPlatform)
}

fn atomic_write(path: &Path, contents: &[u8], mode: u32) -> Result<(), LifecycleError> {
    let temporary = unique_sibling(path, "write")?;
    write_new_file(&temporary, contents, mode)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(LifecycleError::AtomicRename(error));
    }
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), LifecycleError> {
    let parent = path.parent().ok_or(LifecycleError::UnsafePath)?;
    let directory = fs::File::open(parent).map_err(LifecycleError::OpenParent)?;
    directory.sync_all().map_err(LifecycleError::SyncParent)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallFailpoint {
    None,
    AfterProfile,
}

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("a lifecycle path must be absolute, normalized, and narrowly scoped")]
    UnsafePath,
    #[error("the install root is too broad or overlaps protected state")]
    UnsafeInstallRoot,
    #[error("the install root already exists; reinstall and overwrite are refused")]
    InstallRootAlreadyExists,
    #[error("the install parent is missing, is a symlink, or is not a directory")]
    UnsafeParent,
    #[error("the Codex home is missing, is a symlink, or is not a directory")]
    UnsafeCodexHome,
    #[error("managed lifecycle paths overlap")]
    OverlappingManagedPaths,
    #[error("the server bind address must be loopback")]
    NonLoopbackBind,
    #[error("the server bind port must be non-zero")]
    ZeroPort,
    #[error("the initial model identifier is invalid")]
    InvalidInitialModel,
    #[error("the LaunchAgent label is invalid")]
    InvalidLaunchAgentLabel,
    #[error("the LaunchAgent contents are empty or too large")]
    InvalidLaunchAgentContents,
    #[error("LaunchAgent contents must not contain caller capability material")]
    LaunchAgentContainsCapability,
    #[error("the source binary is missing or cannot be inspected")]
    InspectSourceBinary(#[source] std::io::Error),
    #[error("the source binary must be a regular non-symlink file")]
    UnsafeSourceBinary,
    #[error("the source binary must be non-empty and executable")]
    SourceBinaryNotExecutable,
    #[error("the launcher bundle cannot be inspected")]
    InspectLauncherBundle(#[source] std::io::Error),
    #[error(
        "the launcher bundle must be a regular non-symlink .app with a valid executable and Info.plist"
    )]
    UnsafeLauncherBundle,
    #[error("the installed binary cannot be inspected")]
    InspectInstalledBinary(#[source] std::io::Error),
    #[error("the installed binary must be a non-empty regular 0755 executable")]
    UnsafeInstalledBinary,
    #[error("an external target could not be inspected")]
    InspectExternalTarget(#[source] std::io::Error),
    #[error("an external target must be a regular non-symlink file")]
    UnsafeExternalTarget,
    #[error("an external target has unsafe permissions")]
    UnsafeExternalPermissions,
    #[error("failed to create the staged install tree")]
    CreateStage(#[source] std::io::Error),
    #[error("failed to create a lifecycle file without overwriting")]
    CreateFile(#[source] std::io::Error),
    #[error("failed to write a lifecycle file")]
    WriteFile(#[source] std::io::Error),
    #[error("failed to synchronize a lifecycle file")]
    SyncFile(#[source] std::io::Error),
    #[error("failed to set lifecycle file permissions")]
    SetPermissions(#[source] std::io::Error),
    #[error("failed to atomically replace a lifecycle file")]
    AtomicRename(#[source] std::io::Error),
    #[error("failed to materialize the install root atomically")]
    InstallRootRename(#[source] std::io::Error),
    #[error("failed to restore an external lifecycle target")]
    RestoreExternalTarget(#[source] std::io::Error),
    #[error("install rollback could not restore the exact pre-install state")]
    InstallRollbackFailed,
    #[error("uninstall rollback could not restore the installed state")]
    UninstallRollbackFailed,
    #[error("failed to remove an external backup")]
    RemoveBackup(#[source] std::io::Error),
    #[error("failed to remove the manifest-proven install root")]
    RemoveInstallRoot(#[source] std::io::Error),
    #[error("failed to inspect the install tree")]
    InspectInstallTree(#[source] std::io::Error),
    #[error("the install tree contains a symlink or special file")]
    UnsafeInstallTree,
    #[error("failed to open a lifecycle file")]
    OpenFile(#[source] std::io::Error),
    #[error("failed to inspect a lifecycle file")]
    InspectFile(#[source] std::io::Error),
    #[error("a lifecycle control file is not a safe regular file")]
    UnsafeControlFile,
    #[error("failed to read a lifecycle file")]
    ReadFile(#[source] std::io::Error),
    #[error("failed to open a lifecycle parent directory")]
    OpenParent(#[source] std::io::Error),
    #[error("failed to synchronize a lifecycle parent directory")]
    SyncParent(#[source] std::io::Error),
    #[error("a lifecycle path is not valid UTF-8")]
    NonUtf8Path,
    #[error("failed to serialize bridge runtime configuration")]
    SerializeConfig(#[source] toml::ser::Error),
    #[error("failed to serialize the isolated Codex profile")]
    SerializeProfile(#[source] toml::ser::Error),
    #[error("failed to serialize install manifest")]
    SerializeManifest(#[source] serde_json::Error),
    #[error("failed to parse install manifest")]
    ParseManifest(#[source] serde_json::Error),
    #[error("install manifest content or path ownership is invalid")]
    InvalidManifest,
    #[error("install manifest contains caller capability material")]
    ManifestContainsCapability,
    #[error("bridge runtime configuration is invalid")]
    InvalidRuntimeConfig,
    #[error("the caller capability is invalid")]
    InvalidCapability,
    #[error("the isolated Codex profile is invalid or altered")]
    InvalidProfile,
    #[error("the LaunchAgent is invalid or altered")]
    InvalidLaunchAgent,
    #[error("the model catalog cache path is unsafe")]
    UnsafeCatalogPath,
    #[error("the admitted Grok catalog is unavailable for picker generation")]
    PickerCatalogUnavailable,
    #[error("the Codex base configuration is invalid for managed picker state")]
    InvalidPickerConfig,
    #[error(
        "Native compatibility requires the root model to exist in the authoritative Native catalog"
    )]
    NativeCompatibilityFallbackUnavailable,
    #[error("the Codex base configuration already owns conflicting picker settings")]
    PickerConfigConflict,
    #[error("picker managed state or its recorded artifacts are invalid or altered")]
    InvalidPickerState,
    #[error(transparent)]
    Picker(#[from] crate::picker::PickerError),
    #[error(transparent)]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error(transparent)]
    Native(#[from] crate::native::NativeError),
    #[error("test-only injected install failure")]
    InjectedInstallFailure,
    #[cfg(not(unix))]
    #[error("lifecycle file permissions are unsupported on this platform")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::catalog::CatalogSnapshot;

    struct Fixture {
        _temporary: tempfile::TempDir,
        home: PathBuf,
        codex_home: PathBuf,
        install_root: PathBuf,
        source_binary: PathBuf,
        source_launcher: PathBuf,
        launch_agent: PathBuf,
        credential: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let home = temporary.path().join("home");
            let codex_home = home.join(".codex");
            let install_parent = home.join("Library/Application Support");
            let launch_parent = home.join("Library/LaunchAgents");
            fs::create_dir_all(&codex_home).unwrap();
            fs::create_dir_all(&install_parent).unwrap();
            fs::create_dir_all(&launch_parent).unwrap();
            let source_binary = temporary.path().join("source-binary");
            fs::write(&source_binary, b"fake native executable").unwrap();
            fs::set_permissions(&source_binary, fs::Permissions::from_mode(0o755)).unwrap();
            let source_launcher = temporary.path().join("Grok Codex Switch.app");
            let launcher_executable = source_launcher.join("Contents/MacOS/Grok Codex Switch");
            fs::create_dir_all(launcher_executable.parent().unwrap()).unwrap();
            fs::create_dir_all(source_launcher.join("Contents/Resources")).unwrap();
            fs::write(
                source_launcher.join("Contents/Info.plist"),
                br#"<plist><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleExecutable</key><string>Grok Codex Switch</string></plist>"#,
            )
            .unwrap();
            fs::write(
                source_launcher.join("Contents/Resources/grok-codex-bridge-overlay.md"),
                b"Grok overlay",
            )
            .unwrap();
            fs::write(&launcher_executable, b"fake launcher executable").unwrap();
            fs::set_permissions(&launcher_executable, fs::Permissions::from_mode(0o755)).unwrap();
            let credential = temporary.path().join("auth.json");
            fs::write(
                &credential,
                br#"{"https://auth.x.ai::test-client":{"key":"test-secret","auth_mode":"oidc","create_time":"2026-08-01T00:00:00Z","user_id":"test-user","expires_at":"2099-01-01T00:00:00Z","oidc_issuer":"https://auth.x.ai"}}"#,
            )
            .unwrap();
            fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
            Self {
                install_root: install_parent.join("grok-codex-bridge"),
                launch_agent: launch_parent.join("com.example.grok-codex-bridge.plist"),
                _temporary: temporary,
                home,
                codex_home,
                source_binary,
                source_launcher,
                credential,
            }
        }

        fn install_request(&self) -> InstallRequest {
            InstallRequest {
                source_binary: self.source_binary.clone(),
                source_launcher: self.source_launcher.clone(),
                install_root: self.install_root.clone(),
                codex_home: self.codex_home.clone(),
                launch_agent_path: self.launch_agent.clone(),
                launch_agent_label: "com.example.grok-codex-bridge".to_owned(),
                launch_agent_contents: format!(
                    "<plist><label>com.example.grok-codex-bridge</label><binary>{}</binary></plist>",
                    self.install_root.join("bin/grok-codex-bridge").display()
                ),
                bind: "127.0.0.1:4545".parse().unwrap(),
                initial_model: "grok-4.6".to_owned(),
            }
        }

        fn uninstall_request(&self) -> UninstallRequest {
            UninstallRequest {
                install_root: self.install_root.clone(),
                codex_home: self.codex_home.clone(),
                launch_agent_path: self.launch_agent.clone(),
            }
        }

        fn doctor_request(&self) -> DoctorRequest {
            DoctorRequest {
                install_root: self.install_root.clone(),
                codex_home: self.codex_home.clone(),
                launch_agent_path: self.launch_agent.clone(),
                credential_path: self.credential.clone(),
            }
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn manifest(fixture: &Fixture) -> (InstallManifest, String) {
        read_manifest(&fixture.install_root).unwrap()
    }

    fn install_test_picker(
        fixture: &Fixture,
        original_config: &[u8],
    ) -> (PickerInstallReceipt, PathBuf) {
        install(&fixture.install_request()).unwrap();
        let catalog = CatalogSnapshot::new(["grok-4.6"], Some("\"v1\"".to_owned())).unwrap();
        CatalogCache::new(fixture.install_root.join("state/models.json"))
            .persist(&catalog)
            .unwrap();
        let native_catalog = fixture.home.join("native-models.json");
        fs::write(
            &native_catalog,
            br#"{"models":[{"slug":"gpt-native","display_name":"Native GPT","supported_reasoning_levels":[{"effort":"medium","description":"Balanced"}],"shell_type":"default","visibility":"list","supported_in_api":true,"priority":1,"base_instructions":"native","support_verbosity":true,"truncation_policy":{"mode":"tokens","limit":1000},"experimental_supported_tools":[]}]}"#,
        )
        .unwrap();
        fs::set_permissions(&native_catalog, fs::Permissions::from_mode(0o600)).unwrap();
        let config = fixture.codex_home.join("config.toml");
        fs::write(&config, original_config).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let receipt = install_picker(&PickerInstallRequest {
            install_root: fixture.install_root.clone(),
            codex_home: fixture.codex_home.clone(),
            native_catalog_path: native_catalog.clone(),
            grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
            native_upstream: NativeUpstream::ChatgptCodex,
            bind: "127.0.0.1:4545".parse().unwrap(),
            native_compatibility: false,
        })
        .unwrap();
        (receipt, native_catalog)
    }

    #[test]
    fn fresh_install_materializes_exact_private_safe_provider_layout() {
        let fixture = Fixture::new();
        let receipt = install(&fixture.install_request()).unwrap();
        assert_eq!(
            receipt.binary_path,
            fixture.install_root.join("bin/grok-codex-bridge")
        );
        assert_eq!(
            fs::read(&receipt.binary_path).unwrap(),
            b"fake native executable"
        );
        assert_eq!(mode(&receipt.binary_path), 0o755);
        assert_eq!(
            receipt.launcher_path,
            fixture.install_root.join("bin/Grok Codex Switch.app")
        );
        assert_eq!(
            fs::read(
                fixture
                    .install_root
                    .join("bin/Grok Codex Switch.app/Contents/MacOS/Grok Codex Switch")
            )
            .unwrap(),
            b"fake launcher executable"
        );
        assert_eq!(
            mode(
                &fixture
                    .install_root
                    .join("bin/Grok Codex Switch.app/Contents/MacOS/Grok Codex Switch")
            ),
            0o755
        );
        assert_eq!(
            fs::read(
                fixture.install_root.join(
                    "bin/Grok Codex Switch.app/Contents/Resources/grok-codex-bridge-overlay.md"
                )
            )
            .unwrap(),
            b"Grok overlay"
        );
        let (installed_manifest, _) = manifest(&fixture);
        assert_eq!(installed_manifest.launcher_path, receipt.launcher_path);
        assert_eq!(
            CatalogCache::new(fixture.install_root.join("state/models.json"))
                .load()
                .unwrap()
                .unwrap()
                .model_ids(),
            ["grok-4.6", "grok-4.5"]
        );
        assert_eq!(mode(&receipt.config_path), 0o600);
        assert_eq!(mode(&receipt.profile_path), 0o600);
        assert_eq!(mode(&receipt.manifest_path), 0o600);
        assert_eq!(
            mode(&fixture.install_root.join("secrets/caller-capability")),
            0o600
        );

        let capability =
            fs::read_to_string(fixture.install_root.join("secrets/caller-capability")).unwrap();
        assert_eq!(capability.len(), CAPABILITY_LENGTH);
        let profile = fs::read_to_string(&receipt.profile_path).unwrap();
        assert!(profile.contains("model = \"grok-4.6\""));
        assert!(profile.contains("model_provider = \"grok_bridge\""));
        assert!(profile.contains("name = \"Grok Codex Bridge\""));
        assert!(profile.contains(&format!(
            "base_url = \"http://127.0.0.1:4545/_grok/{capability}/v1\""
        )));
        assert!(profile.contains("wire_api = \"responses\""));
        assert!(profile.contains("requires_openai_auth = false"));
        assert!(profile.contains("supports_websockets = false"));
        assert!(!profile.contains("[profiles"));
        assert!(!fixture.codex_home.join("config.toml").exists());

        let (installed_manifest, raw_manifest) = manifest(&fixture);
        assert_eq!(
            validate_current_profile(&installed_manifest, &capability).unwrap(),
            profile.as_bytes()
        );
        assert!(!raw_manifest.contains(&capability));
        assert!(!raw_manifest.contains("test-secret"));
        assert!(!raw_manifest.contains("test-user"));
        assert!(fixture.install_root.join("config").is_dir());
        assert!(fixture.install_root.join("state").is_dir());
        assert!(fixture.install_root.join("secrets").is_dir());
        assert!(fixture.install_root.join("logs").is_dir());
    }

    #[test]
    fn preexisting_profile_and_launch_agent_are_backed_up_and_restored_exactly() {
        let fixture = Fixture::new();
        let profile = fixture.codex_home.join(PROFILE_FILE_NAME);
        let original_profile = b"# exact prior profile\nmodel = \"prior\"\n";
        let original_launch = b"<plist>prior launch agent</plist>\n";
        fs::write(&profile, original_profile).unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&fixture.launch_agent, original_launch).unwrap();
        fs::set_permissions(&fixture.launch_agent, fs::Permissions::from_mode(0o644)).unwrap();

        let receipt = install(&fixture.install_request()).unwrap();
        assert!(receipt.profile_replaced);
        assert!(receipt.launch_agent_replaced);
        let (manifest, _) = manifest(&fixture);
        let profile_backup = manifest.profile_backup.as_ref().unwrap();
        let launch_backup = manifest.launch_agent_backup.as_ref().unwrap();
        assert_eq!(fs::read(&profile_backup.path).unwrap(), original_profile);
        assert_eq!(fs::read(&launch_backup.path).unwrap(), original_launch);
        assert_eq!(mode(&profile_backup.path), 0o600);
        assert_eq!(mode(&launch_backup.path), 0o600);

        let result = uninstall(&fixture.uninstall_request()).unwrap();
        assert!(result.profile_restored);
        assert!(result.launch_agent_restored);
        assert_eq!(fs::read(&profile).unwrap(), original_profile);
        assert_eq!(fs::read(&fixture.launch_agent).unwrap(), original_launch);
        assert_eq!(mode(&profile), 0o600);
        assert_eq!(mode(&fixture.launch_agent), 0o644);
        assert!(!profile_backup.path.exists());
        assert!(!launch_backup.path.exists());
        assert!(!fixture.install_root.exists());
    }

    #[test]
    fn uninstall_removes_only_bridge_created_profile_launch_agent_and_root() {
        let fixture = Fixture::new();
        install(&fixture.install_request()).unwrap();
        let unrelated = fixture.codex_home.join("config.toml");
        fs::write(&unrelated, "native = true\n").unwrap();
        uninstall(&fixture.uninstall_request()).unwrap();
        assert!(!fixture.codex_home.join(PROFILE_FILE_NAME).exists());
        assert!(!fixture.launch_agent.exists());
        assert!(!fixture.install_root.exists());
        assert_eq!(fs::read_to_string(unrelated).unwrap(), "native = true\n");
    }

    #[test]
    fn picker_lifecycle_generates_provider_catalog_preserves_config_and_rolls_back_exactly() {
        let fixture = Fixture::new();
        install(&fixture.install_request()).unwrap();
        let catalog = CatalogSnapshot::new(["grok-4.6"], Some("\"v1\"".to_owned())).unwrap();
        CatalogCache::new(fixture.install_root.join("state/models.json"))
            .persist(&catalog)
            .unwrap();
        let native_catalog = fixture.home.join("native-models.json");
        fs::write(
            &native_catalog,
            br#"{"models":[{"slug":"gpt-native","display_name":"Native GPT","supported_reasoning_levels":[{"effort":"medium","description":"Balanced"}],"shell_type":"default","visibility":"list","supported_in_api":true,"priority":1,"base_instructions":"native","support_verbosity":true,"truncation_policy":{"mode":"tokens","limit":1000},"experimental_supported_tools":[]}]}"#,
        )
        .unwrap();
        fs::set_permissions(&native_catalog, fs::Permissions::from_mode(0o600)).unwrap();
        let config = fixture.codex_home.join("config.toml");
        let original =
            b"# keep this comment\nmodel = \"gpt-native\"\n\n[features]\nfuture = true\n";
        fs::write(&config, original).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();

        let receipt = install_picker(&PickerInstallRequest {
            install_root: fixture.install_root.clone(),
            codex_home: fixture.codex_home.clone(),
            native_catalog_path: native_catalog,
            grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
            native_upstream: NativeUpstream::ChatgptCodex,
            bind: "127.0.0.1:4545".parse().unwrap(),
            native_compatibility: false,
        })
        .unwrap();
        let generated: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt.generated_catalog_path).unwrap()).unwrap();
        assert!(generated["models"][0].get("model_provider").is_none());
        assert!(generated["models"][1].get("model_provider").is_none());
        let native_route =
            NativeRouteState::from_json(&fs::read(&receipt.native_route_path).unwrap()).unwrap();
        assert!(native_route.contains("gpt-native"));
        assert!(!native_route.contains("grok-4.6"));
        assert_eq!(native_route.upstream(), NativeUpstream::ChatgptCodex);
        let managed = fs::read_to_string(&config).unwrap();
        assert!(managed.starts_with("# keep this comment\nmodel = \"gpt-native\"\n\n"));
        assert!(managed.contains(PICKER_BEGIN));
        let parsed: toml::Value = toml::from_str(&managed).unwrap();
        assert_eq!(parsed["model_provider"].as_str(), Some(PICKER_PROVIDER_ID));
        assert_eq!(
            parsed["model_catalog_json"].as_str(),
            receipt.generated_catalog_path.to_str()
        );
        assert!(parsed.get("openai_base_url").is_none());
        let provider = &parsed["model_providers"][PICKER_PROVIDER_ID];
        assert_eq!(provider["name"].as_str(), Some(PICKER_PROVIDER_NAME));
        assert_eq!(
            provider["base_url"].as_str(),
            Some("http://127.0.0.1:4545/v1")
        );
        let capability = fs::read_to_string(
            fixture
                .install_root
                .join("secrets")
                .join("caller-capability"),
        )
        .unwrap();
        assert_eq!(
            provider["http_headers"][PICKER_CALLER_HEADER].as_str(),
            Some(capability.as_str())
        );
        assert!(
            !provider["base_url"]
                .as_str()
                .unwrap()
                .contains(capability.as_str())
        );
        assert_eq!(provider["wire_api"].as_str(), Some("responses"));
        assert_eq!(provider["requires_openai_auth"].as_bool(), Some(true));
        assert_eq!(provider["supports_websockets"].as_bool(), Some(false));
        assert_eq!(provider["request_max_retries"].as_integer(), Some(0));
        assert_eq!(provider["stream_max_retries"].as_integer(), Some(0));
        assert!(managed.ends_with("[features]\nfuture = true\n"));
        assert_eq!(mode(&config), 0o600);

        assert!(uninstall_picker(&fixture.install_root, &fixture.codex_home).unwrap());
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!receipt.generated_catalog_path.exists());
        assert!(!receipt.native_route_path.exists());
        assert!(!receipt.managed_state_path.exists());
    }

    #[test]
    fn native_compatibility_uses_existing_root_model_without_changing_config() {
        let fixture = Fixture::new();
        let original =
            b"# keep this comment\nmodel = \"gpt-native\"\n\n[features]\nfuture = true\n";
        let (_grok_receipt, native_catalog) = install_test_picker(&fixture, original);
        let config = fixture.codex_home.join("config.toml");
        let before: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let receipt = install_picker(&PickerInstallRequest {
            install_root: fixture.install_root.clone(),
            codex_home: fixture.codex_home.clone(),
            native_catalog_path: native_catalog,
            grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
            native_upstream: NativeUpstream::ChatgptCodex,
            bind: "127.0.0.1:4545".parse().unwrap(),
            native_compatibility: true,
        })
        .unwrap();
        let route =
            NativeRouteState::from_json(&fs::read(receipt.native_route_path).unwrap()).unwrap();
        assert_eq!(route.native_compatibility_fallback(), Some("gpt-native"));
        assert_eq!(receipt.grok_model_count, 1);
        let catalog: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt.generated_catalog_path).unwrap()).unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 2);
        assert_eq!(catalog["models"][0]["slug"], "gpt-native");
        assert_eq!(catalog["models"][1]["slug"], "grok-4.6");
        assert_eq!(catalog["models"][1]["visibility"], "hide");
        assert_eq!(
            catalog["models"][1]["base_instructions"],
            catalog["models"][0]["base_instructions"]
        );
        let after: toml::Value = toml::from_str(&fs::read_to_string(config).unwrap()).unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn native_compatibility_rejects_root_model_missing_from_native_catalog() {
        let fixture = Fixture::new();
        let original = b"model = \"gpt-missing\"\n";
        let (_grok_receipt, native_catalog) = install_test_picker(&fixture, original);
        let result = install_picker(&PickerInstallRequest {
            install_root: fixture.install_root.clone(),
            codex_home: fixture.codex_home.clone(),
            native_catalog_path: native_catalog,
            grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
            native_upstream: NativeUpstream::ChatgptCodex,
            bind: "127.0.0.1:4545".parse().unwrap(),
            native_compatibility: true,
        });
        assert!(matches!(
            result,
            Err(LifecycleError::NativeCompatibilityFallbackUnavailable)
        ));
    }

    #[test]
    fn picker_update_and_uninstall_recover_desktop_rewrite_and_native_catalog_refresh() {
        let fixture = Fixture::new();
        let original = b"model = \"gpt-native\"\n\n[features]\nfuture = true\n";
        let (receipt, native_catalog) = install_test_picker(&fixture, original);
        let config = fixture.codex_home.join("config.toml");

        let desktop_value: toml::Value =
            toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let desktop_rewrite = format!(
            "{}\n[marketplace]\nlast_refresh = \"desktop-current\"\n",
            toml::to_string(&desktop_value).unwrap()
        );
        assert!(!desktop_rewrite.contains(PICKER_BEGIN));
        fs::write(&config, desktop_rewrite).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();

        let native_before = fs::read_to_string(&native_catalog).unwrap();
        let native_refreshed = native_before.replace("Native GPT", "Native GTP");
        assert_eq!(native_before.len(), native_refreshed.len());
        fs::write(&native_catalog, &native_refreshed).unwrap();
        fs::set_permissions(&native_catalog, fs::Permissions::from_mode(0o600)).unwrap();

        let updated = install_picker(&PickerInstallRequest {
            install_root: fixture.install_root.clone(),
            codex_home: fixture.codex_home.clone(),
            native_catalog_path: native_catalog.clone(),
            grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
            native_upstream: NativeUpstream::ChatgptCodex,
            bind: "127.0.0.1:4545".parse().unwrap(),
            native_compatibility: false,
        })
        .unwrap();
        assert_eq!(updated.config_path, receipt.config_path);

        assert!(uninstall_picker(&fixture.install_root, &fixture.codex_home).unwrap());
        let recovered: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(recovered["model"].as_str(), Some("gpt-native"));
        assert_eq!(recovered["features"]["future"].as_bool(), Some(true));
        assert_eq!(
            recovered["marketplace"]["last_refresh"].as_str(),
            Some("desktop-current")
        );
        assert!(recovered.get("model_provider").is_none());
        assert!(recovered.get("model_catalog_json").is_none());
        assert!(
            recovered
                .get("model_providers")
                .and_then(toml::Value::as_table)
                .is_none_or(|providers| !providers.contains_key(PICKER_PROVIDER_ID))
        );
        assert_eq!(
            fs::read_to_string(&native_catalog).unwrap(),
            native_refreshed
        );
        assert_eq!(mode(&config), 0o644);
        assert!(!updated.generated_catalog_path.exists());
        assert!(!updated.native_route_path.exists());
        assert!(!updated.managed_state_path.exists());
    }

    #[test]
    fn picker_uninstall_directly_recovers_desktop_rewrite_and_native_catalog_refresh() {
        let fixture = Fixture::new();
        let (receipt, native_catalog) = install_test_picker(&fixture, b"model = \"gpt-native\"\n");
        let config = fixture.codex_home.join("config.toml");
        let desktop_value: toml::Value =
            toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let desktop_rewrite = format!(
            "{}\n[app]\nlast_opened = \"desktop-current\"\n",
            toml::to_string(&desktop_value).unwrap()
        );
        assert!(!desktop_rewrite.contains(PICKER_BEGIN));
        fs::write(&config, desktop_rewrite).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();

        let native_before = fs::read_to_string(&native_catalog).unwrap();
        let native_refreshed = native_before.replace("Native GPT", "Native GTP");
        assert_eq!(native_before.len(), native_refreshed.len());
        fs::write(&native_catalog, &native_refreshed).unwrap();
        fs::set_permissions(&native_catalog, fs::Permissions::from_mode(0o600)).unwrap();

        assert!(uninstall_picker(&fixture.install_root, &fixture.codex_home).unwrap());
        let recovered: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(recovered["model"].as_str(), Some("gpt-native"));
        assert_eq!(
            recovered["app"]["last_opened"].as_str(),
            Some("desktop-current")
        );
        assert!(recovered.get("model_provider").is_none());
        assert!(recovered.get("model_catalog_json").is_none());
        assert!(
            recovered
                .get("model_providers")
                .and_then(toml::Value::as_table)
                .is_none_or(|providers| !providers.contains_key(PICKER_PROVIDER_ID))
        );
        assert_eq!(
            fs::read_to_string(&native_catalog).unwrap(),
            native_refreshed
        );
        assert_eq!(mode(&config), 0o644);
        assert!(!receipt.generated_catalog_path.exists());
        assert!(!receipt.native_route_path.exists());
        assert!(!receipt.managed_state_path.exists());
    }

    #[test]
    fn picker_uninstall_rejects_rewritten_managed_provider_tampering() {
        let fixture = Fixture::new();
        let (receipt, _native_catalog) = install_test_picker(&fixture, b"model = \"gpt-native\"\n");
        let config = fixture.codex_home.join("config.toml");
        let mut tampered: toml::Value =
            toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        tampered["model_providers"][PICKER_PROVIDER_ID]["base_url"] =
            toml::Value::String("http://127.0.0.1:9999/v1".to_owned());
        fs::write(&config, toml::to_string(&tampered).unwrap()).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();

        assert!(matches!(
            uninstall_picker(&fixture.install_root, &fixture.codex_home),
            Err(LifecycleError::InvalidPickerState)
        ));
        assert!(receipt.generated_catalog_path.exists());
        assert!(receipt.native_route_path.exists());
        assert!(receipt.managed_state_path.exists());
        assert_eq!(
            toml::from_str::<toml::Value>(&fs::read_to_string(&config).unwrap())
                .unwrap()["model_providers"][PICKER_PROVIDER_ID]["base_url"]
                .as_str(),
            Some("http://127.0.0.1:9999/v1")
        );
    }

    #[test]
    fn full_uninstall_preflight_rejects_picker_tampering_without_mutation() {
        let fixture = Fixture::new();
        let (receipt, _native_catalog) = install_test_picker(&fixture, b"model = \"gpt-native\"\n");
        let config = fixture.codex_home.join("config.toml");
        let mut tampered: toml::Value =
            toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        tampered["model_providers"][PICKER_PROVIDER_ID]["base_url"] =
            toml::Value::String("http://127.0.0.1:9999/v1".to_owned());
        let tampered = toml::to_string(&tampered).unwrap();
        fs::write(&config, &tampered).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();

        assert!(matches!(
            preflight_uninstall(&fixture.uninstall_request()),
            Err(LifecycleError::InvalidPickerState)
        ));
        assert_eq!(fs::read_to_string(&config).unwrap(), tampered);
        assert!(fixture.install_root.exists());
        assert!(receipt.generated_catalog_path.exists());
        assert!(receipt.native_route_path.exists());
        assert!(receipt.managed_state_path.exists());
    }

    #[test]
    fn full_uninstall_restores_native_codex_and_supports_clean_picker_reinstall() {
        let fixture = Fixture::new();
        let original = b"# native Codex\nmodel = \"gpt-native\"\n\n[features]\nfuture = true\n";
        let (_first_picker, native_catalog) = install_test_picker(&fixture, original);

        preflight_uninstall(&fixture.uninstall_request()).unwrap();
        uninstall(&fixture.uninstall_request()).unwrap();

        let config = fixture.codex_home.join("config.toml");
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!fixture.install_root.exists());
        assert!(!fixture.codex_home.join(PROFILE_FILE_NAME).exists());
        assert!(!fixture.launch_agent.exists());

        install(&fixture.install_request()).unwrap();
        let catalog = CatalogSnapshot::new(["grok-4.6"], Some("\"v2\"".to_owned())).unwrap();
        CatalogCache::new(fixture.install_root.join("state/models.json"))
            .persist(&catalog)
            .unwrap();
        let second_picker = install_picker(&PickerInstallRequest {
            install_root: fixture.install_root.clone(),
            codex_home: fixture.codex_home.clone(),
            native_catalog_path: native_catalog,
            grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
            native_upstream: NativeUpstream::ChatgptCodex,
            bind: "127.0.0.1:4545".parse().unwrap(),
            native_compatibility: false,
        })
        .unwrap();

        let reinstalled =
            toml::from_str::<toml::Value>(&fs::read_to_string(&second_picker.config_path).unwrap())
                .unwrap();
        assert_eq!(
            reinstalled["model_provider"].as_str(),
            Some(PICKER_PROVIDER_ID)
        );
        assert_eq!(
            reinstalled["model_catalog_json"].as_str(),
            second_picker.generated_catalog_path.to_str()
        );
        assert!(second_picker.generated_catalog_path.exists());
        assert!(second_picker.native_route_path.exists());
        assert!(second_picker.managed_state_path.exists());
    }

    #[test]
    fn picker_uninstall_recovers_the_concrete_pre_header_v1_1_provider_block() {
        let fixture = Fixture::new();
        let (receipt, _native_catalog) = install_test_picker(&fixture, b"model = \"gpt-native\"\n");
        let config = fixture.codex_home.join("config.toml");
        let generated_catalog = receipt.generated_catalog_path.to_str().unwrap();
        let capability = fs::read_to_string(
            fixture
                .install_root
                .join("secrets")
                .join("caller-capability"),
        )
        .unwrap();
        let legacy = format!(
            "model = \"gpt-native\"\nmodel_provider = {PICKER_PROVIDER_ID:?}\nmodel_catalog_json = {generated_catalog:?}\n\n[model_providers.{PICKER_PROVIDER_ID}]\nname = {PICKER_PROVIDER_NAME:?}\nbase_url = \"http://127.0.0.1:4545/_grok/{capability}/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\n\n[desktop]\nreparsed = true\n"
        );
        fs::write(&config, legacy).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();

        assert!(uninstall_picker(&fixture.install_root, &fixture.codex_home).unwrap());
        let recovered: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(recovered["model"].as_str(), Some("gpt-native"));
        assert_eq!(recovered["desktop"]["reparsed"].as_bool(), Some(true));
        assert!(recovered.get("model_provider").is_none());
        assert!(recovered.get("model_catalog_json").is_none());
        assert!(
            recovered
                .get("model_providers")
                .and_then(toml::Value::as_table)
                .is_none_or(|providers| !providers.contains_key(PICKER_PROVIDER_ID))
        );
    }

    #[test]
    fn picker_config_conflicts_fail_closed_before_publication() {
        let fixture = Fixture::new();
        install(&fixture.install_request()).unwrap();
        let catalog = CatalogSnapshot::new(["grok-4.6"], None).unwrap();
        CatalogCache::new(fixture.install_root.join("state/models.json"))
            .persist(&catalog)
            .unwrap();
        let native_catalog = fixture.home.join("native-models.json");
        fs::write(
            &native_catalog,
            br#"{"models":[{"slug":"gpt-native","display_name":"Native GPT","supported_reasoning_levels":[],"shell_type":"default","visibility":"list","supported_in_api":true,"priority":1,"base_instructions":"native","support_verbosity":true,"truncation_policy":{"mode":"tokens","limit":1000},"experimental_supported_tools":[]}]}"#,
        )
        .unwrap();
        fs::set_permissions(&native_catalog, fs::Permissions::from_mode(0o600)).unwrap();
        let config = fixture.codex_home.join("config.toml");
        let conflicts = [
            "model_provider = \"openai\"\n",
            "model_catalog_json = \"/tmp/user-catalog.json\"\n",
            "openai_base_url = \"https://example.invalid/v1\"\n",
            "[model_providers.grok_codex_picker]\nname = \"user-owned\"\n",
        ];

        for conflict in conflicts {
            fs::write(&config, conflict).unwrap();
            fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
            let result = install_picker(&PickerInstallRequest {
                install_root: fixture.install_root.clone(),
                codex_home: fixture.codex_home.clone(),
                native_catalog_path: native_catalog.clone(),
                grok_overlay_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Grok.md"),
                native_upstream: NativeUpstream::ChatgptCodex,
                bind: "127.0.0.1:4545".parse().unwrap(),
                native_compatibility: false,
            });
            assert!(matches!(result, Err(LifecycleError::PickerConfigConflict)));
            assert_eq!(fs::read_to_string(&config).unwrap(), conflict);
            assert!(
                !fixture
                    .install_root
                    .join("state/picker-models.json")
                    .exists()
            );
            assert!(
                !fixture
                    .install_root
                    .join("state/picker-native-route.json")
                    .exists()
            );
            assert!(
                !fixture
                    .install_root
                    .join("state/picker-managed-state.json")
                    .exists()
            );
        }
    }

    #[test]
    fn automatic_catalog_sync_absorbs_future_native_and_grok_models() {
        let fixture = Fixture::new();
        let original = b"model = \"gpt-native\"\n\n[features]\nfuture = true\n";
        let (_receipt, native_catalog) = install_test_picker(&fixture, original);
        let grok_catalog = fixture.install_root.join("state/models.json");

        let mut native: serde_json::Value =
            serde_json::from_slice(&fs::read(&native_catalog).unwrap()).unwrap();
        let mut astra = native["models"][0].clone();
        astra["slug"] = serde_json::Value::String("gpt-6-astra".to_owned());
        astra["display_name"] = serde_json::Value::String("GPT-6 Astra".to_owned());
        native["models"].as_array_mut().unwrap().push(astra);
        fs::write(&native_catalog, serde_json::to_vec_pretty(&native).unwrap()).unwrap();
        fs::set_permissions(&native_catalog, fs::Permissions::from_mode(0o600)).unwrap();
        CatalogCache::new(&grok_catalog)
            .persist(
                &CatalogSnapshot::new(["grok-4.7", "grok-4.6"], Some("\"v47\"".to_owned()))
                    .unwrap(),
            )
            .unwrap();

        let synced = sync_installed_picker_catalog(&grok_catalog)
            .unwrap()
            .unwrap();
        assert!(synced.changed);
        assert_eq!(synced.native_catalog.path(), native_catalog);
        assert_eq!(synced.grok_catalog.path(), grok_catalog);
        assert_eq!(synced.native_model_count, 2);
        assert_eq!(synced.grok_model_ids, ["grok-4.7", "grok-4.6"]);
        assert!(synced.native_route.contains("gpt-native"));
        assert!(synced.native_route.contains("gpt-6-astra"));

        let generated: serde_json::Value = serde_json::from_slice(
            &fs::read(fixture.install_root.join("state/picker-models.json")).unwrap(),
        )
        .unwrap();
        let slugs = generated["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["slug"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(slugs.contains(&"gpt-6-astra"));
        assert!(slugs.contains(&"grok-4.7"));

        let unchanged = sync_installed_picker_catalog(&grok_catalog)
            .unwrap()
            .unwrap();
        assert!(!unchanged.changed);
        assert!(uninstall_picker(&fixture.install_root, &fixture.codex_home).unwrap());
        assert_eq!(
            fs::read(fixture.codex_home.join("config.toml")).unwrap(),
            original
        );
    }

    #[test]
    fn unsafe_source_target_and_install_roots_fail_closed() {
        let fixture = Fixture::new();
        fs::create_dir(&fixture.install_root).unwrap();
        assert!(matches!(
            install(&fixture.install_request()),
            Err(LifecycleError::InstallRootAlreadyExists)
        ));
        fs::remove_dir(&fixture.install_root).unwrap();

        let source_link = fixture.source_binary.with_extension("link");
        std::os::unix::fs::symlink(&fixture.source_binary, &source_link).unwrap();
        let mut request = fixture.install_request();
        request.source_binary = source_link;
        assert!(matches!(
            install(&request),
            Err(LifecycleError::UnsafeSourceBinary)
        ));

        let mut broad = fixture.install_request();
        broad.install_root = fixture.home.clone();
        assert!(matches!(
            install(&broad),
            Err(LifecycleError::UnsafeInstallRoot)
        ));

        let profile = fixture.codex_home.join(PROFILE_FILE_NAME);
        std::os::unix::fs::symlink(&fixture.source_binary, &profile).unwrap();
        assert!(matches!(
            install(&fixture.install_request()),
            Err(LifecycleError::UnsafeExternalTarget)
        ));
    }

    #[test]
    fn doctor_passes_offline_and_detects_profile_and_binary_tampering() {
        let fixture = Fixture::new();
        install(&fixture.install_request()).unwrap();
        let report = doctor(&fixture.doctor_request()).unwrap();
        assert!(report.is_healthy(), "{report:?}");
        assert_eq!(report.checks.len(), 10);
        assert!(
            report
                .checks
                .iter()
                .all(|check| !check.message.contains("test-secret")
                    && !check.message.contains("test-user")
                    && !check.message.contains(fixture.credential.to_str().unwrap()))
        );

        let profile = fixture.codex_home.join(PROFILE_FILE_NAME);
        fs::write(&profile, "model = \"tampered\"\n").unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(
            fixture.install_root.join("bin/grok-codex-bridge"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let report = doctor(&fixture.doctor_request()).unwrap();
        assert!(!report.is_healthy());
        assert_eq!(
            report
                .checks
                .iter()
                .find(|check| check.id == "codex_profile")
                .unwrap()
                .status,
            DoctorCheckStatus::Failed
        );
        assert_eq!(
            report
                .checks
                .iter()
                .find(|check| check.id == "binary")
                .unwrap()
                .status,
            DoctorCheckStatus::Failed
        );
    }

    #[test]
    fn failure_after_profile_replacement_rolls_back_all_external_and_staged_state() {
        let fixture = Fixture::new();
        let profile = fixture.codex_home.join(PROFILE_FILE_NAME);
        fs::write(&profile, b"original profile bytes\n").unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        let error =
            install_inner(&fixture.install_request(), InstallFailpoint::AfterProfile).unwrap_err();
        assert!(matches!(error, LifecycleError::InjectedInstallFailure));
        assert_eq!(fs::read(profile).unwrap(), b"original profile bytes\n");
        assert!(!fixture.launch_agent.exists());
        assert!(!fixture.install_root.exists());
        let backups: Vec<_> = fs::read_dir(&fixture.codex_home)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("backup"))
            .collect();
        assert!(backups.is_empty());
    }
}
