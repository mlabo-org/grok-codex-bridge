use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use grok_codex_bridge::cli::{
    AuthCommand, DoctorArgs, InstallArgs, LifecyclePathArgs, PickerCommand, PickerInstallArgs,
    ServiceCommand, ServicePathArgs,
};
use grok_codex_bridge::launchd::{
    LaunchAgentSpec, RECOMMENDED_LAUNCH_AGENT_LABEL, ServiceStatus, ServiceUninstallOutcome,
    service_install, service_status, service_uninstall,
};
use grok_codex_bridge::lifecycle::{
    AuthAvailability, DoctorCheckStatus, DoctorRequest, InstallRequest, UninstallRequest,
    PickerInstallRequest, auth_status, doctor, install, install_picker, uninstall, uninstall_picker,
};
use grok_codex_bridge::{
    CatalogCache, CatalogCommand, CatalogSnapshot, Cli, Command, CredentialStore, GrokClient,
    GrokConfig, ModelCatalog, NativeUpstream, RuntimeConfig, serve,
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
                "phase F source includes local Responses, reversible lifecycle, doctor, auth status, and launchd controls; this command does not inspect installation or activation"
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
        Some(Command::Service { command }) => command_result(service_command(command)),
        Some(Command::Picker { command }) => command_result(picker_command(command)),
        Some(Command::Uninstall(arguments)) => command_result(uninstall_command(arguments)),
    }
}

fn picker_command(command: PickerCommand) -> Result<ExitCode, OperationError> {
    match command {
        PickerCommand::Install(arguments) => picker_install_command(arguments),
        PickerCommand::Uninstall(arguments) => {
            let paths = resolve_lifecycle_paths(&arguments)?;
            let removed = uninstall_picker(&paths.install_root, &paths.codex_home)?;
            println!(
                "picker state: {}",
                if removed { "removed; restart the accepted Codex CLI/Desktop runtime before relying on configuration" } else { "not installed" }
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn picker_install_command(arguments: PickerInstallArgs) -> Result<ExitCode, OperationError> {
    let paths = resolve_lifecycle_paths(&arguments.paths)?;
    let native_catalog_path = require_absolute(arguments.native_catalog, "native catalog")?;
    let bind = arguments.bind.parse::<SocketAddr>().map_err(|_| OperationError::InvalidBind)?;
    let native_upstream = NativeUpstream::parse_base_url(&arguments.native_upstream_base_url)?;
    let receipt = install_picker(&PickerInstallRequest {
        install_root: paths.install_root,
        codex_home: paths.codex_home,
        native_catalog_path,
        native_upstream,
        bind,
    })?;
    println!(
        "picker state: generated {} native and {} admitted Grok models",
        receipt.native_model_count, receipt.grok_model_count
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
    let bind = arguments
        .bind
        .parse::<SocketAddr>()
        .map_err(|_| OperationError::InvalidBind)?;
    let spec = recommended_launch_agent(&paths.install_root)?;
    let launch_agent_contents = String::from_utf8(spec.render_plist())
        .map_err(|_| OperationError::InvalidLaunchAgentEncoding)?;
    let receipt = install(&InstallRequest {
        source_binary,
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
    let stopped = service_uninstall(&spec)?;
    let receipt = uninstall(&UninstallRequest {
        install_root: paths.install_root,
        codex_home: paths.codex_home,
        launch_agent_path: paths.launch_agent,
    })?;

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
    serve(config, catalog).await.map_err(RunError::Server)
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

    if config.refresh_on_start() {
        match refresh_catalog(config, &catalog).await {
            Ok(count) => tracing::info!(models = count, "model catalog refreshed"),
            Err(error) => {
                tracing::warn!(error_class = error.class(), "model catalog refresh skipped")
            }
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
    #[error("the bind address must be a valid socket address")]
    InvalidBind,
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
}
