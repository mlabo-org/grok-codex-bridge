# grok-codex-bridge

[日本語](README.ja.md)

**A native Rust Responses-to-Responses bridge that lets Grok run inside the Codex harness without replacing Native GPT.**

`grok-codex-bridge` is a standalone, loopback-only provider bridge for macOS on Apple Silicon. Codex continues to own the agent loop, tools, permissions, MCP servers, Skills, and session state. This project owns only the local provider boundary, strict Responses protocol translation, validated SSE streaming, read-only Grok authentication, and the upstream connection to xAI.

It is not a Codex plugin, a general-purpose LLM router, or an agent harness.

## Install with Codex

The repository includes [AGENTS.md](AGENTS.md) as a binding safety and lifecycle contract for coding agents. To let Codex inspect the source, build the native executable, and install the conservative V1.0 isolated profile, clone the repository and start Codex from its root:

```sh
git clone https://github.com/mlabo-org/grok-codex-bridge.git
cd grok-codex-bridge
codex
```

Then choose one of the following requests.

### Install the isolated V1.0 profile

```text
Read AGENTS.md completely and follow it. Build and install the isolated V1.0
grok-bridge profile on this Mac. Verify the platform and prerequisites, preserve
existing changes, use ./scripts/materialize-macos.sh and the repository-owned
lifecycle commands, and run only the minimum primary-path checks. Do not enable
the experimental V1.1 merged picker, edit the Codex binary, Codex configuration,
Grok authentication, or LaunchAgent files directly, or commit, push, or publish.
Stop and explain the missing boundary if this is not Apple Silicon macOS or a
required authoritative input cannot be verified.
```

### Install through the experimental V1.1 merged picker

Use this request when you want Codex to complete the build, isolated installation, and merged Native GPT/Grok picker activation in one job:

```text
Read AGENTS.md completely and follow it. On this Mac, build and install the V1.0
isolated grok-bridge profile, then continue through the experimental V1.1 merged
Native GPT/Grok picker activation in the same job. Before picker activation,
identify the current authoritative Native Codex catalog and the exact effective
first-party Responses upstream without reading, copying, or printing credentials.
Use ./scripts/materialize-macos.sh and only the repository-owned native lifecycle
commands. Preserve existing changes and the exact rollback boundary. Verify the
installed service, merged native/Grok catalog, and 272,000-token Grok context
metadata with the minimum primary-path checks. Do not guess either Native Codex
input, patch the Codex binary, edit Codex configuration, Grok authentication, or
LaunchAgent files directly, or commit, push, or publish. If an authoritative input
cannot be verified, stop before picker activation and explain what is missing. On
success, report the required fresh CLI/Desktop restart and exact picker rollback
command.
```

This route automates every repository-owned V1.1 installation step. A fresh Codex CLI process or full Desktop relaunch remains necessary after activation so Codex loads the published catalog and provider state.

## Acknowledgements

This project owes a substantial design debt to [duolahypercho/codex-router](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174). Its implementation and documentation were an important feasibility reference and materially informed our decisions around official Grok credential freshness, provider-bound metadata, xAI Responses/SSE taxonomy, native picker integration, and reversible activation. We sincerely thank the author and contributors for publishing that work under the MIT License.

No `codex-router` source code is copied into this repository. `grok-codex-bridge` remains an independent Rust implementation with a direct Responses-to-Responses transport rather than a LiteLLM/Chat multi-hop.

## Why Rust

The bridge is written in Rust so the normal runtime is one prebuilt native executable with no Python, Node.js, or JIT runtime dependency. Rust also gives the protocol and lifecycle boundaries strong types, explicit error handling, memory-safe concurrency, and deterministic release materialization.

Normal use never compiles on demand. Cargo is used only for development and construction; the launcher executes the materialized binary directly and fails closed when it is missing or stale.

## Architecture

```text
Codex harness
  agent loop · tools · permissions · MCP · Skills · sessions
        |
        | capability-scoped Responses request
        v
grok-codex-bridge (native Rust executable)
  local authentication · request normalization · SSE validation
        |
        | origin-locked Responses transport
        v
Grok / xAI
```

The bridge does not execute tool calls. It preserves function definitions, ordered tool calls and results, text, image URLs and data URIs, reasoning items, and supported Responses controls while Codex remains responsible for execution.

## Project status

| Surface | Status |
| --- | --- |
| V1.0 isolated `grok-bridge` profile | Implemented and validated in the Codex CLI |
| Native Rust build and reversible user service | Implemented and validated |
| V1.1 merged Native GPT/Grok model picker | Implemented; CLI switching and history continuity validated |
| V1.1 skill metadata budget | Grok catalog entries publish a 272,000-token context window, using Codex's native 2% calculation |
| Desktop picker and final rollback acceptance | Pending final verification |
| Public release binaries | Not published; build and materialize from source |

V1.0 is the conservative public route: it uses a separate Codex profile and leaves Native GPT configuration untouched. V1.1 is currently experimental because its Desktop and final rollback acceptance are not complete.

## Features

- Strict Codex Responses-to-xAI Responses normalization; no legacy Chat Completions conversion.
- Event-by-event SSE validation with stable IDs, coordinates, sequence numbers, completed text, tool arguments, and terminal state.
- Ordered function calls/results and mixed text/image inputs without downloading or re-encoding image data.
- Read-only use of the official Grok session credential, with in-memory zeroizing cache reload when the source changes.
- Fixed official xAI origin through rustls, redirects disabled, and typed authentication, rate-limit, status, and stream failures.
- Catalog-driven Grok model admission with atomic metadata-only last-known-good state.
- Loopback-only listener and capability-scoped routes; invalid capabilities return `404`.
- Reversible install, LaunchAgent service lifecycle, diagnostics, picker activation, and exact configuration rollback.
- Metadata-only logging that does not log request paths, capability material, credentials, or response bodies.

## Requirements

- macOS on Apple Silicon.
- Rust 1.95.0 for building from source, pinned by [rust-toolchain.toml](rust-toolchain.toml).
- An existing official Grok login for live Grok requests.
- A current Codex CLI installation.

Prebuilt Intel macOS, Linux, and Windows artifacts are not currently provided.

## Quick start: isolated V1.0 profile

From the repository root, materialize the native executable once, then use the repository launcher:

```sh
./scripts/materialize-macos.sh
./scripts/grok-codex.sh
```

The launcher installs the bridge on first use, starts its user service when needed, and opens Codex with the isolated `grok-bridge` profile. Later runs reuse the installed native executable. Arguments are passed through to Codex:

```sh
./scripts/grok-codex.sh --version
./scripts/grok-codex.sh --activate-only
```

The launcher is repository-scoped. Moving or symlinking the shell script by itself is not a supported distribution method.

## Experimental V1.1 merged picker

The V1.1 route publishes a merged model catalog so Native GPT and admitted Grok models can be selected in one Codex model picker. Native model traffic remains bound to the captured first-party Codex upstream; Grok traffic is sent to xAI through the bridge.

First materialize and install the native bridge:

```sh
./scripts/materialize-macos.sh
./dist/aarch64-apple-darwin/grok-codex-bridge install
```

Then activate the picker with the current authoritative Native Codex catalog and the exact effective first-party Responses base URL captured before activation. The following is an example for a standard ChatGPT-authenticated Codex setup:

```sh
CODEX_DIR="${CODEX_HOME:-"$HOME/.codex"}"

./dist/aarch64-apple-darwin/grok-codex-bridge picker install \
  --native-catalog "$CODEX_DIR/models_cache.json" \
  --native-upstream-base-url "https://chatgpt.com/backend-api/codex"
```

`--native-catalog` must resolve to an absolute existing file. Do not copy the example upstream URL when a different first-party upstream is effective for your Codex authentication route.

Start a fresh Codex CLI process after activation. Fully quit and relaunch Codex Desktop before testing the Desktop picker.

The merged Grok 4.5 and 4.6 entries expose a 272,000-token context window. Codex therefore applies the same native 2% skill-description budget calculation instead of falling back to the small unknown-window budget.

### Current resume limitation

With Grok selected at shutdown, resuming that session directly may show:

```text
MCP startup interrupted. The following servers were not initialized: codex_apps
```

Current evidence places this at Codex's TUI resume/MCP-startup boundary, not at the bridge transport or the `codex_apps` handshake. Until the Codex-side behavior is resolved, resume with a Native GPT model and switch to Grok after startup:

```sh
codex resume <SESSION_ID> -m <NATIVE_GPT_MODEL>
```

## Lifecycle and rollback

All repository commands below use the materialized executable directly:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge doctor
./dist/aarch64-apple-darwin/grok-codex-bridge auth status
./dist/aarch64-apple-darwin/grok-codex-bridge service status
```

Remove only the merged picker state and restore the exact pre-picker Codex configuration:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge picker uninstall
```

Stop the user service, then remove the bridge-owned installation:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge service uninstall
./dist/aarch64-apple-darwin/grok-codex-bridge uninstall
```

The lifecycle manifest owns only files created or replaced by the bridge. Full uninstall does not remove the base Codex configuration, official Grok authentication state, or Native GPT configuration.

## Model catalog and credentials

The bridge discovers the official Grok session credential from the configured `GROK_AUTH_PATH`, an absolute `GROK_HOME`, or the official default Grok home. The selected credential file is opened read-only without following symlinks. Login, token refresh, and credential repair remain the responsibility of the official Grok flow.

Request one bounded official catalog refresh without starting the server:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge catalog refresh \
  --config ./docs/bridge-config.example.toml
```

The checked-in example intentionally disables refresh-on-start. For a live refresh, copy it to an untracked local file, replace its placeholder paths with valid absolute runtime paths, and enable the option there. Never commit credentials or runtime-specific paths.

## Security boundary

- The listener binds to loopback only; LAN exposure is outside the V1 scope.
- Caller capability material is placed in the local route and never written to service logs.
- Credentials remain in their authoritative official file and a zeroizing memory cache; they are not copied into catalog or Codex state.
- Grok transport is restricted to the official xAI origin, uses rustls, and does not follow redirects.
- Catalog writes and managed configuration changes are atomic and reversible.
- Credentials, tokens, `.env` files, private keys, runtime state, session logs, generated catalogs, and machine-specific paths must not enter Git history.
- The crate declares `publish = false` to prevent accidental crates.io publication.

## Development

Run the source test suite:

```sh
cargo test --locked
```

For a development-only source invocation:

```sh
cargo run -- --version
```

`cargo run` is not a normal runtime route. Materialize the release binary and invoke it directly:

```sh
./scripts/materialize-macos.sh
./dist/aarch64-apple-darwin/grok-codex-bridge status
```

The product scope and acceptance contracts are defined in [docs/spec-v0.1.md](docs/spec-v0.1.md). Distribution requirements are tracked in [docs/distribution-contract.md](docs/distribution-contract.md).

## Source layout

```text
src/cli.rs                    CLI boundary
src/config.rs                 versioned runtime configuration
src/credential.rs             read-only Grok credential boundary
src/catalog.rs                atomic metadata-only model catalog
src/grok.rs                   origin-locked xAI transport
src/protocol.rs               Responses normalization and SSE validation
src/server.rs                 capability-scoped loopback service
src/lifecycle.rs              reversible install and rollback ownership
src/picker.rs                 merged Native GPT/Grok catalog generation
src/picker_activation.rs      atomic picker publication and activation
src/launchd.rs                typed user LaunchAgent boundary
scripts/materialize-macos.sh  deterministic macOS arm64 materialization
```

## License

Licensed under the [MIT License](LICENSE).
