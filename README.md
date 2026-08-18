# grok-codex-bridge

`grok-codex-bridge` is a standalone Rust project for a native local protocol bridge between Codex and Grok. It is not a Codex plugin, a general-purpose LLM router, or an agent harness.

## Motivation

The project exists to keep using the Codex harness we value while letting Grok work at full strength as the inference model. The bridge should make Grok usable through Codex's tools and operating model without replacing GPT, rebuilding the harness, or taking over unrelated Codex traffic.

## Current status

This repository currently contains the development foundation and the implemented Phase A through Phase F source slices:

- a Rust library and binary package;
- a fail-closed CLI with `run`, `status`, and `version`;
- a versioned TOML runtime configuration;
- a loopback-only HTTP server with capability-path caller authentication;
- capability-scoped `healthz` and catalog-backed `/v1/models` endpoints;
- a replaceable last-known-good Grok model catalog bootstrapped with the current verified Grok 4.6 and 4.5 entries;
- read-only discovery and validation of the official Grok session credential file;
- an in-memory credential cache that reloads changed source bytes without copying credentials to bridge state;
- an origin-locked rustls xAI client with redirects disabled and typed authentication, rate-limit, status, and stream failures;
- a bounded official `/v1/models` refresh that admits Responses-backed models and atomically persists metadata-only catalog state;
- a capability-scoped, bounded `POST /v1/responses` route that admits catalog models and loads the read-only Grok credential only after local request validation;
- strict Codex Responses normalization that preserves instructions, message order, text and image bytes, and verified request controls without a legacy protocol conversion;
- an explicit Responses SSE lifecycle validator for stable IDs, coordinates, monotonically increasing sequence numbers, completed text, and terminal state;
- lossless function tool definitions and tool choice, ordered calls and string results, and parallel argument-delta validation keyed by output index;
- ordered image URL/data-URI inputs and mixed text/image function results without download, decode, or re-encoding;
- a transport boundary that accepts only a normalized request and can yield validated text or function-call events;
- validated event-by-event SSE relay with typed pre-stream errors for malformed requests, missing credentials, authentication, rate limits, and upstream failures;
- metadata-only service logging that never logs request paths or capability material;
- an atomic, reversible install manifest that owns only the bridge binary, runtime files, isolated Codex profile, LaunchAgent plist, and exact backups it creates;
- separate `install`, `doctor`, `auth status`, `service`, and `uninstall` CLI boundaries, with service activation kept out of install;
- a user-domain LaunchAgent contract with typed load, stop, and status results and no raw `launchctl` output;
- focused CLI tests;
- a deterministic macOS arm64 materialization route;
- the normalized [v0.1 product specification](docs/spec-v0.1.md);
- the [CAO V1.0 development brief](docs/cao-v1.0-development-brief.md);
- the [new Codex project handoff](docs/codex-project-handoff.md);
- the [CAO V1.1 development brief](docs/cao-v1.1-development-brief.md) and [new-session handoff](docs/codex-v1.1-session-handoff.md);
- the [future native distribution contract](docs/distribution-contract.md);
- the R0 current external-contract record and source/security boundaries for later phases.

Phase G used the logged-in official Grok session read-only. With current Codex CLI `0.148.0-alpha.15` through the isolated `grok-bridge` profile and Grok 4.6, exactly one Codex-owned shell tool call read the scratch marker and returned its exact final token. For acceptance, the prebuilt bridge was installed as a user LaunchAgent, then service/uninstall removed the install root, profile, plist, and listener; the Native GPT base-config hash remained unchanged. The isolated CLI did not expose a Computer Use/screenshot tool, so the conditional live Computer Use branch was unavailable; image preservation remains proven by focused mocks, not live Computer Use. V1.1 remains excluded.

## One-command use

From the repository root, start the prebuilt bridge and enter Codex with the isolated Grok profile:

```sh
./scripts/grok-codex.sh
```

Arguments are passed to Codex, so `./scripts/grok-codex.sh --version` is a non-interactive handoff check. Use `./scripts/grok-codex.sh --activate-only` to start the background service without entering Codex.

Only the first run installs the bridge. Later runs validate the existing installation, start the service only when it is stopped, and enter Codex without reinstalling.

The launcher never invokes Cargo, rustc, or an on-demand build. If the native binary is missing or older than its Rust source, it fails closed and asks for an explicit `./scripts/materialize-macos.sh` construction step.

This launcher is intentionally repository-scoped: moving or symlinking the shell file by itself is not a supported distribution method. The reproducible install-once, `grok-codex`-from-`PATH` design is recorded in [the native distribution contract](docs/distribution-contract.md). It remains future packaging work and does not start V1.1.

## Responsibility boundary

Codex remains the owner of the agent loop, permission decisions, tool execution, shell access, filesystem access, MCP, Skills, Browser, Computer Use, and task/session state.

The bridge owns only the local provider endpoint, protocol translation, streaming translation, read-only access to the selected Grok credential source, local caller authentication, and the Grok upstream client. It must not execute requested tools itself.

```text
Codex harness
    | capability-scoped Responses handoff
    v
grok-codex-bridge
    | origin-locked Responses transport with validated SSE relay
    v
Grok
```

Exact Codex and Grok protocol contracts must be verified against their current authoritative sources before implementation. The repository does not treat an earlier design conversation as executable authority.

The product scope, responsibility boundary, implementation order, and acceptance conditions are authoritative in `docs/spec-v0.1.md`. CAO keeps the active V1.0 development state locally under `.CAO/`; that state is excluded from Git and does not replace the source specification.

## Acknowledgements

[duolahypercho/codex-router v0.4.0-beta.4](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174) informed three specification boundaries: credential freshness remains mediated by the official Grok flow rather than copied or reimplemented as bridge-owned auto-refresh; external-provider `client_metadata` terminates at the provider boundary; and xAI Responses reasoning and SSE event types are treated as an explicit protocol taxonomy. No codex-router source code was copied into this repository.

This project remains a direct Rust Responses-to-Responses bridge. It does not adopt codex-router's LiteLLM/Chat multi-hop, namespace flattening, fixed model registry, hosted-tool injection, or any V1.1 behavior.

## Development

Requirements:

- macOS on Apple Silicon for the provided materialization route;
- Rust 1.95.0, pinned by `rust-toolchain.toml`.

Run the source tests:

```sh
cargo test --locked
```

For a development-only source invocation:

```sh
cargo run -- --version
```

`cargo run` is not the normal runtime route. Materialize the release binary instead:

```sh
./scripts/materialize-macos.sh
./dist/aarch64-apple-darwin/grok-codex-bridge status
```

Normal callers must execute the materialized binary directly. They must not compile on first use, search Cargo build caches, or fall back to an interpreted implementation.

### Phase A–F development run

Copy `docs/bridge-config.example.toml` to a temporary development location. Set `capability_token_file` and `catalog_cache_file` to absolute paths. The capability token file must be a regular non-symlink file with mode `0600` and contain 32–128 URL-safe ASCII bytes.

`refresh_on_start` defaults to `true`. The checked-in development example sets it to `false` so an ordinary local service smoke does not read Grok credentials or contact xAI. Set it to `true` only when one bounded startup refresh is intended, then run the materialized binary:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge run --config /absolute/path/to/bridge.toml
```

With the token substituted for `<capability>`, the service exposes:

```text
GET /_grok/<capability>/healthz
GET /_grok/<capability>/v1/models
POST /_grok/<capability>/v1/responses
```

Wrong capabilities return `404`. Health and models do not read the Grok credential or contact xAI. A Responses request is bounded and strictly normalized, must select an admitted catalog model, then loads the official Grok session credential and streams only validated upstream events. On startup, a valid metadata-only catalog cache is loaded before an optional official refresh. An authentication, network, timeout, origin, schema, or model-admission failure preserves the last-known-good cache or built-in bootstrap catalog rather than replacing it with partial data.

### Grok credential and model catalog

Phase B resolves the official Grok session credential source in this order:

1. the absolute path in `GROK_AUTH_PATH`, when set;
2. `$GROK_HOME/auth.json`, when `GROK_HOME` is set to an absolute path;
3. `~/.grok/auth.json` from the current absolute home directory.

The selected file is opened read-only without following a symlink. Session material remains in a redacted, zeroizing memory cache and is reloaded when the source file changes; it is never written to the model catalog cache. Login, refresh-token exchange, credential repair, or mutation remains the responsibility of the official Grok flow.

To request one explicit official model refresh without starting the server:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge catalog refresh --config /absolute/path/to/bridge.toml
```

Only entries explicitly backed by the Responses API and the fixed official xAI inference origin are admitted. A successful refresh replaces the in-memory catalog and writes a versioned metadata-only cache atomically; a failed refresh leaves the previous catalog intact. Because routing is catalog-driven rather than hard-coded to one model slug, a future official entry such as `grok-4.7` can be admitted by refresh without a bridge source change. This does not add the excluded V1.1 native model picker or change native GPT behavior.

### Reversible lifecycle and isolated profile

The lifecycle CLI keeps materialization, diagnosis, service activation, and removal as distinct actions. Defaults are scoped to the current absolute home directory:

```text
install root:  ~/Library/Application Support/grok-codex-bridge
Codex home:    $CODEX_HOME when absolute, otherwise ~/.codex
LaunchAgent:   ~/Library/LaunchAgents/com.local.grok-codex-bridge.plist
bind:          127.0.0.1:8746
initial model: grok-4.6
```

Install the materialized current executable and isolated profile without loading the service:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge install
```

`install` does not call `launchctl`. It prints the two explicit next actions:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge service install
codex --profile grok-bridge
```

The generated `grok-bridge.config.toml` uses the current profile-v2 provider shape: its top level selects `model = "grok-4.6"` and `model_provider = "grok_bridge"`; `[model_providers.grok_bridge]` supplies the capability-scoped loopback `base_url`, `wire_api = "responses"`, `requires_openai_auth = false`, and `supports_websockets = false`. This file belongs only to the explicit `grok-bridge` profile. It does not edit the base Codex configuration, intercept Native GPT, or change the ordinary model route.

The initial profile model is a safe default, not a hard-coded model ceiling. Run the explicit catalog refresh command after xAI publishes a new Responses-backed model; an official future entry such as `grok-4.7` is admitted without a source change. Select an admitted model with Codex `-m <model>` or by changing the isolated profile through its managed lifecycle. V1.0 deliberately does not implement the excluded V1.1 native model picker.

Lifecycle inspection and control commands are:

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge doctor
./dist/aarch64-apple-darwin/grok-codex-bridge auth status
./dist/aarch64-apple-darwin/grok-codex-bridge service status
./dist/aarch64-apple-darwin/grok-codex-bridge service uninstall
./dist/aarch64-apple-darwin/grok-codex-bridge uninstall
```

`doctor` reports only stable check identifiers, passed/failed messages, and the typed service state; `auth status` reports only availability. Neither command prints paths, caller capabilities, tokens, identities, response bodies, or raw `launchctl` output. `service uninstall` stops the user service but never deletes its plist. Full `uninstall` first requires a successful stop or authoritative already-stopped result, then restores or removes only files proven by the install manifest. It never removes the base Codex configuration, Grok authentication state, or Native GPT configuration.

All lifecycle path options accept absolute overrides (`--source-binary`, `--install-root`, `--codex-home`, `--launch-agent`, and `--credential-file` where applicable). Lifecycle and auth inspection do not contact xAI. `catalog refresh` remains the only explicit CLI command in this phase that performs a bounded xAI network request; `run` may also perform the configured startup refresh and inference traffic.

## Source layout

```text
src/cli.rs                    CLI boundary
src/config.rs                 versioned config and caller-capability loading
src/credential.rs             read-only official Grok session credential boundary
src/catalog.rs                atomic catalog and metadata-only disk cache
src/grok.rs                   origin-locked models and Responses transport
src/protocol.rs               typed Responses normalization and SSE lifecycle
src/server.rs                 loopback HTTP service and scoped routes
src/main.rs                   binary entry point and metadata-only logging
src/lifecycle.rs              reversible install, doctor, auth status, and uninstall ownership
src/launchd.rs                user LaunchAgent rendering and typed launchctl boundary
tests/cli.rs                  representative process-level CLI checks
scripts/materialize-macos.sh  Source-owned release materialization
dist/                       Ignored materialized runtime output
target/                     Ignored Cargo build cache
```

## Security and publication boundary

- Listeners bind to loopback only; LAN exposure is outside the V1 scope.
- Credentials, tokens, `.env` files, private keys, runtime state, and user-specific absolute paths must not enter Git history.
- Grok credentials are read from the authoritative file in place and are not copied into this repository, the catalog cache, or Codex state.
- Grok HTTP transport uses the fixed official xAI origin through rustls and does not follow redirects; catalog refresh is bounded and a streaming Responses request has no whole-stream timeout.
- `publish = false` prevents accidental crates.io publication.
- GitHub remote creation, GitHub publication, release artifacts, repository URL metadata, and license selection remain separate user-authorized decisions.
