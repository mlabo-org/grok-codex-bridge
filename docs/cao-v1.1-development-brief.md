# CAO V1.1 Development Brief

Status: `READY_FOR_SEPARATE_CAO_INTAKE`

This brief prepares a future Codex task. It does not activate V1.1 in the current V1.0 root thread, mutate the finalized V1.0 `.CAO` contract, edit live Codex configuration, install a new runtime, or begin implementation by itself.

## Objective

Implement `docs/spec-v0.1.md` Phase H through Phase J as V1.1 Native Picker Mode on top of the accepted V1.0 bridge. Native GPT and Grok must both be independently selectable from:

1. the Codex CLI model picker; and
2. the Codex Desktop GUI model picker.

Success on only one picker surface is not V1.1 completion.

## New CAO Identity

- task_id: `grok-codex-bridge-v1-1`
- epoch: `e1`
- state transition: `initialize-unrelated`
- scope: `V1.1 Native Picker Mode (Phases H-J) for both Codex CLI and Desktop GUI; V1.0 bridge retained as the foundation; public distribution excluded`
- delivery mode: `ITERATIVE_DELIVERY`
- state SSOT after intake: `.CAO/`
- product specification: `docs/spec-v0.1.md`
- implementation brief: `docs/cao-v1.1-development-brief.md`
- distribution boundary: `docs/distribution-contract.md`

`initialize-unrelated` is required because V1.0 is finalized under a different task ID and explicitly excludes V1.1. The future task must bind its own actual root thread ID. It must not use the current V1.0 root ID and must not use `continue-related`.

## Exact Start Request For The New Session

Use this request from the separate Codex task while its working directory is this repository:

> CAOで`grok-codex-bridge-v1-1` / epoch `e1`を`initialize-unrelated`として現在のroot taskへbindする。既存のfinalized V1.0 `.CAO` ledgerとhandoffを履歴として保持し、`docs/spec-v0.1.md`、`docs/cao-v1.1-development-brief.md`、`docs/distribution-contract.md`を引き継ぐ。scopeはV1.1 Native Picker ModeのPhase H〜Jで、Codex CLIとDesktop GUIの両モデルピッカーを必須経路とする。まずR0だけを開始し、current Codex source/runtimeとcodex-routerのpicker統合箇所を確定する。V1.0 bridgeを別実装へ分岐させず、public distribution、commit、push、publication、live config mutation、runtime activationは個別authorityなしに開始しない。

After intake, read `coding-agents context` before opening material work. Open only R0 with `begin-work`; do not pre-open every later responsibility.

## Accepted Product Decisions To Record After Intake

The new root records these as CAO decisions before material implementation:

### `D-v1-1-single-product`

- Decision: V1.0 and V1.1 are successive modes of one bridge product and one installed service, not separate repositories, protocol implementations, or independently maintained bridges.
- Impact: V1.1 reuses the accepted V1.0 protocol, credential, catalog, lifecycle, and service boundaries. The explicit V1.0 `grok-codex` route remains an active safe-provider entry without becoming a compatibility fork.
- Evidence: `file:docs/spec-v0.1.md;file:docs/distribution-contract.md`

### `D-v1-1-both-pickers`

- Decision: V1.1 must make native GPT and Grok independently selectable in both the Codex CLI model picker and Codex Desktop GUI model picker.
- Impact: CLI-only or Desktop-only visibility cannot satisfy acceptance. Each surface requires its own primary-path evidence unless current source proves and runtime confirms one shared producer.
- Evidence: `file:docs/spec-v0.1.md;file:docs/cao-v1.1-development-brief.md`

### `D-v1-1-picker-authority`

- Decision: Current authoritative Codex source and loaded runtime determine picker/catalog/config semantics. `duolahypercho/codex-router` is a bounded feasibility reference for the picker integration mechanism, not architecture authority.
- Impact: R0 may transfer the verified model visibility/config mechanism and rollback implications. It must not import LiteLLM/Chat multi-hop routing, namespace conversion, hosted-tool injection, or a fixed Grok registry.
- Evidence: `file:docs/cao-v1.1-development-brief.md;file:README.md`

### `D-v1-1-routing-separation`

- Decision: Native GPT and Grok remain strictly separated by verified model identity. Prefer a current Codex-owned per-model/provider route if it exists. Add transparent GPT passthrough only if authoritative Phase H evidence proves it is required for the shared picker design.
- Impact: GPT traffic never reaches xAI, Grok traffic never reaches OpenAI inference, and the bridge must not modify GPT request or response bytes when passthrough is necessary.
- Evidence: `file:docs/spec-v0.1.md`

### `D-v1-1-native-runtime`

- Decision: Normal runtime remains prebuilt native execution with no Cargo, source checkout, build-on-first-use, or interpreted fallback.
- Impact: A V1.1 runtime change invalidates the old materialized binary and requires the source-owned materialization route before any authorized runtime handoff. Public release packaging remains a separate task.
- Evidence: `file:AGENTS.md;file:docs/distribution-contract.md`

## Authority Order

1. Applicable system/developer/user instructions and `AGENTS.md` files.
2. `docs/spec-v0.1.md` for product intent, fixed Phase H-I-J order, and acceptance.
3. Current official Codex source, current loaded CLI/Desktop runtime, and current official Codex documentation for picker, catalog, provider, configuration, reload, and request routing facts.
4. Current official xAI Grok Build source for model and upstream facts already owned by V1.0.
5. The exact current `duolahypercho/codex-router` revision that demonstrates Desktop picker use, with the already pinned commit `9995c77278608640759982c98ec5bdaeb371c174` retained as a reproducible reference baseline.
6. Captured local fixtures only when the owning source cannot resolve a field.

Do not infer current picker behavior from README prose, past conversation, stale screenshots, cache contents, or V1.0 custom-profile success alone.

## Preserved V1.0 Baseline

V1.0 is an accepted implementation input, not a producer to rewrite routinely:

- direct Rust Responses-to-Responses bridge;
- loopback-only capability-scoped provider;
- read-only official Grok credential boundary;
- current official model catalog refresh with last-known-good preservation;
- validated text, reasoning, tool, image, and streaming handoffs;
- isolated `grok-bridge` CLI profile;
- reversible lifecycle and user LaunchAgent;
- prebuilt macOS arm64 runtime;
- live Grok 4.6 Codex shell-tool loop acceptance with native GPT base configuration preserved.

A concrete Phase H contradiction may return only the affected boundary to its owner. It does not authorize a rewrite of the accepted V1.0 protocol path.

## Responsibility Map

Each responsibility becomes one CAO `begin-work` transaction only when production starts. The root integrates it and records exactly one terminal result.

### R0 — Current picker contract investigation

- Inputs: current Codex CLI/Desktop source and runtime, current config/model catalog schemas, current `codex-router` picker implementation, V1.0 baseline.
- Complete output: source-backed facts identifying the exact producer and consumer for CLI model entries, Desktop model entries, provider selection, model metadata, config precedence, reload/restart behavior, and uninstall restoration.
- Required decision: whether CLI and Desktop share one producer; whether native Codex can route different models to different providers without routing GPT through the bridge; whether Phase I passthrough is actually required.
- Stop: any picker entry, native upstream, provider-selection, or rollback fact remains speculative.

### R1 — Phase H generated catalog and configuration design

- Inputs: completed R0 evidence and the existing reversible lifecycle.
- Complete output: one generated, bridge-owned catalog/config representation that preserves every native GPT entry and adds only admitted Grok entries; exact managed-state ownership; exact backup and rollback rules.
- Constraints: never edit the native catalog in place, never steal a visible GPT slug, never use a hard-coded fixed Grok ceiling, and never make one picker surface a proxy for testing the other.
- Consumer: routing and picker integration.

### R2 — Phase I native GPT route, only if required

- Inputs: R0 proof that the selected picker architecture requires the bridge to receive native GPT requests.
- Complete output: verified native upstream discovery and byte-preserving GPT request/stream passthrough with no xAI headers, prompt/body logging, Grok normalization, or credential crossover.
- Alternative completion: if current Codex supports a safe per-model/provider route, record Phase I as unnecessary with the authoritative evidence and do not add passthrough code.
- Stop: native upstream or authentication would need to be guessed or hard-coded.

### R3 — Phase J picker integration and lifecycle

- Inputs: R1 generated state and the R2 routing decision.
- Complete output: lifecycle-owned installation/update/uninstall of picker state, exact non-overwriting backups, comments/format preservation, CLI and Desktop Grok visibility, Native GPT preservation, service coordination, and complete rollback.
- Native alias is permitted only as the documented last resort after the current Desktop filter is proven to require it. It must not replace or hide a native GPT slug.

### R4 — V1.1 acceptance

- Inputs: integrated Phase H-J candidate and `docs/spec-v0.1.md` V1.1 criteria.
- Complete output: one admitted bundle proving independent CLI picker and Desktop picker selection, GPT/Grok upstream separation, unchanged GPT bytes where passthrough exists, continued Grok Codex-tool use, prebuilt direct runtime, and complete rollback.
- Live boundary: real config mutation, Desktop/CLI restart, service activation, OpenAI/xAI inference, and quota use require explicit current authority before execution.

## CAO Progress Items

After intake, create these durable progress items:

1. `grok-codex-bridge-v1-1.1` — R0 current picker contracts.
2. `grok-codex-bridge-v1-1.2` — Phase H generated catalog/config ownership.
3. `grok-codex-bridge-v1-1.3` — Phase I routing decision and implementation only if required.
4. `grok-codex-bridge-v1-1.4` — Phase J CLI and Desktop picker integration plus rollback.
5. `grok-codex-bridge-v1-1.5` — admitted V1.1 acceptance evidence.
6. `grok-codex-bridge-v1-1.6` — CAO coverage, finalization, doctor, and completed handoff.

## V1.1 Acceptance Contract

- Native GPT entries remain visible and selectable in both CLI and Desktop.
- Admitted Grok entries remain visible and selectable in both CLI and Desktop.
- Selection of a GPT model sends no inference traffic to xAI.
- Selection of a Grok model sends no inference traffic to OpenAI.
- GPT request and response bytes remain unchanged by the bridge wherever passthrough is used.
- Grok continues to use Codex-owned shell, MCP, tools, images, and available Computer Use paths.
- A future official Responses-backed Grok model can enter through bounded catalog refresh without a bridge source edit or fixed picker registry rewrite.
- Normal operation uses materialized native binaries directly and never Cargo or a source-relative launcher.
- Uninstall removes generated picker state and bridge-owned aliases, restores exact backups, and leaves native GPT/Codex configuration operational.

## Explicitly Excluded

- public distribution, GitHub release, Homebrew publication, license selection, signing, and notarization;
- a second bridge product or duplicated V1.0 implementation;
- codex-router's multi-hop router architecture;
- fixed Grok model lists as the long-term picker source;
- remote/LAN provider access;
- credential copying, interactive login automation, or refresh-token exchange; a bounded invocation of the official Grok CLI's non-interactive refresh path remains allowed;
- commit, push, publication, or live runtime/config activation without separate current authority.
