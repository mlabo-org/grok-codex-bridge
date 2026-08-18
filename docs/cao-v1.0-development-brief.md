# CAO V1.0 Development Brief

Status: `READY_FOR_NATIVE_CODEX_EXECUTION`

## Objective

`docs/spec-v0.1.md`のPhase A〜Gを順番どおり実装し、GrokをCodex Harnessの推論モデルとして利用できるV1.0 Safe Provider Modeを完成させる。Native GPT経路へ介入せず、Bridgeはprotocol handoffだけを所有する。

## Motivation

Codex Harnessのtools、permissions、MCP、Skills、Browser、Computer Use、session continuityを保ったまま、高性能なGrokを推論modelとして使い倒す。

## Active CAO identity

- task_id: `grok-codex-bridge-v1-0`
- epoch: `e1`
- scope: `V1.0 Safe Provider Mode (Phases A-G) in this repository; V1.1 excluded`
- delivery mode: `ITERATIVE_DELIVERY`
- state SSOT: `.CAO/`
- source specification: `docs/spec-v0.1.md`

Native Codex owns decomposition、official subagents、model/reasoning selection、live supervision、integration、semantic acceptance。CAO owns durable identity、work transactions、decisions、progress、typed evidence、finalization、handoff。CAOはworkerを起動しない。

## Authority order

1. Applicable system/developer/user instructions and `AGENTS.md` files。
2. `docs/spec-v0.1.md` for product intent、scope、order、acceptance。
3. Current official Codex source/docs/runtime schema for Codex protocol facts。
4. Current official xAI Grok Build source/docs for Grok protocol/auth facts。
5. `codex-router` as a licensed reference, never as architecture authority。
6. Captured local fixtures when official sources do not resolve a field。

README、past conversation、cache、runtime outputはprotocol authorityではない。

## Declared V1.0 slice

The implementation sequence is fixed:

1. Phase A — core service foundation。
2. Phase B — verified Grok auth/upstream transport and official model catalog refresh。
3. Phase C — text Responses normalization and stream state machine。
4. Phase D — function tools and parallel tool loop preservation。
5. Phase E — image preservation for Computer Use paths。
6. Phase F — Safe Provider lifecycle: install、doctor、uninstall、launchd。
7. Phase G — explicit live V1.0 acceptance in a scratch environment。

V1.1 model picker、Native GPT passthrough、merged catalog、native aliasは別taskであり、このscopeへ入れない。

## Responsibility map

Each item becomes a CAO `begin-work` transaction only when its production starts. The parent records exactly one terminal result after integration.

### R0 — External contract verification

- Inputs: current official Codex source/docs/runtime schema、current Grok Build source/docs、reference licenses。
- Complete output: source-backed contract notes or fixtures for provider config、Responses request/stream、Grok auth/upstream、official model catalog、tool/image schema。
- Consumer: Phase A〜E implementers。
- Stop: any required protocol or credential fact remains speculative。

### R1 — Core service and configuration

- Inputs: Phase A requirements and R0 evidence。
- Complete output: loopback-only server、capability-path routing、healthz、configuration types、metadata-only logging、prebuilt binary materialization。
- Consumer: all later phases。

### R2 — Grok credential and transport

- Inputs: verified Grok contract and security allowlist。
- Complete output: read-only credential access、memory-only secret handling、safe redirect policy、xAI request/SSE transport、bounded official model catalog refresh with last-known-good preservation、typed errors。
- Consumer: translation layers。

### R3 — Codex protocol normalization and streaming

- Inputs: verified Codex Responses contract and R2 transport types。
- Complete output: normalized conversation types、lossless text/instructions、strict Responses event preservation、stable IDs where bridge generation is required、monotonic sequence、explicit stream state machine。
- Consumer: tool/image integration and Codex runtime。

### R4 — Tools and images

- Inputs: R3 normalized types and verified xAI schemas。
- Complete output: tool definition/choice/calls/results、parallel call ordering、image URL/data URI/tool-result images、unsupported errors without silent loss。
- Consumer: Codex tool loop and Computer Use path。

### R5 — Safe Provider lifecycle

- Inputs: working bridge binary and current Codex config semantics。
- Complete output: reversible install、non-overwriting backup、managed config block、atomic writes、doctor、kill switch、uninstall、LaunchAgent materialization。
- Consumer: real V1.0 use。
- Stop: existing config cannot be preserved or rollback cannot be proven。

### R6 — V1.0 acceptance

- Inputs: integrated Phase A〜F candidate and `docs/spec-v0.1.md` §68。
- Complete output: one admitted acceptance bundle covering mock protocol paths、direct prebuilt binary、Safe Provider tool loop、explicit quota-bearing live tests、rollback and GPT non-interference evidence。
- Consumer: root acceptance and CAO finalization。

## First execution step

Start R0 before adding networking/auth/translation dependencies. Record current source references and exact schema/field evidence. Then start Phase A only after the necessary provider endpoint and Responses boundary are resolved. Do not create placeholder module trees for unstarted phases.

## Verification topology

- Seal the minimum semantic acceptance bundle before each production slice。
- Use focused mock tests for protocol transformations and failure classes。
- Materialize the affected release binary and execute it directly when runnable behavior changes。
- Real xAI inference、real tool loop、Computer Use smoke、Codex config mutation、LaunchAgent activationはそれぞれ明示authorityと安全なscratch boundaryを必要とする。
- Success ends verification; do not add reviewer loops or broad matrices after a passing bundle。

## Forbidden actions

- Native GPT traffic interception during V1.0。
- Credential copy、prompt/body/tool/screenshot logging、secret-bearing debug output。
- Bridge-owned tool execution or Computer Use implementation。
- Unverified endpoint/header/schema guesses、silent drop、hidden fallback。
- V1.1 work、GUI、provider abstraction、remote/LAN access。
- Commit、remote creation、push、publication、license selection、runtime installation/activation without separate authority。

## Resume

For a new Codex project/task, follow `docs/codex-project-handoff.md`: explicitly continue the same task ID and epoch with `state-transition=continue-related`, rebind only the root thread ID, then read CAO `context`, `docs/spec-v0.1.md`, and this brief. Open only the next independently meaningful responsibility with `begin-work`; do not pre-open every phase. Preserve exact task identity and scope on every CAO write.
