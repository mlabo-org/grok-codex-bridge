# Distribution Contract

Status: design record only. This document does not start V1.1, publish a release, or change the installed runtime.

## Purpose

The repository launcher `scripts/grok-codex.sh` is a source-owner convenience. It resolves the repository from its own location, verifies the materialized binary against the checked-out Rust source, and therefore cannot be copied or symlinked into an unrelated `PATH` directory by itself.

The distributable product must remove that repository dependency. A user installs once, then starts the Grok-backed Codex CLI with:

```sh
grok-codex
```

The ordinary `codex` command must continue to start the native GPT configuration unchanged.

## V1.0 And V1.1 Product Relationship

V1.0 and V1.1 are successive modes of one product, not separate repositories, services, protocol implementations, or independently distributed bridge products.

- V1.0 is the explicit Safe Provider entry: the repository launcher, and later the installed `grok-codex` command, start Codex CLI with the isolated `grok-bridge` profile. It does not integrate Grok into the native picker.
- V1.1 adds Native Picker Mode on top of the same bridge, lifecycle, credential boundary, catalog, and installed service.
- V1.1 targets both user-facing Codex selection surfaces: the CLI model picker and the Codex Desktop GUI model picker. CLI-only picker success is not completion of the V1.1 product goal.
- Before implementation, V1.1 R0 must verify whether the current CLI and Desktop builds consume the same model/provider catalog and configuration path. If they differ, each surface needs its own source-owned integration boundary and primary-path evidence; one surface must not be assumed to prove the other.
- Native GPT remains the default and must stay selectable. Grok entries are additive. GPT and Grok traffic separation, byte-preserving GPT behavior where passthrough is required, and complete rollback remain V1.1 acceptance conditions.

The installed `grok-codex` command remains the explicit V1.0 entry while that mode is an active supported route. It must share the installed service and implementation with Native Picker Mode rather than becoming a second bridge or compatibility fork.

### Picker feasibility reference

[duolahypercho/codex-router at the pinned reference commit](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174) is a concrete feasibility reference for selecting non-native models through the Codex Desktop model picker. V1.1 R0 must identify and verify the exact current catalog/configuration producer that makes those entries visible in both Desktop and CLI before this bridge edits Codex-owned state.

Only that picker integration mechanism and its rollback implications are candidates for transfer. codex-router's LiteLLM/Chat multi-hop, namespace conversion, hosted-tool injection, and fixed Grok registry remain outside this direct Responses-to-Responses bridge.

## Fixed Runtime Decisions

- Normal operation uses prebuilt native executables. It never invokes Cargo, rustc, a source checkout, `target/`, build-on-first-use, or an interpreted fallback.
- The release package contains two Rust binaries built from this repository:
  - `grok-codex-bridge`: provider service and lifecycle owner;
  - `grok-codex`: thin native launch coordinator.
- `grok-codex` owns only installed-runtime validation, typed service-status/start sequencing, and `exec` of `codex --profile grok-bridge` with the user's remaining arguments. It does not own protocol translation, credentials, model routing, or tool execution.
- `grok-codex-bridge` remains the only owner of installation, doctor, auth status, catalog refresh, provider service, LaunchAgent rendering, and uninstall.
- The release initially supports only `aarch64-apple-darwin`. Another platform or architecture requires its own declared artifact and acceptance evidence; it must not reuse an incompatible binary.

## Installed Layout

The per-user installer derives every path from the current user's home directory and records every owned path in the existing private install manifest. No personal absolute path may be compiled into or shipped with an artifact.

```text
~/Library/Application Support/grok-codex-bridge/
  bin/grok-codex-bridge
  bin/grok-codex
  config.toml
  caller-token
  catalog.json
  install-manifest.json

~/.local/bin/grok-codex
  -> installed native grok-codex executable

~/.codex/grok-bridge.config.toml
~/Library/LaunchAgents/com.local.grok-codex-bridge.plist
```

The command entry may be an exact installer-owned symlink to the installed native launcher or an exact copy installed atomically. The manifest must record which form was created. The installer must not edit shell startup files without separate explicit authority. If the selected bin directory is absent from `PATH`, installation fails with one exact corrective instruction or accepts an explicit absolute `--command-path` override.

## Responsibility Model

### Release producer

- Builds both binaries once per declared target using the pinned toolchain.
- Places them outside Cargo build caches in one versioned release archive.
- Publishes checksums and, when a publication decision authorizes it, the selected macOS signing and notarization evidence.
- Never packages credentials, capability tokens, Codex state, Grok state, logs, or user-specific paths.

### Native setup coordinator

- Runs only from an already downloaded prebuilt artifact.
- Verifies platform, architecture, executable bytes, destination safety, and prerequisites before mutation.
- Calls the existing lifecycle boundaries in order: install files/profile, validate with doctor, install the user LaunchAgent, then expose the `grok-codex` command.
- Rolls back only files created by the failed setup attempt. It does not remove or overwrite unrelated Codex configuration.

### Installed launcher

- Resolves the bridge only from the install manifest and declared install root, never from the repository or Cargo cache.
- Does not reinstall on every invocation.
- Starts the service only when typed status says it is not loaded.
- Executes the current `codex` found through the supported executable-resolution contract with the isolated `grok-bridge` profile.
- Passes user arguments through without interpreting model prompts or tool requests.

### LaunchAgent service

- Runs in the current user's launchd domain with `RunAtLoad` and `KeepAlive`.
- Survives Codex CLI exit, stops at logout/shutdown, and starts again at the next login while installed.
- Listens only on loopback and reads the existing private runtime configuration.

### External owners

- Codex owns its agent loop, permissions, tools, sessions, and native GPT behavior.
- The official Grok client flow owns login and credential renewal. Distribution checks availability read-only and never copies or refreshes credentials itself.

## User Flow

One-time setup from an extracted, verified release artifact:

```sh
./grok-codex-bridge setup
```

`setup` is a future native coordinator command. It may sequence existing lifecycle operations but must not merge their underlying ownership or weaken rollback.

Normal use after setup:

```sh
grok-codex
grok-codex --version
codex
```

The first two commands use the isolated Grok provider profile. The last command remains the user's ordinary GPT route. None of them compiles source.

An update installs a newly verified pair of binaries atomically, rewrites only bridge-owned LaunchAgent/runtime entries, restarts the user service when required, and preserves the private config, capability, catalog, credential source, and base Codex configuration. Uninstall removes the command entry and other manifest-proven bridge files and restores exact recorded backups.

## Distribution Channels

The canonical artifact is a versioned release archive containing the two prebuilt binaries and integrity metadata. A future GitHub Release or Homebrew Tap may transport that same artifact after publication, license, signing, and repository decisions are explicitly authorized. A package manager must not introduce a second implementation, runtime compilation, or a different lifecycle contract.

## Release Acceptance

One semantic acceptance bundle for the distribution implementation must prove all of the following on the declared target:

1. Both executables are produced by the source-owned materialization route and placed outside `target/`.
2. Setup succeeds from an extracted release artifact when the source repository and Rust toolchain are unavailable.
3. `grok-codex --version` and one representative `grok-codex` launch resolve only installed files, never the source checkout.
4. The user LaunchAgent reaches loaded state and the provider remains available after the launched Codex process exits.
5. Plain `codex` retains the pre-install native GPT configuration.
6. Uninstall removes only manifest-proven bridge artifacts and restores exact backups.
7. Active runtime callers contain no `cargo run`, on-demand build, Cargo-cache search, repository-relative lookup, or user-specific absolute path.

## Explicitly Deferred

- V1.1 native model picker, merged catalog, GPT passthrough, and aliases;
- GitHub publication, Homebrew publication, license selection, signing identity, and notarization execution;
- Intel macOS, Linux, and Windows artifacts;
- automatic modification of shell profiles;
- any credential migration, copying, refresh-token exchange, or login automation.
