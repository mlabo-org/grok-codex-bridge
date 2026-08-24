use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use thiserror::Error;

pub const RECOMMENDED_LAUNCH_AGENT_LABEL: &str = "com.local.grok-codex-bridge";

const LAUNCHCTL_PATH: &str = "/bin/launchctl";
const INSTALLED_BINARY_NAME: &str = "grok-codex-bridge";
const LAUNCH_AGENT_MODE: u32 = 0o644;
const MAX_LAUNCH_AGENT_BYTES: u64 = 1024 * 1024;
const SERVICE_STATE_POLL_ATTEMPTS: usize = 100;
const SERVICE_STATE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAgentSpec {
    label: String,
    installed_binary: PathBuf,
    bridge_config: PathBuf,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl LaunchAgentSpec {
    pub fn new(
        label: impl Into<String>,
        installed_binary: impl Into<PathBuf>,
        bridge_config: impl Into<PathBuf>,
        stdout_log: impl Into<PathBuf>,
        stderr_log: impl Into<PathBuf>,
    ) -> Result<Self, LaunchdError> {
        let label = label.into();
        validate_label(&label)?;

        let installed_binary = installed_binary.into();
        validate_path("installed binary", &installed_binary)?;
        validate_installed_binary(&installed_binary)?;

        let bridge_config = bridge_config.into();
        validate_path("bridge config", &bridge_config)?;
        let stdout_log = stdout_log.into();
        validate_path("stdout log", &stdout_log)?;
        let stderr_log = stderr_log.into();
        validate_path("stderr log", &stderr_log)?;

        Ok(Self {
            label,
            installed_binary,
            bridge_config,
            stdout_log,
            stderr_log,
        })
    }

    #[must_use]
    pub fn recommended(
        installed_binary: impl Into<PathBuf>,
        bridge_config: impl Into<PathBuf>,
        stdout_log: impl Into<PathBuf>,
        stderr_log: impl Into<PathBuf>,
    ) -> Result<Self, LaunchdError> {
        Self::new(
            RECOMMENDED_LAUNCH_AGENT_LABEL,
            installed_binary,
            bridge_config,
            stdout_log,
            stderr_log,
        )
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn installed_binary(&self) -> &Path {
        &self.installed_binary
    }

    #[must_use]
    pub fn bridge_config(&self) -> &Path {
        &self.bridge_config
    }

    #[must_use]
    pub fn stdout_log(&self) -> &Path {
        &self.stdout_log
    }

    #[must_use]
    pub fn stderr_log(&self) -> &Path {
        &self.stderr_log
    }

    #[must_use]
    pub fn render_plist(&self) -> Vec<u8> {
        let label = escape_xml(&self.label);
        let binary = escape_xml(path_text(&self.installed_binary));
        let config = escape_xml(path_text(&self.bridge_config));
        let stdout = escape_xml(path_text(&self.stdout_log));
        let stderr = escape_xml(path_text(&self.stderr_log));

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>Label</key>\n\
  <string>{label}</string>\n\
  <key>ProgramArguments</key>\n\
  <array>\n\
    <string>{binary}</string>\n\
    <string>run</string>\n\
    <string>--config</string>\n\
    <string>{config}</string>\n\
  </array>\n\
  <key>RunAtLoad</key>\n\
  <true/>\n\
  <key>KeepAlive</key>\n\
  <true/>\n\
  <key>ProcessType</key>\n\
  <string>Background</string>\n\
  <key>StandardOutPath</key>\n\
  <string>{stdout}</string>\n\
  <key>StandardErrorPath</key>\n\
  <string>{stderr}</string>\n\
  <key>EnvironmentVariables</key>\n\
  <dict>\n\
    <key>RUST_LOG</key>\n\
    <string>info</string>\n\
  </dict>\n\
</dict>\n\
</plist>\n"
        )
        .into_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceUninstallOutcome {
    Stopped,
    AlreadyStopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceStatus {
    Loaded,
    NotLoaded,
    Failed { exit_code: Option<i32> },
}

pub fn service_install(spec: &LaunchAgentSpec, plist_path: &Path) -> Result<(), LaunchdError> {
    let uid = effective_user_id()?;
    let mut runner = SystemLaunchctlRunner;
    service_install_and_wait_with_runner(
        spec,
        plist_path,
        uid,
        &mut runner,
        SERVICE_STATE_POLL_INTERVAL,
    )
}

pub fn service_uninstall(spec: &LaunchAgentSpec) -> Result<ServiceUninstallOutcome, LaunchdError> {
    let uid = effective_user_id()?;
    let mut runner = SystemLaunchctlRunner;
    service_uninstall_and_wait_with_runner(spec, uid, &mut runner, SERVICE_STATE_POLL_INTERVAL)
}

pub fn service_status(spec: &LaunchAgentSpec) -> Result<ServiceStatus, LaunchdError> {
    let uid = effective_user_id()?;
    let mut runner = SystemLaunchctlRunner;
    service_status_with_runner(spec, uid, &mut runner)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchctlOperation {
    Bootstrap,
    Kickstart,
    Bootout,
    Print,
}

impl fmt::Display for LaunchctlOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Bootstrap => "bootstrap",
            Self::Kickstart => "kickstart",
            Self::Bootout => "bootout",
            Self::Print => "print",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Error)]
pub enum LaunchdError {
    #[error("launchd label must be 1-255 ASCII letters, digits, '.', '-', or '_'")]
    InvalidLabel,
    #[error("{field} path must be absolute")]
    RelativePath { field: &'static str },
    #[error("{field} path contains unsafe or unsupported characters/components")]
    UnsafePath { field: &'static str },
    #[error(
        "installed binary must be a materialized '{INSTALLED_BINARY_NAME}' outside a build cache"
    )]
    InvalidInstalledBinary,
    #[error("LaunchAgent plist filename does not match its service label")]
    LaunchAgentFilenameMismatch,
    #[error("LaunchAgent plist could not be read safely")]
    LaunchAgentRead { kind: io::ErrorKind },
    #[error("LaunchAgent plist must be a regular non-symlink file")]
    UnsafeLaunchAgentFileType,
    #[error("LaunchAgent plist permissions must be exactly 0644")]
    UnsafeLaunchAgentPermissions,
    #[error("LaunchAgent plist exceeds the admitted size bound")]
    LaunchAgentTooLarge,
    #[error("LaunchAgent plist bytes do not match the expected service definition")]
    LaunchAgentContentMismatch,
    #[error("launchd user service cannot run in the root/system domain")]
    RootDomainUnsupported,
    #[cfg(not(unix))]
    #[error("launchd user services require a Unix effective user ID")]
    UnsupportedPlatform,
    #[error("failed to execute launchctl {operation}")]
    LaunchctlIo {
        operation: LaunchctlOperation,
        #[source]
        source: io::Error,
    },
    #[error("launchctl {operation} failed with exit code {exit_code:?}")]
    LaunchctlFailed {
        operation: LaunchctlOperation,
        exit_code: Option<i32>,
    },
    #[error("launchd service did not reach {expected} state before the bounded deadline")]
    ServiceStateTimeout { expected: &'static str },
}

fn service_install_and_wait_with_runner<R: LaunchctlRunner>(
    spec: &LaunchAgentSpec,
    plist_path: &Path,
    uid: u32,
    runner: &mut R,
    poll_interval: Duration,
) -> Result<(), LaunchdError> {
    service_install_with_runner(spec, plist_path, uid, runner)?;
    wait_for_service_state_with_runner(
        spec,
        uid,
        runner,
        ServiceStatus::Loaded,
        "loaded",
        poll_interval,
    )
}

fn service_uninstall_and_wait_with_runner<R: LaunchctlRunner>(
    spec: &LaunchAgentSpec,
    uid: u32,
    runner: &mut R,
    poll_interval: Duration,
) -> Result<ServiceUninstallOutcome, LaunchdError> {
    let outcome = service_uninstall_with_runner(spec, uid, runner)?;
    if outcome == ServiceUninstallOutcome::Stopped {
        wait_for_service_state_with_runner(
            spec,
            uid,
            runner,
            ServiceStatus::NotLoaded,
            "not_loaded",
            poll_interval,
        )?;
    }
    Ok(outcome)
}

fn wait_for_service_state_with_runner<R: LaunchctlRunner>(
    spec: &LaunchAgentSpec,
    uid: u32,
    runner: &mut R,
    expected: ServiceStatus,
    expected_name: &'static str,
    poll_interval: Duration,
) -> Result<(), LaunchdError> {
    for attempt in 0..SERVICE_STATE_POLL_ATTEMPTS {
        match service_status_with_runner(spec, uid, runner)? {
            status if status == expected => return Ok(()),
            ServiceStatus::Failed { exit_code } => {
                return Err(LaunchdError::LaunchctlFailed {
                    operation: LaunchctlOperation::Print,
                    exit_code,
                });
            }
            _ if attempt + 1 < SERVICE_STATE_POLL_ATTEMPTS => thread::sleep(poll_interval),
            _ => break,
        }
    }
    Err(LaunchdError::ServiceStateTimeout {
        expected: expected_name,
    })
}

fn service_install_with_runner<R: LaunchctlRunner>(
    spec: &LaunchAgentSpec,
    plist_path: &Path,
    uid: u32,
    runner: &mut R,
) -> Result<(), LaunchdError> {
    validate_user_id(uid)?;
    validate_path("LaunchAgent plist", plist_path)?;
    admit_launch_agent(spec, plist_path)?;

    let bootstrap = LaunchctlCommand::bootstrap(uid, plist_path);
    let outcome = runner
        .run(&bootstrap)
        .map_err(|source| LaunchdError::LaunchctlIo {
            operation: LaunchctlOperation::Bootstrap,
            source,
        })?;
    ensure_success(LaunchctlOperation::Bootstrap, &outcome)?;

    let kickstart = LaunchctlCommand::kickstart(uid, spec.label());
    let outcome = runner
        .run(&kickstart)
        .map_err(|source| LaunchdError::LaunchctlIo {
            operation: LaunchctlOperation::Kickstart,
            source,
        })?;
    ensure_success(LaunchctlOperation::Kickstart, &outcome)
}

fn service_uninstall_with_runner<R: LaunchctlRunner>(
    spec: &LaunchAgentSpec,
    uid: u32,
    runner: &mut R,
) -> Result<ServiceUninstallOutcome, LaunchdError> {
    validate_user_id(uid)?;
    let command = LaunchctlCommand::bootout(uid, spec.label());
    let outcome = runner
        .run(&command)
        .map_err(|source| LaunchdError::LaunchctlIo {
            operation: LaunchctlOperation::Bootout,
            source,
        })?;

    if outcome.success {
        return Ok(ServiceUninstallOutcome::Stopped);
    }
    if is_authoritative_not_loaded(LaunchctlOperation::Bootout, &outcome, uid, spec.label()) {
        return Ok(ServiceUninstallOutcome::AlreadyStopped);
    }
    Err(LaunchdError::LaunchctlFailed {
        operation: LaunchctlOperation::Bootout,
        exit_code: outcome.exit_code,
    })
}

fn service_status_with_runner<R: LaunchctlRunner>(
    spec: &LaunchAgentSpec,
    uid: u32,
    runner: &mut R,
) -> Result<ServiceStatus, LaunchdError> {
    validate_user_id(uid)?;
    let command = LaunchctlCommand::print(uid, spec.label());
    let outcome = runner
        .run(&command)
        .map_err(|source| LaunchdError::LaunchctlIo {
            operation: LaunchctlOperation::Print,
            source,
        })?;

    if outcome.success {
        Ok(ServiceStatus::Loaded)
    } else if is_authoritative_not_loaded(LaunchctlOperation::Print, &outcome, uid, spec.label()) {
        Ok(ServiceStatus::NotLoaded)
    } else {
        Ok(ServiceStatus::Failed {
            exit_code: outcome.exit_code,
        })
    }
}

fn ensure_success(
    operation: LaunchctlOperation,
    outcome: &LaunchctlOutcome,
) -> Result<(), LaunchdError> {
    if outcome.success {
        Ok(())
    } else {
        Err(LaunchdError::LaunchctlFailed {
            operation,
            exit_code: outcome.exit_code,
        })
    }
}

fn validate_label(label: &str) -> Result<(), LaunchdError> {
    if label.is_empty()
        || label.len() > 255
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(LaunchdError::InvalidLabel);
    }
    Ok(())
}

fn validate_path(field: &'static str, path: &Path) -> Result<(), LaunchdError> {
    if !path.is_absolute() {
        return Err(LaunchdError::RelativePath { field });
    }

    let text = path.to_str().ok_or(LaunchdError::UnsafePath { field })?;
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err(LaunchdError::UnsafePath { field });
    }

    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || path.file_name().is_none()
    {
        return Err(LaunchdError::UnsafePath { field });
    }
    Ok(())
}

fn validate_installed_binary(path: &Path) -> Result<(), LaunchdError> {
    if path.file_name() != Some(OsStr::new(INSTALLED_BINARY_NAME))
        || path.components().any(|component| {
            matches!(
                component,
                Component::Normal(value) if value == OsStr::new("target") || value == OsStr::new(".build")
            )
        })
    {
        return Err(LaunchdError::InvalidInstalledBinary);
    }
    Ok(())
}

fn admit_launch_agent(spec: &LaunchAgentSpec, plist_path: &Path) -> Result<(), LaunchdError> {
    let expected_filename = format!("{}.plist", spec.label());
    if plist_path.file_name() != Some(OsStr::new(&expected_filename)) {
        return Err(LaunchdError::LaunchAgentFilenameMismatch);
    }

    let file = open_launch_agent(plist_path)?;
    let metadata = file.metadata().map_err(redacted_launch_agent_read)?;
    if !metadata.file_type().is_file() {
        return Err(LaunchdError::UnsafeLaunchAgentFileType);
    }
    validate_launch_agent_permissions(&metadata)?;
    if metadata.len() > MAX_LAUNCH_AGENT_BYTES {
        return Err(LaunchdError::LaunchAgentTooLarge);
    }

    let mut actual = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_LAUNCH_AGENT_BYTES + 1)
        .read_to_end(&mut actual)
        .map_err(redacted_launch_agent_read)?;
    if actual.len() as u64 > MAX_LAUNCH_AGENT_BYTES {
        return Err(LaunchdError::LaunchAgentTooLarge);
    }
    if actual != spec.render_plist() {
        return Err(LaunchdError::LaunchAgentContentMismatch);
    }
    Ok(())
}

#[cfg(unix)]
fn open_launch_agent(path: &Path) -> Result<fs::File, LaunchdError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(redacted_launch_agent_read)
}

#[cfg(not(unix))]
fn open_launch_agent(_path: &Path) -> Result<fs::File, LaunchdError> {
    Err(LaunchdError::UnsupportedPlatform)
}

#[cfg(unix)]
fn validate_launch_agent_permissions(metadata: &fs::Metadata) -> Result<(), LaunchdError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o7777 != LAUNCH_AGENT_MODE {
        return Err(LaunchdError::UnsafeLaunchAgentPermissions);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_launch_agent_permissions(_metadata: &fs::Metadata) -> Result<(), LaunchdError> {
    Err(LaunchdError::UnsupportedPlatform)
}

fn redacted_launch_agent_read(error: io::Error) -> LaunchdError {
    LaunchdError::LaunchAgentRead { kind: error.kind() }
}

fn validate_user_id(uid: u32) -> Result<(), LaunchdError> {
    if uid == 0 {
        Err(LaunchdError::RootDomainUnsupported)
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn effective_user_id() -> Result<u32, LaunchdError> {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    let uid = unsafe { libc::geteuid() };
    validate_user_id(uid)?;
    Ok(uid)
}

#[cfg(not(unix))]
fn effective_user_id() -> Result<u32, LaunchdError> {
    Err(LaunchdError::UnsupportedPlatform)
}

fn path_text(path: &Path) -> &str {
    path.to_str()
        .expect("LaunchAgentSpec validates every path as UTF-8")
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LaunchctlCommand {
    operation: LaunchctlOperation,
    arguments: Vec<OsString>,
}

impl LaunchctlCommand {
    fn bootstrap(uid: u32, plist_path: &Path) -> Self {
        Self {
            operation: LaunchctlOperation::Bootstrap,
            arguments: vec![
                OsString::from("bootstrap"),
                OsString::from(format!("gui/{uid}")),
                plist_path.as_os_str().to_owned(),
            ],
        }
    }

    fn kickstart(uid: u32, label: &str) -> Self {
        Self {
            operation: LaunchctlOperation::Kickstart,
            arguments: vec![
                OsString::from("kickstart"),
                OsString::from("-k"),
                OsString::from(service_target(uid, label)),
            ],
        }
    }

    fn bootout(uid: u32, label: &str) -> Self {
        Self {
            operation: LaunchctlOperation::Bootout,
            arguments: vec![
                OsString::from("bootout"),
                OsString::from(service_target(uid, label)),
            ],
        }
    }

    fn print(uid: u32, label: &str) -> Self {
        Self {
            operation: LaunchctlOperation::Print,
            arguments: vec![
                OsString::from("print"),
                OsString::from(service_target(uid, label)),
            ],
        }
    }
}

fn service_target(uid: u32, label: &str) -> String {
    format!("gui/{uid}/{label}")
}

trait LaunchctlRunner {
    fn run(&mut self, command: &LaunchctlCommand) -> io::Result<LaunchctlOutcome>;
}

struct SystemLaunchctlRunner;

impl LaunchctlRunner for SystemLaunchctlRunner {
    fn run(&mut self, command: &LaunchctlCommand) -> io::Result<LaunchctlOutcome> {
        let output = Command::new(LAUNCHCTL_PATH)
            .args(&command.arguments)
            .stdin(Stdio::null())
            .output()?;
        Ok(LaunchctlOutcome {
            success: output.status.success(),
            exit_code: output.status.code(),
            stderr: output.stderr,
        })
    }
}

#[derive(Clone, Debug)]
struct LaunchctlOutcome {
    success: bool,
    exit_code: Option<i32>,
    stderr: Vec<u8>,
}

fn is_authoritative_not_loaded(
    operation: LaunchctlOperation,
    outcome: &LaunchctlOutcome,
    uid: u32,
    label: &str,
) -> bool {
    if outcome.success {
        return false;
    }

    let Ok(stderr) = std::str::from_utf8(&outcome.stderr) else {
        return false;
    };
    let exact_missing_service =
        format!("Could not find service \"{label}\" in domain for user gui: {uid}");

    stderr.lines().any(|line| {
        let line = line.trim();
        line == exact_missing_service
            || (operation == LaunchctlOperation::Bootout
                && line == "Boot-out failed: 3: No such process")
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    const UID: u32 = 501;

    fn spec() -> LaunchAgentSpec {
        LaunchAgentSpec::recommended(
            "/private/tmp/grok-codex-bridge-test/bin/grok-codex-bridge",
            "/private/tmp/grok-codex-bridge-test/bridge.toml",
            "/private/tmp/grok-codex-bridge-test/logs/stdout.log",
            "/private/tmp/grok-codex-bridge-test/logs/stderr.log",
        )
        .unwrap()
    }

    fn success() -> LaunchctlOutcome {
        LaunchctlOutcome {
            success: true,
            exit_code: Some(0),
            stderr: Vec::new(),
        }
    }

    fn failure(exit_code: i32, stderr: &str) -> LaunchctlOutcome {
        LaunchctlOutcome {
            success: false,
            exit_code: Some(exit_code),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn write_plist(directory: &Path, filename: &str, contents: &[u8], mode: u32) -> PathBuf {
        let path = directory.join(filename);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(path.is_absolute());
        path
    }

    fn write_admitted_plist(spec: &LaunchAgentSpec, directory: &Path) -> PathBuf {
        write_plist(
            directory,
            &format!("{}.plist", spec.label()),
            &spec.render_plist(),
            LAUNCH_AGENT_MODE,
        )
    }

    #[test]
    fn plist_is_deterministic_escaped_and_uses_only_the_direct_binary() {
        let spec = LaunchAgentSpec::new(
            "com.local.grok-codex-bridge",
            "/private/tmp/A&B/Bridge/grok-codex-bridge",
            "/private/tmp/A&B/Bridge/config<safe>.toml",
            "/private/tmp/A&B/Logs/stdout\"one.log",
            "/private/tmp/A&B/Logs/stderr'one.log",
        )
        .unwrap();

        let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>Label</key>\n\
  <string>com.local.grok-codex-bridge</string>\n\
  <key>ProgramArguments</key>\n\
  <array>\n\
    <string>/private/tmp/A&amp;B/Bridge/grok-codex-bridge</string>\n\
    <string>run</string>\n\
    <string>--config</string>\n\
    <string>/private/tmp/A&amp;B/Bridge/config&lt;safe&gt;.toml</string>\n\
  </array>\n\
  <key>RunAtLoad</key>\n\
  <true/>\n\
  <key>KeepAlive</key>\n\
  <true/>\n\
  <key>ProcessType</key>\n\
  <string>Background</string>\n\
  <key>StandardOutPath</key>\n\
  <string>/private/tmp/A&amp;B/Logs/stdout&quot;one.log</string>\n\
  <key>StandardErrorPath</key>\n\
  <string>/private/tmp/A&amp;B/Logs/stderr&apos;one.log</string>\n\
  <key>EnvironmentVariables</key>\n\
  <dict>\n\
    <key>RUST_LOG</key>\n\
    <string>info</string>\n\
  </dict>\n\
</dict>\n\
</plist>\n";

        assert_eq!(spec.render_plist(), expected.as_bytes());
        let plist = String::from_utf8(spec.render_plist()).unwrap();
        assert!(!plist.contains("cargo"));
        assert!(!plist.contains("capability"));
        assert!(!plist.contains("token"));
        assert!(!plist.contains("credential"));
    }

    #[test]
    fn unsafe_labels_and_paths_are_rejected() {
        assert!(matches!(
            LaunchAgentSpec::new(
                "com.local/bad",
                "/opt/grok-codex-bridge",
                "/opt/bridge.toml",
                "/opt/stdout.log",
                "/opt/stderr.log"
            ),
            Err(LaunchdError::InvalidLabel)
        ));
        assert!(matches!(
            LaunchAgentSpec::recommended(
                "relative/grok-codex-bridge",
                "/opt/bridge.toml",
                "/opt/stdout.log",
                "/opt/stderr.log"
            ),
            Err(LaunchdError::RelativePath {
                field: "installed binary"
            })
        ));
        assert!(matches!(
            LaunchAgentSpec::recommended(
                "/repo/target/release/grok-codex-bridge",
                "/opt/bridge.toml",
                "/opt/stdout.log",
                "/opt/stderr.log"
            ),
            Err(LaunchdError::InvalidInstalledBinary)
        ));
        assert!(matches!(
            LaunchAgentSpec::recommended(
                "/opt/grok-codex-bridge",
                "/opt/bridge\n.toml",
                "/opt/stdout.log",
                "/opt/stderr.log"
            ),
            Err(LaunchdError::UnsafePath {
                field: "bridge config"
            })
        ));
        assert!(matches!(
            LaunchAgentSpec::recommended(
                "/bin/bash",
                "/opt/bridge.toml",
                "/opt/stdout.log",
                "/opt/stderr.log"
            ),
            Err(LaunchdError::InvalidInstalledBinary)
        ));
    }

    #[test]
    fn install_bootstraps_then_kickstarts_the_user_service() {
        let spec = spec();
        let temporary = tempfile::tempdir().unwrap();
        let plist_path = write_admitted_plist(&spec, temporary.path());
        let plist_text = plist_path.to_str().unwrap();
        let mut runner = FakeRunner::new([success(), success()]);
        service_install_with_runner(&spec, &plist_path, UID, &mut runner).unwrap();

        assert_eq!(
            runner.commands,
            vec![
                LaunchctlCommand {
                    operation: LaunchctlOperation::Bootstrap,
                    arguments: strings(&["bootstrap", "gui/501", plist_text]),
                },
                LaunchctlCommand {
                    operation: LaunchctlOperation::Kickstart,
                    arguments: strings(
                        &["kickstart", "-k", "gui/501/com.local.grok-codex-bridge",]
                    ),
                },
            ]
        );
    }

    #[test]
    fn service_lifecycle_waits_for_launchd_state_convergence() {
        let spec = spec();
        let temporary = tempfile::tempdir().unwrap();
        let plist_path = write_admitted_plist(&spec, temporary.path());
        let exact = format!(
            "Could not find service \"{}\" in domain for user gui: {UID}",
            RECOMMENDED_LAUNCH_AGENT_LABEL
        );

        let mut install_runner =
            FakeRunner::new([success(), success(), failure(113, &exact), success()]);
        service_install_and_wait_with_runner(
            &spec,
            &plist_path,
            UID,
            &mut install_runner,
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(
            install_runner
                .commands
                .iter()
                .map(|command| command.operation)
                .collect::<Vec<_>>(),
            vec![
                LaunchctlOperation::Bootstrap,
                LaunchctlOperation::Kickstart,
                LaunchctlOperation::Print,
                LaunchctlOperation::Print,
            ]
        );

        let mut uninstall_runner = FakeRunner::new([success(), success(), failure(113, &exact)]);
        assert_eq!(
            service_uninstall_and_wait_with_runner(
                &spec,
                UID,
                &mut uninstall_runner,
                Duration::ZERO,
            )
            .unwrap(),
            ServiceUninstallOutcome::Stopped
        );
        assert_eq!(
            uninstall_runner
                .commands
                .iter()
                .map(|command| command.operation)
                .collect::<Vec<_>>(),
            vec![
                LaunchctlOperation::Bootout,
                LaunchctlOperation::Print,
                LaunchctlOperation::Print,
            ]
        );
    }

    #[test]
    fn bootstrap_failure_stops_before_kickstart() {
        let spec = spec();
        let temporary = tempfile::tempdir().unwrap();
        let plist_path = write_admitted_plist(&spec, temporary.path());
        let mut runner = FakeRunner::new([failure(5, "private launchctl details")]);
        let error = service_install_with_runner(&spec, &plist_path, UID, &mut runner).unwrap_err();

        assert!(matches!(
            error,
            LaunchdError::LaunchctlFailed {
                operation: LaunchctlOperation::Bootstrap,
                exit_code: Some(5)
            }
        ));
        assert_eq!(runner.commands.len(), 1);
        assert!(!error.to_string().contains("private launchctl details"));
    }

    #[test]
    fn install_rejects_unadmitted_plists_before_launchctl() {
        let spec = spec();

        let missing_dir = tempfile::tempdir().unwrap();
        let missing = missing_dir.path().join(format!("{}.plist", spec.label()));
        let mut missing_runner = FakeRunner::new([]);
        let error =
            service_install_with_runner(&spec, &missing, UID, &mut missing_runner).unwrap_err();
        assert!(matches!(error, LaunchdError::LaunchAgentRead { .. }));
        assert!(missing_runner.commands.is_empty());

        let symlink_dir = tempfile::tempdir().unwrap();
        let target = write_plist(
            symlink_dir.path(),
            "target.plist",
            &spec.render_plist(),
            LAUNCH_AGENT_MODE,
        );
        let symlink_path = symlink_dir.path().join(format!("{}.plist", spec.label()));
        symlink(&target, &symlink_path).unwrap();
        let mut symlink_runner = FakeRunner::new([]);
        let error = service_install_with_runner(&spec, &symlink_path, UID, &mut symlink_runner)
            .unwrap_err();
        assert!(matches!(error, LaunchdError::LaunchAgentRead { .. }));
        assert!(symlink_runner.commands.is_empty());

        let mode_dir = tempfile::tempdir().unwrap();
        let wrong_mode = write_plist(
            mode_dir.path(),
            &format!("{}.plist", spec.label()),
            &spec.render_plist(),
            0o600,
        );
        let mut mode_runner = FakeRunner::new([]);
        let error =
            service_install_with_runner(&spec, &wrong_mode, UID, &mut mode_runner).unwrap_err();
        assert!(matches!(error, LaunchdError::UnsafeLaunchAgentPermissions));
        assert!(mode_runner.commands.is_empty());

        let tampered_dir = tempfile::tempdir().unwrap();
        let tampered = write_plist(
            tampered_dir.path(),
            &format!("{}.plist", spec.label()),
            b"<plist>tampered</plist>\n",
            LAUNCH_AGENT_MODE,
        );
        let mut tampered_runner = FakeRunner::new([]);
        let error =
            service_install_with_runner(&spec, &tampered, UID, &mut tampered_runner).unwrap_err();
        assert!(matches!(error, LaunchdError::LaunchAgentContentMismatch));
        assert!(tampered_runner.commands.is_empty());

        let filename_dir = tempfile::tempdir().unwrap();
        let wrong_filename = write_plist(
            filename_dir.path(),
            "different-label.plist",
            &spec.render_plist(),
            LAUNCH_AGENT_MODE,
        );
        let mut filename_runner = FakeRunner::new([]);
        let error = service_install_with_runner(&spec, &wrong_filename, UID, &mut filename_runner)
            .unwrap_err();
        assert!(matches!(error, LaunchdError::LaunchAgentFilenameMismatch));
        assert!(filename_runner.commands.is_empty());
    }

    #[test]
    fn bootout_only_accepts_an_authoritative_not_loaded_diagnostic() {
        let exact = format!(
            "Could not find service \"{}\" in domain for user gui: {UID}\n",
            RECOMMENDED_LAUNCH_AGENT_LABEL
        );
        let mut absent = FakeRunner::new([failure(113, &exact)]);
        assert_eq!(
            service_uninstall_with_runner(&spec(), UID, &mut absent).unwrap(),
            ServiceUninstallOutcome::AlreadyStopped
        );
        assert_eq!(
            absent.commands[0].arguments,
            strings(&["bootout", "gui/501/com.local.grok-codex-bridge"])
        );

        let mut failed = FakeRunner::new([failure(3, "permission denied")]);
        let error = service_uninstall_with_runner(&spec(), UID, &mut failed).unwrap_err();
        assert!(matches!(
            error,
            LaunchdError::LaunchctlFailed {
                operation: LaunchctlOperation::Bootout,
                exit_code: Some(3)
            }
        ));
    }

    #[test]
    fn status_reports_loaded_not_loaded_or_failed_without_output() {
        let exact = format!(
            "Could not find service \"{}\" in domain for user gui: {UID}",
            RECOMMENDED_LAUNCH_AGENT_LABEL
        );
        let mut loaded = FakeRunner::new([success()]);
        assert_eq!(
            service_status_with_runner(&spec(), UID, &mut loaded).unwrap(),
            ServiceStatus::Loaded
        );

        let mut absent = FakeRunner::new([failure(113, &exact)]);
        assert_eq!(
            service_status_with_runner(&spec(), UID, &mut absent).unwrap(),
            ServiceStatus::NotLoaded
        );

        let mut failed = FakeRunner::new([failure(78, "sensitive diagnostic body")]);
        assert_eq!(
            service_status_with_runner(&spec(), UID, &mut failed).unwrap(),
            ServiceStatus::Failed {
                exit_code: Some(78)
            }
        );
        assert_eq!(
            failed.commands[0].arguments,
            strings(&["print", "gui/501/com.local.grok-codex-bridge"])
        );
    }

    #[test]
    fn root_domain_and_relative_plist_are_rejected_before_launchctl() {
        let mut root_runner = FakeRunner::new([]);
        let error = service_status_with_runner(&spec(), 0, &mut root_runner).unwrap_err();
        assert!(matches!(error, LaunchdError::RootDomainUnsupported));
        assert!(root_runner.commands.is_empty());

        let mut relative_runner = FakeRunner::new([]);
        let error = service_install_with_runner(
            &spec(),
            Path::new("relative.plist"),
            UID,
            &mut relative_runner,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LaunchdError::RelativePath {
                field: "LaunchAgent plist"
            }
        ));
        assert!(relative_runner.commands.is_empty());
    }

    fn strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    struct FakeRunner {
        outcomes: VecDeque<LaunchctlOutcome>,
        commands: Vec<LaunchctlCommand>,
    }

    impl FakeRunner {
        fn new(outcomes: impl IntoIterator<Item = LaunchctlOutcome>) -> Self {
            Self {
                outcomes: outcomes.into_iter().collect(),
                commands: Vec::new(),
            }
        }
    }

    impl LaunchctlRunner for FakeRunner {
        fn run(&mut self, command: &LaunchctlCommand) -> io::Result<LaunchctlOutcome> {
            self.commands.push(command.clone());
            Ok(self.outcomes.pop_front().expect("unexpected command"))
        }
    }
}
