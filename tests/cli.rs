use std::process::Command;
use std::{fs, path::Path};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_grok-codex-bridge"))
}

#[test]
fn version_reports_package_version() {
    let output = binary()
        .arg("--version")
        .output()
        .expect("the test binary must start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version output must be UTF-8"),
        format!("grok-codex-bridge {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn status_reports_capability_without_claiming_activation() {
    let output = binary()
        .arg("status")
        .output()
        .expect("the test binary must start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("status output must be UTF-8"),
        "phase F source includes local Responses, reversible lifecycle, doctor, auth status/ensure, and launchd controls; this command does not inspect installation or activation\n"
    );
}

#[test]
fn unsupported_commands_fail_closed() {
    let output = binary()
        .arg("serve")
        .output()
        .expect("the test binary must start");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .expect("error output must be UTF-8")
            .contains("unrecognized subcommand 'serve'")
    );
}

#[test]
fn run_requires_an_explicit_config_path() {
    let output = binary()
        .arg("run")
        .output()
        .expect("the test binary must start");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .expect("error output must be UTF-8")
            .contains("--config <FILE>")
    );
}

#[test]
fn catalog_refresh_requires_an_explicit_config_path() {
    let output = binary()
        .args(["catalog", "refresh"])
        .output()
        .expect("the test binary must start");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .expect("error output must be UTF-8")
            .contains("--config <FILE>")
    );
}

#[test]
fn lifecycle_subcommands_fail_closed_when_the_operation_is_missing() {
    for command in ["auth", "service"] {
        let output = binary()
            .arg(command)
            .output()
            .expect("the test binary must start");

        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8(output.stderr).expect("error output must be UTF-8");
        assert!(stderr.contains(&format!("Usage: grok-codex-bridge {command} <COMMAND>")));
    }
}

#[test]
fn auth_ensure_is_exposed_as_an_explicit_operation() {
    let output = binary()
        .args(["auth", "ensure", "--help"])
        .output()
        .expect("the test binary must start");

    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .expect("auth ensure help must be UTF-8")
            .contains("official browser OAuth flow")
    );
}

#[test]
fn install_rejects_unknown_options() {
    let output = binary()
        .args(["install", "--activate"])
        .output()
        .expect("the test binary must start");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .expect("error output must be UTF-8")
            .contains("unexpected argument '--activate'")
    );
}

#[test]
fn install_materializes_without_activation_or_secret_output() {
    let temporary = tempfile::tempdir().expect("a temporary install root must be available");
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let install_parent = home.join("Library/Application Support");
    let install_root = install_parent.join("grok-codex-bridge");
    let launch_parent = home.join("Library/LaunchAgents");
    let launch_agent = launch_parent.join("com.local.grok-codex-bridge.plist");
    fs::create_dir_all(&codex_home).expect("the temporary Codex home must be created");
    fs::create_dir_all(&install_parent).expect("the temporary install parent must be created");
    fs::create_dir_all(&launch_parent).expect("the temporary LaunchAgents parent must be created");

    let output = binary()
        .args([
            "install",
            "--source-binary",
            env!("CARGO_BIN_EXE_grok-codex-bridge"),
            "--install-root",
        ])
        .arg(&install_root)
        .args(["--codex-home"])
        .arg(&codex_home)
        .args(["--launch-agent"])
        .arg(&launch_agent)
        .output()
        .expect("the test binary must start");

    let stdout = String::from_utf8(output.stdout).expect("install output must be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("install errors must be UTF-8");
    assert!(output.status.success(), "install failed: {stderr}");
    assert!(stdout.contains("install: complete"));
    assert!(stdout.contains("next: grok-codex-bridge service install"));
    assert!(stdout.contains("next: codex --profile grok-bridge"));
    assert!(stderr.is_empty());

    let capability = fs::read_to_string(install_root.join("secrets/caller-capability"))
        .expect("the caller capability must be materialized");
    assert!(!stdout.contains(&capability));
    assert!(!stderr.contains(&capability));
    assert_eq!(capability.len(), 64);
    assert_materialized(&install_root.join("bin/grok-codex-bridge"));
    assert!(install_root.join("config/bridge.toml").is_file());
    assert!(install_root.join("install-manifest.json").is_file());
    assert!(codex_home.join("grok-bridge.config.toml").is_file());
    assert!(launch_agent.is_file());
}

fn assert_materialized(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path).expect("installed binary must be inspectable");
    assert!(metadata.file_type().is_file());
    assert!(!metadata.file_type().is_symlink());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
}
