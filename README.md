# grok-codex-bridge

[日本語](README.ja.md)

**A native Rust Responses-to-Responses bridge that lets Grok run inside the Codex harness without replacing Native GPT.**

`grok-codex-bridge` is a standalone, loopback-only provider bridge for macOS on Apple Silicon. Codex continues to own the agent loop, tools, permissions, MCP servers, Skills, and session state. This project owns only the local provider boundary, tolerant provider projection for Responses transport, Codex-consumed SSE extraction, the bridge-side Grok credential boundary, and the upstream connection to xAI. Credential recovery is delegated to the official Grok CLI; see [Model catalog and credentials](#model-catalog-and-credentials).

It is not a Codex plugin, a general-purpose LLM router, or an agent harness.

## Install with Codex

The repository includes [AGENTS.md](AGENTS.md) as a binding safety and lifecycle contract for coding agents. To let Codex inspect the source, build the native executable, and activate the current V1.1 merged Native GPT/Grok route, clone the repository and start Codex from its root:

```sh
git clone https://github.com/mlabo-org/grok-codex-bridge.git
cd grok-codex-bridge
codex
```

Then choose one of the following requests. V1.1 is the primary route; the isolated V1.0 profile is retained as an experimental alternative.

### Install the V1.1 merged picker

Use this request when you want Codex to complete the native build, service installation, and merged Native GPT/Grok picker activation in one job:

```text
Read AGENTS.md completely and follow it. On this Mac, build and install the native
grok-codex-bridge, then activate the V1.1 merged Native GPT/Grok picker in the same
job. Before picker activation,
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

### Install the experimental isolated V1.0 profile

```text
Read AGENTS.md completely and follow it. Build and install the isolated V1.0
grok-bridge profile on this Mac. Verify the platform and prerequisites, preserve
existing changes, use ./scripts/materialize-macos.sh and the repository-owned
lifecycle commands, and run only the minimum primary-path checks. Do not enable
the V1.1 merged picker, edit the Codex binary, Codex configuration, Grok
authentication, or LaunchAgent files directly, or commit, push, or publish.
Stop and explain the missing boundary if this is not Apple Silicon macOS or a
required authoritative input cannot be verified.
```

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

The bridge does not execute tool calls. It preserves valid function definitions, ordered tool calls and results, text, image URLs and data URIs, reasoning summaries, and required Responses controls while Codex remains responsible for execution. Function schemas that xAI would reject for the entire request are omitted from the Grok projection only; Codex catalog, tool_search history, and the Native GPT path keep the original tools. Integer-valued JSON numbers in function and tool_search arguments, such as `8.0`, are rewritten to JSON integers; real fractions are unchanged. Replayable message, function, and tool-search history is forwarded to Grok; completed Native `custom_tool_call` items and foreign reasoning are omitted. On GPT/Grok switches it excludes only provider-unreplayable item IDs and reasoning state while preserving the `call_id` links between tool calls and outputs. If Grok closes after useful Codex-consumed events without a terminal marker, the bridge synthesizes only `response.completed`; it does not invent output items, and it does not synthesize after `response.failed` or `response.incomplete`.

## Project status

| Surface | Status |
| --- | --- |
| V1.1 merged Native GPT/Grok model picker | Primary route; implemented in source with bidirectional preservation of supported message/function/tool-search history at the bridge boundary |
| V1.1 skill metadata budget | Grok catalog entries publish a 272,000-token context window, using Codex's native 2% calculation |
| V1.0 isolated `grok-bridge` profile | Experimental isolated route; implemented in source |
| Native Rust build and reversible user service | Implemented for Apple Silicon macOS |
| Desktop picker and final rollback acceptance | Pending final verification |
| Public release binaries | Not published; build and materialize from source |

V1.1 is the current primary route because the bridge's main catalog, projection, model-switching, and overlay behavior is built around the merged picker. V1.0 is retained as an experimental isolated profile that leaves Native GPT configuration untouched. Desktop picker and final rollback acceptance remain explicitly pending rather than being presented as completed live validation.

## Features

- Tolerant Codex Responses-to-xAI Responses provider projection with `store: false`. Replayable message, function, and tool-search history is forwarded; completed Native `custom_tool_call` items and foreign reasoning are omitted from Grok requests. No legacy Chat Completions conversion.
- Codex-consumed SSE extraction for text, reasoning summaries, function calls, and terminal/usage events. Unknown auxiliary events do not terminate the stream. Integer-valued JSON numbers in function and tool_search arguments are canonicalized to JSON integers. If Grok ends a useful stream without `response.completed`, the bridge closes it with that Codex lifecycle marker only so the turn is not reported as `stream closed before response.completed`.
- Ordered function calls/results and mixed text/image inputs without downloading or re-encoding image data.
- Read-only bridge-side use of the official Grok session credential, with bounded recovery delegated to the official Grok CLI. See [Model catalog and credentials](#model-catalog-and-credentials).
- Fixed official xAI origin through rustls, redirects disabled, and typed authentication, rate-limit, status, and stream failures.
- Catalog-driven Grok model admission with atomic metadata-only last-known-good state.
- Loopback-only listener and capability-scoped routes; invalid capabilities return `404`.
- Reversible install, LaunchAgent service lifecycle, diagnostics, picker activation, and exact configuration rollback.
- Metadata-only logging that does not log request paths, capability material, credentials, or response bodies.
- Official Codex subagents stay on the Codex harness from Grok-backed sessions. Omitted spawn `model` / `reasoning_effort` follow Codex `[agents]` defaults, not the parent Grok session.

## Requirements

- macOS on Apple Silicon.
- Rust 1.95.0 for building from source, pinned by [rust-toolchain.toml](rust-toolchain.toml).
- An official Grok CLI installation and either a current login or the ability to complete its official browser OAuth flow.
- A current Codex CLI installation.

Prebuilt Intel macOS, Linux, and Windows artifacts are not currently provided.

## Quick start: V1.1 merged picker

The primary V1.1 route publishes a merged model catalog so Native GPT and admitted Grok models can be selected in one Codex model picker. Native GPT `responses` and `responses/compact` stay on the captured first-party Codex upstream. `images/generations`, `images/edits`, and `alpha/search` are Native-only passthrough endpoints with no Grok protocol conversion. Grok traffic is sent to xAI through the bridge. Grok has no authoritative `responses/compact` contract, so that route fails closed for admitted Grok models.

Native and Grok model slugs must remain unique. Catalog generation and runtime routing both fail closed on a duplicate slug; the bridge never overwrites a Native row or invents an alias.

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

![Codex Desktop model picker with Native GPT models plus grok-4.5 and grok-4.6](docs/images/desktop-merged-picker.png)

Codex Desktop after V1.1 picker activation. Native GPT and admitted Grok models appear in one picker. This is a display example; Desktop picker and final rollback acceptance remain pending.

Admitted Grok catalog entries, including the bootstrap `grok-4.5` and `grok-4.6` models, expose a 272,000-token context window. Codex therefore applies the same native 2% skill-description budget calculation instead of falling back to the small unknown-window budget.

### Grok.md overlay

[`Grok.md`](Grok.md) is the source of truth for the Grok-only execution overlay. `picker install` reads that file from disk and copies it into each admitted Grok row's `base_instructions` in the generated catalog. Codex consumes the generated catalog; Native GPT rows never receive this overlay. The binary does not bake the file in at compile time, and the live HTTP path does not re-read it on every request.

Omit `--grok-overlay` only when the current working directory already contains `Grok.md`. After changing the overlay, run `picker install` again and start a fresh Codex CLI process or fully relaunch Desktop. Existing Grok sessions keep the overlay that was in the catalog when they started.

The overlay is a companion contract, not a second constitution. It tells Grok to finish declared work in the same turn: call tools when needed, then return the user-visible result instead of ending on tool calls or progress notes alone. In a merged-picker session after catalog refresh and restart, that path is the one Codex actually loads for Grok.

### Official subagents

Codex owns subagent dispatch. The bridge only translates provider protocol; it does not spawn workers.

- A Grok parent can spawn official Codex subagents (`spawn_agent` / `wait_agent` / `close_agent`).
- Omitting `model` or `reasoning_effort` applies Codex `[agents].default_subagent_model` and `[agents].default_subagent_reasoning_effort`. Those values are not the parent Grok model or effort.
- To run the child as Grok, set `model` to an admitted catalog id such as `grok-4.6` or `grok-4.5`.
- To set reasoning depth, set `reasoning_effort` explicitly. Current Grok catalog entries advertise `low`, `medium`, `high`, and `xhigh`.

## Experimental isolated V1.0 profile

The V1.0 launcher keeps Grok in a separate Codex profile and is retained as an experimental isolated route. From the repository root, materialize the native executable once, then use the repository launcher:

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

## Lifecycle and rollback

### Updating an existing install

A Codex session that uses this bridge depends on the local loopback service. That includes the isolated `grok-bridge` profile and the V1.1 picker when a Grok model is selected. Stopping the service or replacing the installed binary from inside that session cuts the model connection. If the reload does not finish, the service stays `not_loaded` and Codex cannot reach Grok until the service is started again.

Perform materialization and installed-binary replacement from a session that does not use this bridge. Use Grok Build for that step, or a Codex session on a Native GPT model.

After materializing a new executable, replace the loaded install with the repository-owned replacement script. Before stopping the service, it runs `auth ensure`: silent refresh is attempted first, and the official OAuth browser opens only when interactive recovery is still required. The script then swaps the installed binary, restarts the service, and runs `doctor`. If replacement or restart fails, it attempts to restore the previous binary and service state:

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

When a Responses provider request encounters recoverable missing, incomplete, or expired credential state, the bridge invokes the official `bin/grok models` command once with stdin, stdout, and stderr disconnected and a 7-second timeout. It then rereads the authoritative file for up to 60 seconds. This non-interactive path does not open a browser; if it cannot refresh, the request fails with an authentication error.

For explicit lifecycle work, `auth ensure` first performs the same read-only check and silent refresh. A valid or silently renewed credential exits immediately. If recoverable missing, incomplete, or expired state remains, it launches the official desktop OAuth flow once with process output suppressed, waits up to five minutes for browser completion, and rereads the authoritative file. The official CLI owns the browser and credential update; malformed, ambiguous, or unsafe files fail closed without launching login.

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge auth ensure
```

Complete browser confirmation only on the official `auth.x.ai` page opened by the CLI. Never paste authentication material into chat, logs, or the repository. The loaded-binary replacement script runs `auth ensure` before it stops the current service, so an expired login no longer causes a post-restart doctor rollback. Verify without revealing credentials:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge auth status
./dist/aarch64-apple-darwin/grok-codex-bridge service status
```

`catalog refresh` is separate from all automatic credential recovery. It requires a currently usable credential, invokes neither the silent-refresh helper nor interactive login, and updates only the last-known-good model catalog. The checked-in configuration is a template with placeholder absolute paths and must not be run unchanged. Copy it to an untracked local configuration, replace the placeholders with valid absolute paths, and then run:

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
src/lib.rs                           crate root
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
Grok.md                              Grok overlay SSOT read at picker catalog generation
src/picker_activation.rs             atomic picker publication and activation
src/launchd.rs                       typed user LaunchAgent boundary
scripts/materialize-macos.sh         deterministic macOS arm64 materialization
scripts/grok-codex.sh                V1.0 isolated profile launcher
scripts/replace-installed-bridge.sh  loaded-install binary replacement
```

## License

Licensed under the [MIT License](LICENSE).
