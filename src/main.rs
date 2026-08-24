use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use clap::{CommandFactory, Parser};
use grok_codex_bridge::cli::{
    AuthCommand, DesktopSwitchArgs, DoctorArgs, InstallArgs, LifecyclePathArgs, ModeCommand,
    PickerCommand, PickerInstallArgs, ServiceCommand, ServicePathArgs,
};
use grok_codex_bridge::desktop_transition;
use grok_codex_bridge::launchd::{
    LaunchAgentSpec, RECOMMENDED_LAUNCH_AGENT_LABEL, ServiceStatus, ServiceUninstallOutcome,
    service_install, service_status, service_uninstall,
};
use grok_codex_bridge::lifecycle::{
    AuthAvailability, DoctorCheckStatus, DoctorRequest, InstallRequest, PickerInstallRequest,
    UninstallRequest, auth_status, doctor, install, preflight_uninstall, uninstall,
    uninstall_picker,
};
use grok_codex_bridge::picker_activation::{PickerActivationRequest, activate_picker};
use grok_codex_bridge::{
    CatalogCache, CatalogCommand, CatalogSnapshot, Cli, Command, CredentialStore, GrokClient,
    GrokConfig, ModelCatalog, NativeUpstream, RuntimeConfig, bind,
};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        None => {
            if Cli::command().print_help().is_err() {
                return ExitCode::FAILURE;
            }
            println!();
            ExitCode::SUCCESS
        }
        Some(Command::Status) => {
            println!(
                "phase F source includes local Responses, reversible lifecycle, doctor, auth status/ensure, and launchd controls; this command does not inspect installation or activation"
            );
            ExitCode::SUCCESS
        }
        Some(Command::Version) => {
            println!("grok-codex-bridge {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(Command::Run { config }) => {
            init_tracing();
            match run(&config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    tracing::error!(error_class = error.class(), "service did not start");
                    eprintln!("error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(Command::Catalog {
            command: CatalogCommand::Refresh { config },
        }) => {
            init_tracing();
            match refresh_command(&config).await {
                Ok(count) => {
                    println!("refreshed {count} admitted Grok models");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    tracing::error!(error_class = error.class(), "catalog refresh failed");
                    eprintln!("error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(Command::Install(arguments)) => command_result(install_command(arguments)),
        Some(Command::Doctor(arguments)) => command_result(doctor_command(arguments)),
        Some(Command::Auth {
            command: AuthCommand::Status,
        }) => command_result(auth_status_command()),
        Some(Command::Auth {
            command: AuthCommand::Ensure,
        }) => command_result(auth_ensure_command()),
        Some(Command::Service { command }) => command_result(service_command(command)),
        Some(Command::Picker { command }) => command_result(picker_command(command)),
        Some(Command::Mode { command }) => command_result(mode_command(command).await),
        Some(Command::Switch(arguments)) => command_result(desktop_switch_command(arguments)),
        Some(Command::Uninstall(arguments)) => command_result(uninstall_command(arguments)),
    }
}

async fn mode_command(command: ModeCommand) -> Result<ExitCode, OperationError> {
    let lifecycle = LifecyclePathArgs {
        install_root: None,
        codex_home: None,
        launch_agent: None,
    };
    let paths = resolve_lifecycle_paths(&lifecycle)?;
    let installed_binary = paths.install_root.join("bin/grok-codex-bridge");
    let installed_launcher = paths.install_root.join("bin/Grok Codex Switch.app");
    let installed_overlay =
        installed_launcher.join("Contents/Resources/grok-codex-bridge-overlay.md");
    let native_catalog = paths.codex_home.join("models_cache.json");
    let config = paths.install_root.join("config/bridge.toml");

    preflight_uninstall(&UninstallRequest {
        install_root: paths.install_root.clone(),
        codex_home: paths.codex_home.clone(),
        launch_agent_path: paths.launch_agent.clone(),
    })?;
    require_regular_file(&native_catalog, "Native Codex catalog")?;
    require_regular_file(&installed_overlay, "installed Grok overlay")?;
    verify_chatgpt_native_route()?;

    if matches!(command, ModeCommand::Grok) {
        auth_ensure_command()?;
        let count = refresh_command(&config).await?;
        println!("refreshed {count} admitted Grok models");
    }

    let mut switch_arguments = vec![
        installed_binary.as_os_str().to_owned(),
        "switch".into(),
        "--native-catalog".into(),
        native_catalog.as_os_str().to_owned(),
        "--native-upstream-base-url".into(),
        "https://chatgpt.com/backend-api/codex".into(),
        "--grok-overlay".into(),
        installed_overlay.as_os_str().to_owned(),
    ];
    if matches!(command, ModeCommand::Native) {
        switch_arguments.push("--native-compatibility".into());
    }

    let switch_log = paths.install_root.join("logs/mode-switch.log");
    append_mode_request(&switch_log, command)?;
    let status = ProcessCommand::new("/usr/bin/open")
        .arg("-g")
        .arg(&installed_launcher)
        .arg("--args")
        .args(switch_arguments)
        .status()
        .map_err(OperationError::LaunchModeSwitcher)?;
    if !status.success() {
        return Err(OperationError::ModeSwitcherFailed(status.code()));
    }

    println!(
        "{} mode switch handed off to installed native launcher",
        match command {
            ModeCommand::Grok => "grok",
            ModeCommand::Native => "native",
        }
    );
    println!("estimated completion time: approximately 15-20 seconds");
    println!("ChatGPT.app will quit gracefully and relaunch automatically");
    println!("transition log: {}", switch_log.display());
    Ok(ExitCode::SUCCESS)
}

fn require_regular_file(path: &Path, label: &'static str) -> Result<(), OperationError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| OperationError::InspectModeInput { label, source })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(OperationError::UnsafeModeInput { label });
    }
    Ok(())
}

fn verify_chatgpt_native_route() -> Result<(), OperationError> {
    let binary = Path::new("/Applications/ChatGPT.app/Contents/Resources/codex");
    require_regular_file(binary, "ChatGPT.app Codex executable")?;
    let output = ProcessCommand::new(binary)
        .args(["login", "status"])
        .stdin(Stdio::null())
        .output()
        .map_err(OperationError::InspectChatgptLogin)?;
    if !chatgpt_login_status_is_supported(output.status.success(), &output.stdout, &output.stderr) {
        return Err(OperationError::UnsupportedChatgptLogin);
    }
    Ok(())
}

fn chatgpt_login_status_is_supported(success: bool, stdout: &[u8], stderr: &[u8]) -> bool {
    const EXPECTED: &str = "Logged in using ChatGPT";
    success
        && (String::from_utf8_lossy(stdout).trim() == EXPECTED
            || String::from_utf8_lossy(stderr).trim() == EXPECTED)
}

fn append_mode_request(path: &Path, command: ModeCommand) -> Result<(), OperationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            return Err(OperationError::UnsafeModeLog);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(OperationError::InspectModeLog(error)),
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(OperationError::OpenModeLog)?;
    writeln!(
        log,
        "{} mode switch requested at {}",
        match command {
            ModeCommand::Grok => "grok",
            ModeCommand::Native => "native",
        },
        chrono::Utc::now().to_rfc3339()
    )
    .map_err(OperationError::WriteModeLog)
}

fn desktop_switch_command(arguments: DesktopSwitchArgs) -> Result<ExitCode, OperationError> {
    if arguments.grace_period_ms > 5_000 {
        return Err(OperationError::InvalidGracePeriod);
    }
    let paths = resolve_lifecycle_paths(&arguments.picker.paths)?;
    let source_binary = env::current_exe().map_err(OperationError::CurrentExecutable)?;
    let installed_binary = paths.install_root.join("bin/grok-codex-bridge");
    let installed_launcher = paths.install_root.join("bin/Grok Codex Switch.app");
    thread::sleep(Duration::from_millis(arguments.grace_period_ms));

    desktop_transition::transition(
        Duration::from_secs(30),
        Duration::from_millis(100),
        || -> Result<(), OperationError> {
            replace_installed_runtime_if_needed(
                &source_binary,
                &installed_binary,
                &installed_launcher,
                arguments.replacement_script.as_deref(),
                arguments.replacement_launcher.as_deref(),
                &paths,
            )?;
            picker_install_command(arguments.picker).map(|_| ())
        },
    )?;
    println!("desktop mode switch: complete");
    Ok(ExitCode::SUCCESS)
}

fn replace_installed_runtime_if_needed(
    source_binary: &Path,
    installed_binary: &Path,
    installed_launcher: &Path,
    replacement_script: Option<&Path>,
    replacement_launcher: Option<&Path>,
    paths: &LifecyclePaths,
) -> Result<(), OperationError> {
    let launcher_equal = match replacement_launcher {
        Some(source_launcher) => launcher_bundles_equal(source_launcher, installed_launcher)?,
        None => true,
    };
    if files_equal(source_binary, installed_binary)? && launcher_equal {
        return Ok(());
    }
    let script = replacement_script.ok_or(OperationError::ReplacementRequired)?;
    let launcher = replacement_launcher.ok_or(OperationError::ReplacementLauncherRequired)?;
    let metadata =
        fs::symlink_metadata(script).map_err(OperationError::InspectReplacementScript)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(OperationError::UnsafeReplacementScript);
    }
    let spec = recommended_launch_agent(&paths.install_root)?;
    match service_status(&spec)? {
        ServiceStatus::Loaded => {}
        ServiceStatus::NotLoaded => service_install(&spec, &paths.launch_agent)?,
        status @ ServiceStatus::Failed { .. } => {
            return Err(OperationError::UnexpectedServiceStatus { status });
        }
    }
    let status = ProcessCommand::new(script)
        .arg(source_binary)
        .arg(launcher)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(OperationError::RunReplacementScript)?;
    if !status.success() {
        return Err(OperationError::ReplacementScriptFailed(status.code()));
    }
    Ok(())
}

fn launcher_bundles_equal(left: &Path, right: &Path) -> Result<bool, OperationError> {
    for relative in [
        "Contents/MacOS/Grok Codex Switch",
        "Contents/Info.plist",
        "Contents/Resources/grok-codex-bridge-overlay.md",
    ] {
        let left_bytes =
            fs::read(left.join(relative)).map_err(OperationError::ReadBinaryForComparison)?;
        let right_bytes = match fs::read(right.join(relative)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(OperationError::ReadBinaryForComparison(error)),
        };
        if left_bytes != right_bytes {
            return Ok(false);
        }
    }
    Ok(true)
}

fn files_equal(left: &Path, right: &Path) -> Result<bool, OperationError> {
    let left = fs::read(left).map_err(OperationError::ReadBinaryForComparison)?;
    let right = fs::read(right).map_err(OperationError::ReadBinaryForComparison)?;
    Ok(left == right)
}

fn picker_command(command: PickerCommand) -> Result<ExitCode, OperationError> {
    match command {
        PickerCommand::Install(arguments) => picker_install_command(arguments),
        PickerCommand::Uninstall(arguments) => {
            let paths = resolve_lifecycle_paths(&arguments)?;
            let removed = uninstall_picker(&paths.install_root, &paths.codex_home)?;
            println!(
                "picker state: {}",
                if removed {
                    "removed; restart the accepted Codex CLI/Desktop runtime before relying on configuration"
                } else {
                    "not installed"
                }
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn picker_install_command(arguments: PickerInstallArgs) -> Result<ExitCode, OperationError> {
    let paths = resolve_lifecycle_paths(&arguments.paths)?;
    let native_catalog_path = require_absolute(arguments.native_catalog, "native catalog")?;
    let grok_overlay_path = match arguments.grok_overlay {
        Some(path) => require_absolute(path, "Grok overlay")?,
        None => {
            let candidate = env::current_dir()
                .map_err(OperationError::CurrentExecutable)?
                .join("Grok.md");
            if candidate.is_file() {
                require_absolute(candidate, "Grok overlay")?
            } else {
                return Err(OperationError::MissingGrokOverlay);
            }
        }
    };
    let bind = arguments
        .bind
        .parse::<SocketAddr>()
        .map_err(|_| OperationError::InvalidBind)?;
    let native_upstream = NativeUpstream::parse_base_url(&arguments.native_upstream_base_url)?;
    let launch_agent = recommended_launch_agent(&paths.install_root)?;
    let receipt = activate_picker(&PickerActivationRequest {
        picker: PickerInstallRequest {
            install_root: paths.install_root,
            codex_home: paths.codex_home,
            native_catalog_path,
            grok_overlay_path,
            native_upstream,
            bind,
            native_compatibility: arguments.native_compatibility,
        },
        launch_agent,
        launch_agent_path: paths.launch_agent,
    })?;
    println!(
        "picker state: generated {} native and {} admitted Grok models",
        receipt.picker.native_model_count, receipt.picker.grok_model_count
    );
    println!(
        "restart required: start a fresh Codex CLI process or fully relaunch Desktop after bridge publication"
    );
    Ok(ExitCode::SUCCESS)
}

fn command_result(result: Result<ExitCode, OperationError>) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn install_command(arguments: InstallArgs) -> Result<ExitCode, OperationError> {
    let paths = resolve_lifecycle_paths(&arguments.paths)?;
    let source_binary = match arguments.source_binary {
        Some(path) => require_absolute(path, "source binary")?,
        None => env::current_exe().map_err(OperationError::CurrentExecutable)?,
    };
    let source_launcher = require_absolute(arguments.source_launcher, "source launcher")?;
    let bind = arguments
        .bind
        .parse::<SocketAddr>()
        .map_err(|_| OperationError::InvalidBind)?;
    let spec = recommended_launch_agent(&paths.install_root)?;
    let launch_agent_contents = String::from_utf8(spec.render_plist())
        .map_err(|_| OperationError::InvalidLaunchAgentEncoding)?;
    let receipt = install(&InstallRequest {
        source_binary,
        source_launcher,
        install_root: paths.install_root,
        codex_home: paths.codex_home,
        launch_agent_path: paths.launch_agent,
        launch_agent_label: spec.label().to_owned(),
        launch_agent_contents,
        bind,
        initial_model: arguments.model,
    })?;

    println!("install: complete");
    println!(
        "codex profile: {}",
        if receipt.profile_replaced {
            "installed with a reversible backup"
        } else {
            "installed"
        }
    );
    println!(
        "launch agent: {}",
        if receipt.launch_agent_replaced {
            "materialized with a reversible backup"
        } else {
            "materialized"
        }
    );
    println!("next: grok-codex-bridge service install");
    println!("next: codex --profile grok-bridge");
    Ok(ExitCode::SUCCESS)
}

fn doctor_command(arguments: DoctorArgs) -> Result<ExitCode, OperationError> {
    let paths = resolve_lifecycle_paths(&arguments.paths)?;
    let credential_path = match arguments.credential_file {
        Some(path) => require_absolute(path, "credential file")?,
        None => resolve_credential_path()?,
    };
    let spec = recommended_launch_agent(&paths.install_root)?;
    let report = doctor(&DoctorRequest {
        install_root: paths.install_root,
        codex_home: paths.codex_home,
        launch_agent_path: paths.launch_agent,
        credential_path,
    })?;
    let service = service_status(&spec)?;

    for check in &report.checks {
        let status = match check.status {
            DoctorCheckStatus::Passed => "passed",
            DoctorCheckStatus::Failed => "failed",
        };
        println!("check {} {status}: {}", check.id, check.message);
    }
    print_service_status(service);

    if report.is_healthy() && !matches!(service, ServiceStatus::Failed { .. }) {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn auth_status_command() -> Result<ExitCode, OperationError> {
    let store = CredentialStore::from_environment()?;
    let status = auth_status(&store);
    let availability = match status.availability {
        AuthAvailability::Available => "available",
        AuthAvailability::Unavailable => "unavailable",
    };
    println!("auth {availability}: {}", status.message);
    Ok(if status.availability == AuthAvailability::Available {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn auth_ensure_command() -> Result<ExitCode, OperationError> {
    let store = CredentialStore::from_environment()?;
    store.ensure_with_official_login()?;
    println!("auth available: official Grok session credential is valid");
    Ok(ExitCode::SUCCESS)
}

fn service_command(command: ServiceCommand) -> Result<ExitCode, OperationError> {
    match command {
        ServiceCommand::Install(arguments) => {
            let paths = resolve_service_paths(&arguments)?;
            let spec = recommended_launch_agent(&paths.install_root)?;
            service_install(&spec, &paths.launch_agent)?;
            println!("service installed");
            Ok(ExitCode::SUCCESS)
        }
        ServiceCommand::Uninstall(arguments) => {
            let paths = resolve_service_paths(&arguments)?;
            let spec = recommended_launch_agent(&paths.install_root)?;
            print_service_uninstall(service_uninstall(&spec)?);
            Ok(ExitCode::SUCCESS)
        }
        ServiceCommand::Status(arguments) => {
            let paths = resolve_service_paths(&arguments)?;
            let spec = recommended_launch_agent(&paths.install_root)?;
            let status = service_status(&spec)?;
            print_service_status(status);
            Ok(if matches!(status, ServiceStatus::Failed { .. }) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

fn uninstall_command(arguments: LifecyclePathArgs) -> Result<ExitCode, OperationError> {
    let paths = resolve_lifecycle_paths(&arguments)?;
    let spec = recommended_launch_agent(&paths.install_root)?;
    let request = UninstallRequest {
        install_root: paths.install_root.clone(),
        codex_home: paths.codex_home,
        launch_agent_path: paths.launch_agent.clone(),
    };
    preflight_uninstall(&request)?;
    let prior_status = service_status(&spec)?;
    let stopped = service_uninstall(&spec)?;
    let receipt = match uninstall(&request) {
        Ok(receipt) => receipt,
        Err(error) => {
            if matches!(prior_status, ServiceStatus::Loaded) {
                service_install(&spec, &paths.launch_agent)?;
            }
            return Err(error.into());
        }
    };

    print_service_uninstall(stopped);
    println!("uninstall: complete");
    println!(
        "codex profile: {}",
        if receipt.profile_restored {
            "restored"
        } else {
            "removed"
        }
    );
    println!(
        "launch agent: {}",
        if receipt.launch_agent_restored {
            "restored"
        } else {
            "removed"
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn print_service_status(status: ServiceStatus) {
    println!(
        "service {}",
        match status {
            ServiceStatus::Loaded => "loaded",
            ServiceStatus::NotLoaded => "not_loaded",
            ServiceStatus::Failed { .. } => "failed",
        }
    );
}

fn print_service_uninstall(outcome: ServiceUninstallOutcome) {
    println!(
        "service {}",
        match outcome {
            ServiceUninstallOutcome::Stopped => "stopped",
            ServiceUninstallOutcome::AlreadyStopped => "already_stopped",
        }
    );
}

struct LifecyclePaths {
    install_root: PathBuf,
    codex_home: PathBuf,
    launch_agent: PathBuf,
}

struct ServicePaths {
    install_root: PathBuf,
    launch_agent: PathBuf,
}

fn resolve_lifecycle_paths(
    arguments: &LifecyclePathArgs,
) -> Result<LifecyclePaths, OperationError> {
    let install_root = match arguments.install_root.clone() {
        Some(path) => require_absolute(path, "install root")?,
        None => home_dir()?.join("Library/Application Support/grok-codex-bridge"),
    };
    let codex_home = match arguments.codex_home.clone() {
        Some(path) => require_absolute(path, "Codex home")?,
        None => match environment_path("CODEX_HOME")? {
            Some(path) => path,
            None => home_dir()?.join(".codex"),
        },
    };
    let launch_agent = match arguments.launch_agent.clone() {
        Some(path) => require_absolute(path, "LaunchAgent")?,
        None => home_dir()?.join(format!(
            "Library/LaunchAgents/{RECOMMENDED_LAUNCH_AGENT_LABEL}.plist"
        )),
    };
    Ok(LifecyclePaths {
        install_root,
        codex_home,
        launch_agent,
    })
}

fn resolve_service_paths(arguments: &ServicePathArgs) -> Result<ServicePaths, OperationError> {
    let install_root = match arguments.install_root.clone() {
        Some(path) => require_absolute(path, "install root")?,
        None => home_dir()?.join("Library/Application Support/grok-codex-bridge"),
    };
    let launch_agent = match arguments.launch_agent.clone() {
        Some(path) => require_absolute(path, "LaunchAgent")?,
        None => home_dir()?.join(format!(
            "Library/LaunchAgents/{RECOMMENDED_LAUNCH_AGENT_LABEL}.plist"
        )),
    };
    Ok(ServicePaths {
        install_root,
        launch_agent,
    })
}

fn resolve_credential_path() -> Result<PathBuf, OperationError> {
    if let Some(path) = environment_path("GROK_AUTH_PATH")? {
        return Ok(path);
    }
    if let Some(path) = environment_path("GROK_HOME")? {
        return Ok(path.join("auth.json"));
    }
    Ok(home_dir()?.join(".grok/auth.json"))
}

fn environment_path(name: &'static str) -> Result<Option<PathBuf>, OperationError> {
    let Some(value) = env::var_os(name).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        Err(OperationError::RelativeEnvironmentPath { name })
    }
}

fn home_dir() -> Result<PathBuf, OperationError> {
    environment_path("HOME")?.ok_or(OperationError::HomeUnavailable)
}

fn require_absolute(path: PathBuf, field: &'static str) -> Result<PathBuf, OperationError> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(OperationError::RelativeArgumentPath { field })
    }
}

fn recommended_launch_agent(install_root: &Path) -> Result<LaunchAgentSpec, OperationError> {
    LaunchAgentSpec::recommended(
        install_root.join("bin/grok-codex-bridge"),
        install_root.join("config/bridge.toml"),
        install_root.join("logs/stdout.log"),
        install_root.join("logs/stderr.log"),
    )
    .map_err(OperationError::Launchd)
}

async fn run(path: &std::path::Path) -> Result<(), RunError> {
    let config = RuntimeConfig::load(path).map_err(RunError::Config)?;
    let grok = config.grok().clone();
    let catalog = prepare_catalog(&grok).await?;
    // Bind the loopback listener before contacting the remote catalog endpoint.
    // The refresh is optional startup enrichment; it must not delay provider
    // readiness or hide an actual server failure.
    let server = bind(config, catalog.clone())
        .await
        .map_err(RunError::Server)?;
    let server_future = server.serve();
    tokio::pin!(server_future);

    if grok.refresh_on_start() {
        let refresh_future = refresh_catalog(&grok, &catalog);
        tokio::pin!(refresh_future);
        tokio::select! {
            result = &mut server_future => return result.map_err(RunError::Server),
            result = &mut refresh_future => match result {
                Ok(count) => tracing::info!(models = count, "model catalog refreshed"),
                Err(error) => tracing::warn!(error_class = error.class(), "model catalog refresh skipped"),
            },
        }
    }

    server_future.await.map_err(RunError::Server)
}

async fn prepare_catalog(config: &GrokConfig) -> Result<ModelCatalog, RunError> {
    let catalog = ModelCatalog::bootstrap().map_err(RunError::Catalog)?;
    let cache = CatalogCache::new(config.catalog_cache_file());
    match cache.load() {
        Ok(Some(snapshot)) => {
            catalog
                .replace(snapshot.model_ids().iter().cloned())
                .await
                .map_err(RunError::Catalog)?;
            tracing::info!(
                models = snapshot.model_ids().len(),
                "loaded model catalog cache"
            );
        }
        Ok(None) => {}
        Err(_) => {
            tracing::warn!(
                error_class = "catalog_cache",
                "model catalog cache was not admitted"
            );
        }
    }

    Ok(catalog)
}

async fn refresh_command(path: &std::path::Path) -> Result<usize, RefreshError> {
    let config = RuntimeConfig::load(path).map_err(RefreshError::Config)?;
    let catalog = ModelCatalog::bootstrap().map_err(RefreshError::Catalog)?;
    refresh_catalog(config.grok(), &catalog).await
}

async fn refresh_catalog(
    config: &GrokConfig,
    catalog: &ModelCatalog,
) -> Result<usize, RefreshError> {
    let credential_store = CredentialStore::from_environment().map_err(RefreshError::Credential)?;
    let credential = credential_store.load().map_err(RefreshError::Credential)?;
    let client = GrokClient::production().map_err(RefreshError::Grok)?;
    let fetched = client
        .fetch_models(&credential)
        .await
        .map_err(RefreshError::Grok)?;
    let snapshot =
        CatalogSnapshot::new(fetched.models, fetched.etag).map_err(RefreshError::Catalog)?;
    CatalogCache::new(config.catalog_cache_file())
        .persist(&snapshot)
        .map_err(RefreshError::Catalog)?;
    catalog
        .replace(snapshot.model_ids().iter().cloned())
        .await
        .map_err(RefreshError::Catalog)?;
    Ok(snapshot.model_ids().len())
}

fn init_tracing() {
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("trace") => LevelFilter::TRACE,
        Ok("debug") => LevelFilter::DEBUG,
        Ok("warn") => LevelFilter::WARN,
        Ok("error") => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    };
    let filter = Targets::new()
        .with_default(LevelFilter::OFF)
        .with_target("grok_codex_bridge", level);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false),
        )
        .try_init();
}

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error(transparent)]
    Config(#[from] grok_codex_bridge::ConfigError),
    #[error(transparent)]
    Catalog(#[from] grok_codex_bridge::CatalogError),
    #[error(transparent)]
    Server(#[from] grok_codex_bridge::ServerError),
}

impl RunError {
    fn class(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::Catalog(_) => "catalog",
            Self::Server(_) => "server",
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum RefreshError {
    #[error(transparent)]
    Config(#[from] grok_codex_bridge::ConfigError),
    #[error(transparent)]
    Catalog(#[from] grok_codex_bridge::CatalogError),
    #[error(transparent)]
    Credential(#[from] grok_codex_bridge::CredentialError),
    #[error(transparent)]
    Grok(#[from] grok_codex_bridge::GrokError),
}

impl RefreshError {
    fn class(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::Catalog(_) => "catalog",
            Self::Credential(_) => "credential",
            Self::Grok(_) => "upstream",
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum OperationError {
    #[error("HOME must name an absolute directory for lifecycle defaults")]
    HomeUnavailable,
    #[error("{name} must be an absolute path")]
    RelativeEnvironmentPath { name: &'static str },
    #[error("{field} must be an absolute path")]
    RelativeArgumentPath { field: &'static str },
    #[error("the current executable could not be resolved")]
    CurrentExecutable(#[source] std::io::Error),
    #[error("desktop switch grace period must not exceed 5000 milliseconds")]
    InvalidGracePeriod,
    #[error("failed to inspect {label}")]
    InspectModeInput {
        label: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("{label} must be a non-empty regular non-symlink file")]
    UnsafeModeInput { label: &'static str },
    #[error("failed to inspect the ChatGPT.app login route")]
    InspectChatgptLogin(#[source] std::io::Error),
    #[error("ChatGPT.app must be logged in using ChatGPT before switching modes")]
    UnsupportedChatgptLogin,
    #[error("failed to inspect the mode switch log")]
    InspectModeLog(#[source] std::io::Error),
    #[error("the mode switch log must be a regular non-symlink file")]
    UnsafeModeLog,
    #[error("failed to open the mode switch log")]
    OpenModeLog(#[source] std::io::Error),
    #[error("failed to write the mode switch log")]
    WriteModeLog(#[source] std::io::Error),
    #[error("failed to launch the installed native mode switcher")]
    LaunchModeSwitcher(#[source] std::io::Error),
    #[error("the installed native mode switcher failed with exit code {0:?}")]
    ModeSwitcherFailed(Option<i32>),
    #[error("installed runtime differs; an explicit source-owned replacement script is required")]
    ReplacementRequired,
    #[error("installed runtime differs; its paired materialized launcher is required")]
    ReplacementLauncherRequired,
    #[error("failed to inspect the installed-binary replacement script")]
    InspectReplacementScript(#[source] std::io::Error),
    #[error("installed-binary replacement script must be a regular non-symlink file")]
    UnsafeReplacementScript,
    #[error("failed to read a native runtime component for installed-byte comparison")]
    ReadBinaryForComparison(#[source] std::io::Error),
    #[error("installed bridge service has an unexpected state: {status:?}")]
    UnexpectedServiceStatus { status: ServiceStatus },
    #[error("failed to run the installed-binary replacement script")]
    RunReplacementScript(#[source] std::io::Error),
    #[error("installed-binary replacement script failed with exit code {0:?}")]
    ReplacementScriptFailed(Option<i32>),
    #[error("the bind address must be a valid socket address")]
    InvalidBind,
    #[error(
        "Grok overlay was not provided; pass --grok-overlay or run from the repo root that contains Grok.md"
    )]
    MissingGrokOverlay,
    #[error("the rendered LaunchAgent was not UTF-8")]
    InvalidLaunchAgentEncoding,
    #[error(transparent)]
    Lifecycle(#[from] grok_codex_bridge::lifecycle::LifecycleError),
    #[error(transparent)]
    Launchd(#[from] grok_codex_bridge::launchd::LaunchdError),
    #[error(transparent)]
    Credential(#[from] grok_codex_bridge::CredentialError),
    #[error(transparent)]
    Native(#[from] grok_codex_bridge::native::NativeError),
    #[error(transparent)]
    PickerActivation(#[from] grok_codex_bridge::picker_activation::PickerActivationError),
    #[error(transparent)]
    DesktopTransition(#[from] grok_codex_bridge::desktop_transition::DesktopTransitionError),
    #[error(transparent)]
    Refresh(#[from] RefreshError),
}

#[cfg(test)]
mod tests {
    use super::chatgpt_login_status_is_supported;

    #[test]
    fn chatgpt_login_status_accepts_the_official_stderr_channel_only() {
        assert!(chatgpt_login_status_is_supported(
            true,
            b"",
            b"Logged in using ChatGPT\n"
        ));
        assert!(!chatgpt_login_status_is_supported(
            true,
            b"",
            b"Logged in using API key\n"
        ));
        assert!(!chatgpt_login_status_is_supported(
            false,
            b"",
            b"Logged in using ChatGPT\n"
        ));
    }
}
