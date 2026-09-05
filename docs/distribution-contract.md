# Source-only macOS Distribution Contract

Status: active source-distribution contract for the current Native/Grok environment switcher.

This repository distributes source and deterministic local build/materialization instructions. It does not distribute compiled binaries.

## Scope

The supported target is macOS on Apple Silicon (`arm64`). Each user compiles the runtime locally on their own Mac. Intel macOS, Linux, and Windows are outside this contract.

The product is one paired local runtime:

- `grok-codex-bridge`: the Rust native executable that owns the loopback provider, catalog and picker state, service lifecycle, credentials boundary, and desktop mode-transition coordinator;
- `Grok Codex Switch.app`: the Swift native launcher that survives the ChatGPT.app shutdown and starts the Rust coordinator through LaunchServices;
- `Grok.md`: the repository's reserved Grok-only constitution and overlay source of truth;
- `grok-codex-bridge-overlay.md`: the deliberately distinct installed snapshot name for the bytes copied from `Grok.md` into the launcher bundle.

The Swift launcher and the Rust bridge are one paired materialization unit. A Rust executable without its matching launcher and overlay snapshot is not a complete switcher runtime.

## Source-only distribution

The repository and its normal GitHub source checkout contain source, scripts, manifests, documentation, and the lockfile. They must not contain compiled bridge binaries, compiled `.app` bundles, Cargo build output, or release archives containing compiled artifacts. GitHub Releases are not used to publish compiled binaries for this product.

The user-owned build route is:

```sh
./scripts/materialize-macos.sh
```

That script is the sole materialization route. On macOS arm64 it builds the pinned Rust target `aarch64-apple-darwin`, compiles the Swift launcher, copies `Grok.md` into the launcher as `Contents/Resources/grok-codex-bridge-overlay.md`, signs the launcher, verifies an install-equivalent sanitized staging copy, and places both outputs under:

```text
dist/aarch64-apple-darwin/
├── grok-codex-bridge
└── Grok Codex Switch.app/
    └── Contents/Resources/grok-codex-bridge-overlay.md
```

The materialization script uses a private temporary directory for Cargo and Swift intermediates and removes it on exit. Cargo's output directory and the executable copy source are resolved together, independently of `CARGO_TARGET_DIR`. A pre-existing repository `target/` is neither read nor removed. `dist/` contains only the verified local materialized output; it is not authoritative source or a distribution payload.

The normal runtime never invokes Cargo, `rustc`, `swiftc`, a build-on-first-use path, an interpreted fallback, or a binary from `target/`. Missing or stale materialized output fails closed and instructs the user to run the materialization script.

## Install and update boundary

The repository-owned entry point is:

```sh
./scripts/grok-codex.sh grok
./scripts/grok-codex.sh native
```

Before installation or update, the user must run `./scripts/materialize-macos.sh`. The entry point verifies the local materialized pair, the current Native Codex catalog, the ChatGPT-authenticated Native upstream, and the Grok overlay before mutating bridge-owned state.

Installation copies the paired runtime and its resource into the per-user installed tree:

```text
~/Library/Application Support/grok-codex-bridge/
├── bin/grok-codex-bridge
├── bin/Grok Codex Switch.app/
│   └── Contents/Resources/grok-codex-bridge-overlay.md
├── config/bridge.toml
├── state/
└── logs/
```

The lifecycle manifest and user LaunchAgent record the bridge-owned paths. The installed launcher bundle must be a regular, non-symlink app bundle with a valid executable, Info.plist, and non-empty UTF-8 overlay snapshot. The installed bridge and launcher are replaced as a pair by `scripts/replace-installed-bridge.sh`; replacement stages both, stops the service, swaps both, verifies the new version and launcher signature, restarts the service, and attempts a bounded rollback if the operation fails. Direct and Grok-mode replacements run the new binary's `auth ensure` before service shutdown. A coordinator invocation explicitly targeting Native compatibility skips Grok credential access while preserving pair validation and rollback.

Source build/materialization and installed-runtime replacement are update operations. They are not normal mode switches and should be initiated from a Native GPT task or Terminal when the current Grok task depends on the bridge service.

The Native compatibility replacement also passes `--native-compatibility` to its final `doctor` invocation. This checks the runtime and service without resolving or reading Grok credentials, so a missing or expired Grok login cannot reject an otherwise valid Native escape update.

Updating from the repository therefore requires the source checkout containing the new materialized pair and replacement script. This is a deliberate source-install/update boundary. It does not make the repository a dependency of the already installed normal switching path.

## Installed normal mode switching

After installation, the checkout may be moved or deleted for ordinary operation. The direct installed entry points are:

```sh
BRIDGE="$HOME/Library/Application Support/grok-codex-bridge/bin/grok-codex-bridge"
"$BRIDGE" mode grok
"$BRIDGE" mode native
```

These commands use only the installed bridge, installed launcher, installed overlay snapshot, installed bridge state, the effective Codex home, and the live ChatGPT/Codex route explicitly inspected by the binary. They do not read the checkout's `Grok.md`, `dist/`, `target/`, or replacement scripts, and they do not compile.

`grok` publishes the merged Native GPT/Grok picker, routes admitted Grok models to xAI, and preserves Native GPT routing to the first-party Codex upstream. `native` publishes the Native compatibility mode while retaining the bridge provider metadata and hidden Grok catalog information required to open and continue saved Grok tasks. Neither direction rewrites the provider or model stored in a saved task; request-time compatibility transformation is the bridge's runtime responsibility.

Every mode switch is handed to the native launcher. The coordinator validates and, when required, replaces the paired runtime before requesting a graceful ChatGPT.app and bundled app-server shutdown. After quiescence it applies the rollback-owned picker transition. Successful mutation relaunches ChatGPT.app with the new state; failed picker mutation rolls back and attempts to restore the entry-time Desktop running state before returning the failure. The user-facing estimate is approximately 15–20 seconds. The user must not force-quit ChatGPT.app during this handoff.

## Native compatibility mode versus full uninstall

`mode native` is reversible Native-only operation. It is not uninstall and must not remove the provider definition, resolver, launcher, service, or compatibility metadata. A later `mode grok` must remain possible without rewriting task history.

Full uninstall is a separate explicit lifecycle action. It removes only manifest-proven bridge artifacts, stops/removes the bridge service, restores exact bridge-owned configuration backups, and does not convert historical task records. If saved tasks still refer to the bridge provider or Grok model, uninstalling first can make those tasks unopenable. Any permanent data migration must therefore be separately designed and explicitly completed before full uninstall.

## Ownership and invariants

- Codex owns the agent loop, permissions, tools, MCP servers, Skills, Browser/Computer Use, sessions, and Native GPT behavior.
- The bridge owns only the local provider boundary, Grok protocol translation, picker projection, service lifecycle, and reversible environment transition.
- `Grok.md` is reserved exclusively for the live Grok constitution source. No installed or auxiliary resource uses that filename for another purpose.
- The installed snapshot is named `grok-codex-bridge-overlay.md`; it is a materialized copy of `Grok.md`, not a second source of truth.
- Credentials, capability tokens, Codex state, Grok state, logs containing secrets, and user-specific absolute paths are never distribution artifacts.
- The listener remains loopback-only on `127.0.0.1` or `::1`.
- Normal switching never edits saved task records, SQLite history, rollouts, or the Native catalog source file.

## Acceptance bundle

For a source change affecting this route, the single acceptance bundle must establish:

1. `./scripts/materialize-macos.sh` produces both native outputs for macOS arm64 and copies the `Grok.md` bytes under the distinct installed snapshot name.
2. No compiled artifact is tracked or included as a repository distribution payload.
3. A fresh install copies the bridge, launcher, overlay snapshot, configuration, state, and lifecycle data into the installed tree without symlink substitution.
4. The installed bridge's direct `mode native` and `mode grok` entry points resolve only installed runtime inputs and do not compile or inspect the checkout.
5. The paired replacement path verifies, restarts, and rolls back the installed runtime as one unit when an update is required.
6. Native and Grok picker visibility and routing change in the requested direction while saved provider/model records remain unchanged.
7. A graceful desktop handoff completes before relaunch, with the user-facing estimate of approximately 15–20 seconds.
8. Full uninstall remains distinct from reversible Native compatibility mode and preserves the documented saved-task warning.
