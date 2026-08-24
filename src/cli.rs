use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "grok-codex-bridge",
    version,
    about = "Native local protocol bridge between Codex and Grok"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the capability-scoped loopback service.
    Run {
        /// Path to the versioned bridge TOML configuration.
        #[arg(long, value_name = "FILE")]
        config: PathBuf,
    },
    /// Operate the admitted Grok model catalog.
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
    /// Materialize the bridge and isolated Codex profile without activation.
    Install(InstallArgs),
    /// Check the installed files, credential, and launchd service state.
    Doctor(DoctorArgs),
    /// Inspect the configured Grok credential without revealing it.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Operate the installed user LaunchAgent.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Prepare or remove the Phase J merged picker and routing state.
    Picker {
        #[command(subcommand)]
        command: PickerCommand,
    },
    /// Stop the service and restore only manifest-owned install changes.
    Uninstall(LifecyclePathArgs),
    /// Report source/runtime capability without probing credentials or network.
    Status,
    /// Print the binary version.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum CatalogCommand {
    /// Refresh the last-known-good catalog once from the official xAI endpoint.
    Refresh {
        /// Path to the versioned bridge TOML configuration.
        #[arg(long, value_name = "FILE")]
        config: PathBuf,
    },
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Materialized executable to copy; defaults to this executable.
    #[arg(long, value_name = "FILE")]
    pub source_binary: Option<PathBuf>,
    #[command(flatten)]
    pub paths: LifecyclePathArgs,
    /// Loopback address for the bridge service.
    #[arg(long, default_value = "127.0.0.1:8746", value_name = "ADDRESS")]
    pub bind: String,
    /// Initial model for the isolated Grok profile.
    #[arg(long, default_value = "grok-4.6", value_name = "MODEL")]
    pub model: String,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[command(flatten)]
    pub paths: LifecyclePathArgs,
    /// Grok credential file to inspect read-only.
    #[arg(long, value_name = "FILE")]
    pub credential_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct LifecyclePathArgs {
    /// Bridge-owned installation root.
    #[arg(long, value_name = "DIRECTORY")]
    pub install_root: Option<PathBuf>,
    /// Codex home containing the isolated profile.
    #[arg(long, value_name = "DIRECTORY")]
    pub codex_home: Option<PathBuf>,
    /// User LaunchAgent plist managed by the bridge manifest.
    #[arg(long, value_name = "FILE")]
    pub launch_agent: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Report whether the official Grok session credential is available.
    Status,
    /// Ensure a usable credential, launching the official browser OAuth flow when needed.
    Ensure,
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Load and start the installed user LaunchAgent.
    Install(ServicePathArgs),
    /// Stop the installed user LaunchAgent without deleting its plist.
    Uninstall(ServicePathArgs),
    /// Report the installed user LaunchAgent state.
    Status(ServicePathArgs),
}

#[derive(Debug, Subcommand)]
pub enum PickerCommand {
    /// Generate the merged catalog and atomically publish managed loopback routing state.
    Install(PickerInstallArgs),
    /// Restore the exact pre-picker base configuration and remove generated picker state.
    Uninstall(LifecyclePathArgs),
}

#[derive(Debug, Args)]
pub struct PickerInstallArgs {
    #[command(flatten)]
    pub paths: LifecyclePathArgs,
    /// Current authoritative native Codex catalog JSON to copy into bridge-owned state.
    #[arg(long, value_name = "FILE")]
    pub native_catalog: PathBuf,
    /// Exact effective first-party Codex Responses base URL captured before loopback activation.
    #[arg(long, value_name = "URL")]
    pub native_upstream_base_url: String,
    /// Loopback address used by the installed bridge provider.
    #[arg(long, default_value = "127.0.0.1:8746", value_name = "ADDRESS")]
    pub bind: String,
    /// Grok.md SSOT read at catalog generation. Defaults to ./Grok.md when that file exists.
    #[arg(long, value_name = "FILE")]
    pub grok_overlay: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct ServicePathArgs {
    /// Bridge-owned installation root.
    #[arg(long, value_name = "DIRECTORY")]
    pub install_root: Option<PathBuf>,
    /// User LaunchAgent plist used by service install.
    #[arg(long, value_name = "FILE")]
    pub launch_agent: Option<PathBuf>,
}
