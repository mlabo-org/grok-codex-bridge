# Codex V1.1 Session Handoff

Status: `READY_FOR_NEW_ROOT_TASK`

## Why this is a new CAO task

The completed V1.0 state is bound to task `grok-codex-bridge-v1-0`, epoch `e1`, and a finalized root thread retained only in local runtime state. Its scope explicitly excludes V1.1 and its handoff is finalized. Do not reopen, overwrite, or rebind it for Phase H-J.

V1.1 starts in a separate Codex task with a new CAO identity:

- task_id: `grok-codex-bridge-v1-1`
- epoch: `e1`
- state transition: `initialize-unrelated`
- root_thread_id: the actual new Codex task ID
- scope: `V1.1 Native Picker Mode (Phases H-J) for both Codex CLI and Desktop GUI; V1.0 bridge retained as the foundation; public distribution excluded`

## Paste into the new task

> CAOで`grok-codex-bridge-v1-1` / epoch `e1`を`initialize-unrelated`として現在のroot taskへbindする。既存のfinalized V1.0 `.CAO` ledgerとhandoffを履歴として保持し、`docs/spec-v0.1.md`、`docs/cao-v1.1-development-brief.md`、`docs/distribution-contract.md`を引き継ぐ。scopeはV1.1 Native Picker ModeのPhase H〜Jで、Codex CLIとDesktop GUIの両モデルピッカーを必須経路とする。まずR0だけを開始し、current Codex source/runtimeとcodex-routerのpicker統合箇所を確定する。V1.0 bridgeを別実装へ分岐させず、public distribution、commit、push、publication、live config mutation、runtime activationは個別authorityなしに開始しない。

## Required read order

1. Applicable ancestor and repository `AGENTS.md` files.
2. The finalized V1.0 CAO handoff and semantic ledger as historical evidence.
3. `docs/spec-v0.1.md`.
4. `docs/cao-v1.1-development-brief.md`.
5. `docs/distribution-contract.md`.
6. `README.md`, `Cargo.toml`, current source, current materialized/installed runtime state, and one current dirty-state check.

After the new `intake`, read CAO `context`, record the decisions and progress items named in the V1.1 brief, and open only R0. Do not begin Phase H source edits until R0 has resolved the shared-or-separate CLI/Desktop producer and the native GPT routing decision.
