# grok-codex-bridge

[日本語](README.ja.md)

**A native Rust Responses-to-Responses bridge that lets Grok run inside the Codex harness without replacing Native GPT.**

`grok-codex-bridge` is a standalone, loopback-only provider bridge for macOS on Apple Silicon. Codex continues to own the agent loop, tools, permissions, MCP servers, Skills, and session state. This project owns only the local provider boundary, tolerant provider projection for Responses transport, Codex-consumed SSE extraction, the bridge-side Grok credential boundary, and the upstream connection to xAI. The bridge inspects the official credential read-only; on hard expiry it may invoke the official Grok CLI as a bounded renewal trigger, while the official CLI owns any credential update.

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

A first-time install can run from Codex. Do not stop or replace an already running bridge from a Grok-backed Codex session; see [Updating an existing install](#updating-an-existing-install).

## Acknowledgements

This project owes a substantial design debt to [duolahypercho/codex-router](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174). Its implementation and documentation were an important feasibility reference and materially informed our decisions around official Grok credential freshness, provider-bound metadata, tolerant Responses/SSE transport projection, native picker integration, and reversible activation. We sincerely thank the author and contributors for publishing that work under the MIT License.

No `codex-router` source code is copied into this repository. `grok-codex-bridge` remains an independent Rust implementation with a direct Responses-to-Responses transport rather than a LiteLLM/Chat multi-hop.

## Why Rust

The bridge is written in Rust so the normal runtime is one prebuilt native executable with no Python, Node.js, or JIT runtime dependency. Rust also gives the protocol and lifecycle boundaries strong types, explicit error handling, memory-safe concurrency, and deterministic release materialization.

Normal use never compiles on demand. Cargo is used only for development and construction; the launcher executes the materialized binary directly and fails closed when it is missing or stale.

## Architecture

```text
Codex harness
  agent loop · tools · permissions · MCP · Skills · subagents · sessions
        |
        | capability-scoped Responses request
        v
grok-codex-bridge (native Rust executable)
  local authentication · provider projection · Codex SSE extraction
        |
        | origin-locked Responses transport
        v
Grok / xAI
```

The bridge does not execute tool calls. It preserves valid function definitions, ordered tool calls and results, text, image URLs and data URIs, reasoning summaries, and required Responses controls while Codex remains responsible for execution. Function schemas that xAI would reject for the entire request are omitted from the Grok projection only; Codex catalog, tool_search history, and the Native GPT path keep the original tools. On GPT/Grok switches it excludes only provider-unreplayable item IDs and reasoning state while preserving the `call_id` links between tool calls and outputs.

## Project status

| Surface | Status |
| --- | --- |
| V1.0 isolated `grok-bridge` profile | Implemented and validated in the Codex CLI |
| Native Rust build and reversible user service | Implemented and validated |
| V1.1 merged Native GPT/Grok model picker | Implemented; CLI switching validated; bidirectional switching preserves supported message/function/tool-search history at the bridge boundary |
| V1.1 skill metadata budget | Grok catalog entries publish a 272,000-token context window, using Codex's native 2% calculation |
| Desktop picker and final rollback acceptance | Pending final verification |
| Public release binaries | Not published; build and materialize from source |

V1.0 is the conservative public route: it uses a separate Codex profile and leaves Native GPT configuration untouched. V1.1 is currently experimental because its Desktop and final rollback acceptance are not complete.

## Features

- Tolerant Codex Responses-to-xAI Responses provider projection with `store: false` and full input history; no legacy Chat Completions conversion.
- Codex-consumed SSE extraction for text, reasoning summaries, function calls, terminal/usage events, while unknown auxiliary events do not terminate the stream.
- Ordered function calls/results and mixed text/image inputs without downloading or re-encoding image data.
- Read-only bridge-side use of the official Grok session credential, with in-memory zeroizing cache reload when the source changes. On hard expiry during a provider request, one bounded non-interactive official-CLI invocation may trigger the CLI's own silent OIDC refresh; the bridge never handles refresh tokens, performs OAuth, or writes the credential file.
- Fixed official xAI origin through rustls, redirects disabled, and typed authentication, rate-limit, status, and stream failures.
- Catalog-driven Grok model admission with atomic metadata-only last-known-good state.
- Loopback-only listener and capability-scoped routes; invalid capabilities return `404`.
- Reversible install, LaunchAgent service lifecycle, diagnostics, picker activation, and exact configuration rollback.
- Metadata-only logging that does not log request paths, capability material, credentials, or response bodies.
- Official Codex subagents stay on the Codex harness from Grok-backed sessions. Omitted spawn `model` / `reasoning_effort` follow Codex `[agents]` defaults, not the parent Grok session.

## Requirements

- macOS on Apple Silicon.
- Rust 1.95.0 for building from source, pinned by [rust-toolchain.toml](rust-toolchain.toml).
- An official Grok CLI installation and an existing official Grok login for live Grok requests.
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
  --native-upstream-base-url "https://chatgpt.com/backend-api/codex" \
  --grok-overlay "$PWD/Grok.md"
```

`--native-catalog` must resolve to an absolute existing file. Do not copy the example upstream URL when a different first-party upstream is effective for your Codex authentication route.

Start a fresh Codex CLI process after activation. Fully quit and relaunch Codex Desktop before testing the Desktop picker.

Admitted Grok catalog entries, including the bootstrap `grok-4.5` and `grok-4.6` models, expose a 272,000-token context window. Codex therefore applies the same native 2% skill-description budget calculation instead of falling back to the small unknown-window budget.

### Grok.md overlay

[`Grok.md`](Grok.md) is the source of truth for the Grok-only execution overlay. `picker install` reads that file from disk and copies it into each admitted Grok row's `base_instructions` in the generated catalog. Codex consumes the generated catalog; Native GPT rows never receive this overlay. The binary does not bake the file in at compile time, and the live HTTP path does not re-read it on every request.

Omit `--grok-overlay` only when the current working directory already contains `Grok.md`. After changing the overlay, run `picker install` again and start a fresh Codex CLI process or fully relaunch Desktop. Existing Grok sessions keep the overlay that was in the catalog when they started.

The overlay is a companion contract, not a second constitution. It tells Grok to finish declared work in the same turn: call tools when needed, then return the user-visible result instead of ending on tool calls or progress notes alone. In a merged-picker session after catalog refresh and restart, that path is the one Codex actually loads for Grok.

### Official subagents

Codex owns subagent dispatch. The bridge only translates provider protocol; it does not spawn workers.

Verified on a live V1.1 picker session:

- A Grok parent can spawn official Codex subagents (`spawn_agent` / `wait_agent` / `close_agent`).
- Omitting `model` or `reasoning_effort` applies Codex `[agents].default_subagent_model` and `[agents].default_subagent_reasoning_effort`. Those values are not the parent Grok model or effort.
- To run the child as Grok, set `model` to an admitted catalog id such as `grok-4.6` or `grok-4.5`.
- To set reasoning depth, set `reasoning_effort` explicitly. Current Grok catalog entries advertise `low`, `medium`, `high`, and `xhigh`. `grok-4.5` at `xhigh` has been verified.

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

### Updating an existing install

A Codex session that uses this bridge depends on the local loopback service. That includes the isolated `grok-bridge` profile and the V1.1 picker when a Grok model is selected. Stopping the service or replacing the installed binary from inside that session cuts the model connection. If the reload does not finish, the service stays `not_loaded` and Codex cannot reach Grok until the service is started again.

Perform materialization and installed-binary replacement from a session that does not use this bridge. Use Grok Build for that step, or a Codex session on a Native GPT model.

After materializing a new executable, replace the loaded install with the repository-owned replacement script. It stops the service, swaps the installed binary, restarts the service, and runs `doctor`:

```sh
./scripts/materialize-macos.sh
./scripts/replace-installed-bridge.sh ./dist/aarch64-apple-darwin/grok-codex-bridge
```

After `service status` reports `service loaded`, start a fresh Codex CLI process or fully relaunch Desktop so the client reconnects.

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

The bridge resolves the authoritative credential file in this order:

1. `GROK_AUTH_PATH`, when set;
2. `GROK_HOME/auth.json`, when `GROK_HOME` is set; or
3. `~/.grok/auth.json`.

The selected file is opened read-only without following symlinks. `GROK_AUTH_PATH` selects the file; `GROK_HOME` selects the official CLI home used for renewal. If `GROK_AUTH_PATH` points outside the official Grok home, set `GROK_HOME` as well so the bridge can resolve the matching `GROK_HOME/bin/grok` helper. With no `GROK_HOME`, the helper resolves to `~/.grok/bin/grok` when `HOME` is available.

The bridge uses `expires_at` when the official session record provides it. If that field is absent, it uses `create_time + 30 days` as its parser fallback; this is not a promise about the official Grok session lifetime. `auth status` reports credential availability without revealing the credential or its expiry timestamp.

The following recovery path runs only when a Responses provider request encounters a hard-expired credential. The bridge invokes the official `bin/grok models` command once with stdin, stdout, and stderr disconnected and a 7-second timeout. It then rereads the authoritative file for up to 60 seconds. The bridge does not proactively refresh a credential, read a refresh token, perform OAuth, invoke interactive login, or rewrite `auth.json`. If the official process does not replace the file, the request fails with an authentication error.

To restore an expired or missing official login, run the official device flow outside the bridge session:

```sh
GROK_HOME_DIR="${GROK_HOME:-"$HOME/.grok"}"
"$GROK_HOME_DIR/bin/grok" login --device-auth
```

Complete any device or browser confirmation only on the official page shown by the CLI. Never paste a device code into chat, logs, or the repository. Then verify the bridge without revealing credentials:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge auth status
./dist/aarch64-apple-darwin/grok-codex-bridge service status
```

`catalog refresh` is separate from the expired-credential recovery path. It requires a currently usable credential, does not invoke the renewal helper, and updates only the last-known-good model catalog. The checked-in configuration is a template with placeholder absolute paths and must not be run unchanged. Copy it to an untracked local configuration, replace the placeholders with valid absolute paths, and then run:

```sh
cp ./docs/bridge-config.example.toml ./bridge-config.local.toml
# Edit ./bridge-config.local.toml with machine-local absolute paths.
./dist/aarch64-apple-darwin/grok-codex-bridge catalog refresh \
  --config ./bridge-config.local.toml
```

The `refresh_on_start` field controls service startup only; the explicit `catalog refresh` command always performs its one bounded catalog request. Keep the local configuration untracked and never commit credentials or runtime-specific paths.

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
src/cli.rs                           CLI boundary
src/config.rs                        versioned runtime configuration
src/credential.rs                    read-only Grok credential boundary
src/catalog.rs                       atomic metadata-only model catalog
src/native.rs                        captured first-party Native GPT upstream route
src/grok.rs                          origin-locked xAI transport
src/protocol.rs                      Responses provider projection and SSE extraction
src/server.rs                        capability-scoped loopback service
src/lifecycle.rs                     reversible install and rollback ownership
src/picker.rs                        merged Native GPT/Grok catalog generation
src/picker_activation.rs             atomic picker publication and activation
src/launchd.rs                       typed user LaunchAgent boundary
scripts/materialize-macos.sh         deterministic macOS arm64 materialization
scripts/grok-codex.sh                V1.0 isolated profile launcher
scripts/replace-installed-bridge.sh  loaded-install binary replacement
```

## License

Licensed under the [MIT License](LICENSE).
