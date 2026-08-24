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
fn desktop_switch_exposes_native_compatibility_without_an_uninstall_alias() {
    let output = binary()
        .args(["switch", "--help"])
        .output()
        .expect("the test binary must start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("switch help must be UTF-8");
    assert!(stdout.contains("--native-compatibility"));
    assert!(stdout.contains("--replacement-script"));
    assert!(stdout.contains("--replacement-launcher"));
    assert!(!stdout.contains("uninstall"));
}

#[test]
fn installed_mode_commands_are_explicit_and_do_not_expose_build_options() {
    for mode in ["grok", "native"] {
        let output = binary()
            .args(["mode", mode, "--help"])
            .output()
            .expect("the test binary must start");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("mode help must be UTF-8");
        assert!(!stdout.contains("cargo"));
        assert!(!stdout.contains("materialize"));
        assert!(!stdout.contains("replacement"));
    }
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
    let source_launcher = temporary.path().join("Grok Codex Switch.app");
    let launcher_executable = source_launcher.join("Contents/MacOS/Grok Codex Switch");
    let launch_parent = home.join("Library/LaunchAgents");
    let launch_agent = launch_parent.join("com.local.grok-codex-bridge.plist");
    fs::create_dir_all(&codex_home).expect("the temporary Codex home must be created");
    fs::create_dir_all(&install_parent).expect("the temporary install parent must be created");
    fs::create_dir_all(&launch_parent).expect("the temporary LaunchAgents parent must be created");
    fs::create_dir_all(launcher_executable.parent().unwrap())
        .expect("the launcher bundle must be created");
    fs::create_dir_all(source_launcher.join("Contents/Resources"))
        .expect("the launcher resources must be created");
    fs::write(
        source_launcher.join("Contents/Info.plist"),
        "<plist><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleExecutable</key><string>Grok Codex Switch</string></plist>",
    )
    .expect("the launcher Info.plist must be written");
    fs::write(&launcher_executable, b"fake launcher executable")
        .expect("the launcher executable must be written");
    fs::write(
        source_launcher.join("Contents/Resources/grok-codex-bridge-overlay.md"),
        b"Grok overlay",
    )
    .expect("the launcher overlay must be written");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher_executable, fs::Permissions::from_mode(0o755))
            .expect("the launcher executable must be executable");
    }

    let output = binary()
        .args([
            "install",
            "--source-binary",
            env!("CARGO_BIN_EXE_grok-codex-bridge"),
            "--source-launcher",
        ])
        .arg(&source_launcher)
        .args(["--install-root"])
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
    assert!(
        install_root
            .join("bin/Grok Codex Switch.app/Contents/MacOS/Grok Codex Switch")
            .is_file()
    );
    assert!(
        install_root
            .join("bin/Grok Codex Switch.app/Contents/Resources/grok-codex-bridge-overlay.md")
            .is_file()
    );
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
