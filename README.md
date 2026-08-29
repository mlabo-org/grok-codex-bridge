# grok-codex-bridge

[日本語](README.ja.md)

**A native Rust Responses-to-Responses bridge that lets Grok run inside the Codex harness without replacing Native GPT.**

`grok-codex-bridge` is a standalone, loopback-only provider bridge for macOS on Apple Silicon. Codex continues to own the agent loop, tools, permissions, MCP servers, Skills, and session state. This project owns only the local provider boundary, tolerant provider projection for Responses transport, Codex-consumed SSE extraction, the bridge-side Grok credential boundary, and the upstream connection to xAI. Credential recovery is delegated to the official Grok CLI; see [Model catalog and credentials](#model-catalog-and-credentials).

It is not a Codex plugin, a general-purpose LLM router, or an agent harness.

## One-command environment switching

The repository includes [AGENTS.md](AGENTS.md) as a binding safety and lifecycle contract for coding agents. Clone the repository once, then use the repository-owned migration command from its root:

```sh
git clone https://github.com/mlabo-org/grok-codex-bridge.git
cd grok-codex-bridge
./scripts/materialize-macos.sh
```

Enable the merged Native GPT/Grok picker:

```sh
./scripts/grok-codex.sh grok
```

Switch to Native compatibility mode without rewriting saved tasks:

```sh
./scripts/grok-codex.sh native
```

`grok` routes Grok slugs to xAI and Native GPT slugs to OpenAI. In Grok mode, the picker exposes both families. In Native mode, the picker exposes Native GPT for new selection while retaining the Grok provider metadata needed to open and continue existing tasks; it rewrites a saved Grok slug only in the outbound request copy to the current Native GPT model. Neither direction rewrites the provider or model stored on a task.

Both commands hand the transition through the locally built native LaunchServices launcher to the Rust coordinator, without depending on Terminal.app after handoff. The coordinator validates and, when needed, replaces the paired runtime before asking ChatGPT.app to quit gracefully. After the app and app-server are absent, it applies the rollback-owned picker transition. A successful transition relaunches ChatGPT.app with the new state; a failed picker transition rolls back and attempts to relaunch the entry state before returning the failure.

### Source repository and installed runtime

The repository contains the complete public source for both native components: the Rust bridge executable and its matching Swift `Grok Codex Switch.app` launcher. Materialization must produce this pair for Apple Silicon; do not install one without the other. The launcher survives ChatGPT.app shutdown, runs the Rust switch coordinator to completion, and lets that coordinator relaunch ChatGPT.app after a successful transition. Installation copies the resulting executable, matching launcher bundle, configuration, catalog state, overlay/resource files, and lifecycle data into:

```text
~/Library/Application Support/grok-codex-bridge/
├── bin/grok-codex-bridge
├── bin/Grok Codex Switch.app/
│   └── Contents/Resources/grok-codex-bridge-overlay.md
├── config/bridge.toml
├── state/                 # catalog and picker-managed state
└── logs/
```

After installation, the switch coordinator reads only this installed tree and the live Codex/ChatGPT state it is explicitly handed. It does not compile, search Cargo's `target/`, read `dist/`, or depend on the checkout's `Grok.md` or replacement scripts during a normal switch. The checkout may therefore be moved or removed without invalidating an already installed runtime. The repository-owned `scripts/grok-codex.sh` remains the build/install/update entry point; it is not the runtime dependency of the installed bridge.

The repository entry points install or update the locally materialized runtime pair, then switch mode:

```sh
./scripts/grok-codex.sh grok    # install/update the materialized pair, then switch to Grok mode
./scripts/grok-codex.sh native  # install/update the materialized pair, then switch to Native mode
```

`grok-codex.sh` never compiles automatically. If either materialized component is missing or stale, it stops and asks you to run `./scripts/materialize-macos.sh` first. Source installation and updates therefore depend on the checkout; after installation, normal `mode grok` / `mode native` switching uses only the installed runtime tree and does not depend on the checkout.

After installation, normal switching is repository-independent and runs the installed native executable directly:

```sh
BRIDGE="$HOME/Library/Application Support/grok-codex-bridge/bin/grok-codex-bridge"
"$BRIDGE" mode grok
"$BRIDGE" mode native
```

The command prints that the transition normally takes approximately 15–20 seconds. That time is spent on runtime preparation, graceful ChatGPT.app/app-server shutdown, picker publication, and relaunch. Do not force-quit ChatGPT.app during this window. A successful transition is confirmed only after the automatic relaunch completes. If picker publication fails, the coordinator rolls it back and attempts to restore the entry-time Desktop running state before reporting the failure.

The two native components have separate responsibilities: the Rust executable owns provider, picker, service, and transition state; the Swift launcher survives the parent ChatGPT.app shutdown and starts the coordinator through LaunchServices. Neither component is an interpreter or a build-on-first-use wrapper.

## Acknowledgements

This project owes a substantial design debt to [duolahypercho/codex-router](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174). Its implementation and documentation were an important feasibility reference and materially informed our decisions around official Grok credential freshness, provider-bound metadata, tolerant Responses/SSE transport projection, native picker integration, and reversible activation. We sincerely thank the author and contributors for publishing that work under the MIT License.

No `codex-router` source code is copied into this repository. `grok-codex-bridge` remains an independent Rust implementation with a direct Responses-to-Responses transport rather than a LiteLLM/Chat multi-hop.

## Why Rust

The bridge is written in Rust so the normal runtime is a locally built native executable plus a native Swift launcher, with no Python, Node.js, or JIT runtime dependency. Rust also gives the protocol and lifecycle boundaries strong types, explicit error handling, memory-safe concurrency, and deterministic release materialization.

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

The bridge does not execute tool calls. It preserves valid function definitions, ordered tool calls and results, text, image URLs and data URIs, reasoning summaries, and required Responses controls while Codex remains responsible for execution. Function schemas that xAI would reject for the entire request are omitted from the Grok projection only; Codex catalog, tool_search history, and the Native GPT path keep the original tools. Integer-valued JSON numbers in function and tool_search arguments, such as `8.0`, are rewritten to JSON integers; real fractions are unchanged. Replayable message, function, and tool-search history is forwarded to Grok; completed Native `custom_tool_call` items and foreign reasoning are omitted. When Codex records assistant commentary inside a completed parallel tool batch, the xAI projection moves that commentary before the batch while preserving its content, call order, result order, and every `call_id`; the saved Codex history is not rewritten. On GPT/Grok switches it excludes only provider-unreplayable item IDs and reasoning state while preserving the `call_id` links between tool calls and outputs. Before any response content is committed downstream, transport establishment or body-stream failures are retried up to three times. After useful content has been emitted, the bridge never replays the request. If Grok closes after useful Codex-consumed events without a terminal marker, the bridge synthesizes only `response.completed`; it does not invent output items, and it does not synthesize after `response.failed` or `response.incomplete`.

## Project status

| Surface | Status |
| --- | --- |
| Merged Native GPT/Grok model picker | Primary route; implemented in source with bidirectional preservation of supported message/function/tool-search history at the bridge boundary |
| Skill metadata budget | Grok catalog entries publish a 272,000-token context window, using Codex's native 2% calculation |
| Native Rust build and reversible user service | Implemented for Apple Silicon macOS |
| One-command `grok` / `native` migration | Bidirectional runtime switching preserves saved provider/model values and coordinates graceful Desktop quit/relaunch |
| Public release binaries | Intentionally not distributed; each user builds and materializes locally from source |

The merged picker is the only documented operating route. Native GPT remains available in the same picker, and `native` switches routing mode without removing the provider.

## Features

- Tolerant Codex Responses-to-xAI Responses provider projection with `store: false`. Replayable message, function, and tool-search history is forwarded; completed Native `custom_tool_call` items and foreign reasoning are omitted from Grok requests. No legacy Chat Completions conversion.
- Codex-consumed SSE extraction for text, reasoning summaries, function calls, and terminal/usage events. Unknown auxiliary events do not terminate the stream. Transport establishment and early body-stream failures are retried up to three times only before downstream response content is committed. Integer-valued JSON numbers in function and tool_search arguments are canonicalized to JSON integers. If Grok ends a useful stream without `response.completed`, the bridge closes it with that Codex lifecycle marker only so the turn is not reported as `stream closed before response.completed`.
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
- A local source build is required. This project does not ship compiled binaries in the repository or through GitHub Releases; the Rust bridge and Swift launcher are compiled once on the user's own macOS arm64 machine and then installed.
- An official Grok CLI installation and either a current login or the ability to complete its official browser OAuth flow.
- A current Codex CLI installation.

Prebuilt Intel macOS, Linux, and Windows artifacts are not currently provided.

## Quick start: merged picker

The primary route publishes a merged model catalog so Native GPT and admitted Grok models can be selected in one Codex model picker. Native GPT `responses` and `responses/compact` stay on the captured first-party Codex upstream. `images/generations`, `images/edits`, and `alpha/search` are Native-only passthrough endpoints with no Grok protocol conversion. Grok traffic is sent to xAI through the bridge. Grok has no authoritative `responses/compact` contract, so that route fails closed for admitted Grok models.

Native and Grok model slugs must remain unique. Catalog generation and runtime routing both fail closed on a duplicate slug; the bridge never overwrites a Native row or invents an alias.

Build the matching native pair once, then run the migration:

```sh
./scripts/materialize-macos.sh
./scripts/grok-codex.sh grok
```

The command accepts only the app-bundled Codex authenticated through ChatGPT, resolves the current `models_cache.json` under the effective Codex home, and fails before picker mutation if either authoritative input is unavailable.

Start a fresh Codex CLI process after activation. Fully quit and relaunch Codex Desktop before testing the Desktop picker.

![Codex Desktop model picker with Native GPT models plus grok-4.5 and grok-4.6](docs/images/desktop-merged-picker.png)

Codex Desktop after merged picker activation. Native GPT and admitted Grok models appear in one picker.

Admitted Grok catalog entries, including the bootstrap `grok-4.5` and `grok-4.6` models, expose a 272,000-token context window. Codex therefore applies the same native 2% skill-description budget calculation instead of falling back to the small unknown-window budget.

### Grok.md overlay

[`Grok.md`](Grok.md) is the source of truth for the Grok-only execution overlay. `picker install` reads that file from disk and copies it into each admitted Grok row's `base_instructions` in the generated catalog. Codex consumes the generated catalog; Native GPT rows never receive this overlay. The Rust binary does not embed `Grok.md`; materialization copies its bytes as a separately named launcher resource snapshot, and the live HTTP path does not re-read it on every request.

The filename `Grok.md` is reserved exclusively for this live Grok constitution source. Materialization copies its bytes into the installed launcher under the deliberately distinct runtime-snapshot name `Contents/Resources/grok-codex-bridge-overlay.md`; no other resource is named `Grok.md`.

Omit `--grok-overlay` only when the current working directory already contains `Grok.md`. After changing the overlay, run `picker install` again and start a fresh Codex CLI process or fully relaunch Desktop. Existing Grok sessions keep the overlay that was in the catalog when they started.

The overlay is a companion contract, not a second constitution. It tells Grok to finish declared work in the same turn: call tools when needed, then return the user-visible result instead of ending on tool calls or progress notes alone. In a merged-picker session after catalog refresh and restart, that path is the one Codex actually loads for Grok.

### Official subagents

Codex owns subagent dispatch and lifecycle. The bridge only translates provider protocol; it does not spawn workers, define their tool surface, or own model and reasoning defaults.

- Use the official subagent tools exposed by the current Codex schema (for example, `spawn_agent`, `wait_agent`, and `interrupt_agent`). Tool names and availability can vary with the client or version; this bridge does not guarantee a particular set.
- When `model` or `reasoning_effort` is omitted, follow the current official schema and its context-propagation rules. This README does not promise a configured default, parent-model inheritance, or any other fixed behavior.
- If the current `spawn_agent` schema admits overrides and the child must run as Grok, set an admitted Grok catalog id such as `grok-4.6` or `grok-4.5` explicitly. Set `reasoning_effort` explicitly when that distinction matters and the schema permits it; otherwise the child model or effort must not be assumed.

## Lifecycle and rollback

### Updating an existing install

A Grok-backed Codex session depends on the local loopback service. Stopping the service or replacing the installed binary from inside that session cuts the model connection. Run migration from a Native GPT task or Terminal.

Perform materialization and installed-binary replacement from a session that does not use this bridge. Use Grok Build for that step, or a Codex session on a Native GPT model.

After materializing a new executable, replace the loaded install with the repository-owned replacement script. A direct replacement or a Grok-mode transition runs `auth ensure` before stopping the service: silent refresh is attempted first, and the official OAuth browser opens only when interactive recovery is still required. A source transition explicitly targeting Native compatibility skips Grok credential read, refresh, and login while retaining the same pair validation and rollback. The script then swaps the installed binary, restarts the service, and runs `doctor`. If replacement or restart fails, it attempts to restore the previous binary and service state:

```sh
./scripts/materialize-macos.sh
./scripts/replace-installed-bridge.sh \
  ./dist/aarch64-apple-darwin/grok-codex-bridge \
  "./dist/aarch64-apple-darwin/Grok Codex Switch.app"
```

After `service status` reports `service loaded`, start a fresh Codex CLI process or fully relaunch Desktop so the client reconnects.

The same migration entry point owns both migration directions:

```sh
./scripts/grok-codex.sh grok
./scripts/grok-codex.sh native
```

`native` does not uninstall the bridge. The picker exposes only Native models for selection while retaining hidden Grok metadata required to resolve saved tasks. It keeps the provider definition and loopback resolver, does not construct the Grok inference client, and rewrites only Grok-slug request copies to the current root Native model. The original request, saved tasks, SQLite state, and rollouts are not modified.

Full uninstall is not a routine environment switch. Removing the resolver permanently requires a separate, explicit irreversible migration of remaining picker provider/model references into Native Codex form; otherwise those tasks become unopenable.

Use `native` for a reversible Native-only operating mode. It is deliberately not an uninstall: the installed provider definition, resolver, and compatibility metadata remain available so a later `grok` transition can restore Grok routing without rewriting task history. Use full `uninstall` only when removing the bridge from the machine. That operation removes the installed runtime and rolls back bridge-owned Codex configuration/service state; it does not convert historical task records. If those records still contain bridge provider/model references, uninstalling first can make them impossible to open. Preserve the installed runtime until any separately planned, explicit data migration has completed.

Updating source is a different lifecycle boundary from switching mode. Build/materialize and install the new native components from a Native GPT task or Terminal, then allow the installed replacement path to stop and restart the service. Normal mode switching never rebuilds binaries. After an update, relaunch Codex Desktop once so it reads the newly published picker catalog.

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

- The listener binds to loopback only; LAN exposure is outside the product scope.
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

The product scope and acceptance contracts are defined in [docs/spec.md](docs/spec.md). Distribution requirements are tracked in [docs/distribution-contract.md](docs/distribution-contract.md).

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
scripts/macos-switch-launcher/       Swift LaunchServices launcher source
scripts/grok-codex.sh                one-command Grok/native environment migration
scripts/replace-installed-bridge.sh  loaded-install native runtime replacement
```

## License

Licensed under the [MIT License](LICENSE).
