# Codex Project Handoff

Status: `READY_TO_CONTINUE`

## Project

- source repo: `/Users/suzukimakoto/Desktop/git/current-work/codex-infrastructure/grok-codex-bridge`
- product: standalone native Rust provider bridge, not a Codex plugin
- goal: use Grok as the inference model while retaining the Codex Harness as the agent body
- intended future publication: GitHub, after the implementation is proven useful

## What is complete

- Independent Git repository initialized on branch `main`; no commit or remote exists yet.
- Rust 1.95.0 package, library, binary, focused CLI tests, and macOS arm64 materialization route exist.
- The materialized binary runs directly and reports that the protocol server is not implemented.
- Six Rust tests passed in the development-foundation acceptance bundle.
- The original 71-section design was imported and normalized into `docs/spec-v0.1.md`.
- `docs/cao-v1.0-development-brief.md` defines V1.0 responsibilities R0〜R6 and the fixed Phase A〜G order.
- Local `AGENTS.md` fixes the responsibility, source/runtime, security, verification, and publication boundaries.
- CAO 0.3.0 was verified from the loaded cache; its CLI bytes matched the authoritative plugin source.
- `.CAO/` was initialized and is excluded locally through `.git/info/exclude`.

## Active CAO continuation identity

- task_id: `grok-codex-bridge-v1-0`
- epoch: `e1`
- scope: `V1.0 Safe Provider Mode (Phases A-G) in this repository; V1.1 excluded`
- delivery mode: `ITERATIVE_DELIVERY`
- current state: preparation complete; V1.0 implementation not started

The current `.CAO` root binding belongs to the preparation task. A newly created Codex project/task has a different root thread ID and must not silently consume the old binding.

In the new Codex project, explicitly request:

> CAOで`grok-codex-bridge-v1-0` / epoch `e1`を`continue-related`として現在のroot taskへ再bindし、既存`.CAO`、`docs/spec-v0.1.md`、`docs/cao-v1.0-development-brief.md`を引き継いでR0から開始する。scopeはV1.0 Phase A〜Gのみ。V1.1は開始しない。

The CAO continuation must preserve the same task ID, epoch, and scope while replacing only `root_thread_id` with the new Codex task ID. It must read CAO `context` immediately after intake.

## Read order in the new project

1. Applicable ancestor and repo-local `AGENTS.md` files.
2. CAO `context` and `.CAO/handoff.md`.
3. `docs/spec-v0.1.md`.
4. `docs/cao-v1.0-development-brief.md`.
5. `README.md`, `Cargo.toml`, current source, and current Git status.

## First work after continuation

Begin only R0 — External contract verification.

Resolve, with current authoritative evidence:

- Codex custom provider fields and profile/config location.
- Actual Codex Responses request, tool, image, and SSE event contracts needed by V1.0.
- Grok Build credential schema, permission behavior, official upstream endpoint, required headers, model ID, image/tool schema, and streaming contract.
- Reference repository licenses before reading implementation code for reuse.

Do not add networking, auth, or protocol dependencies until the facts needed by the affected Phase A/B/C boundary are resolved. Do not create empty placeholder modules for later phases.

## Work still pending

- R0 external contract verification.
- Phase A core HTTP/config/logging foundation.
- Phase B Grok credential and transport.
- Phase C Responses text and streaming translation.
- Phase D tools, tool results, and parallel calls.
- Phase E image preservation and Computer Use-compatible result path.
- Phase F reversible Codex Safe Provider install/doctor/uninstall/launchd.
- Phase G explicit real Codex + Grok V1.0 acceptance.
- V1.0 CAO verification coverage, finalization, doctor, and completed handoff.

V1.1 native picker, GPT passthrough, merged catalog, and aliases are deliberately absent and must remain a separate task.

## Authority and safety boundaries

- Product scope, order, and acceptance: `docs/spec-v0.1.md`.
- External protocol facts: current official Codex, then current official Grok Build, then licensed reference, then captured local fixture.
- Bridge never executes Codex tools and never owns Computer Use.
- Native GPT remains untouched during V1.0.
- Credentials remain read-only and must not enter the repo, Codex state, logs, caches, or test fixtures.
- Live inference, account authentication, Codex config mutation, service activation, Git commit, remote creation, push, publication, and license selection require their own current authority.

## Working-tree state

The repository is intentionally uncommitted. All source files are new. `.CAO/`, `target/`, and `dist/` are local ignored state/artifacts. Preserve them during continuation; do not stage `.CAO`, build caches, materialized binaries, secrets, or user-specific runtime files.

