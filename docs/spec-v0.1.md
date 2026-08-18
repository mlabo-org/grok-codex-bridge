# Grok Codex Bridge for Rust — 実装仕様書 v0.1

Status: `ACTIVE_PRODUCT_SPEC`

この文書は、2026-08-18にChatGPT Web会話から取り込んだ原始仕様を、実装で参照可能なMarkdownへ正規化したものである。このrepoにおける製品意図、scope、責務境界、実装順序、受け入れ条件のSSOTとする。

外部製品のendpoint、schema、設定field、credential形式、ライセンスなど時間変化し得る事実は、この文書だけを根拠に実装しない。第70節の確認順序に従い、実装時点のauthoritative sourceで確認する。

現在のCAO taskが宣言する開発sliceは、V1.0 Safe Provider Mode（Phase A〜G）だけである。V1.1 Native Picker Mode（Phase H〜J）はこのsliceに含めない。

## 0. 目的

Codex Harnessをそのまま利用しながら、推論モデルとしてxAI Grokを選択可能にする最小構成のローカルRust bridgeを実装する。

```text
                   Codex Harness
                        │
          ┌─────────────┴─────────────┐
          │                           │
     Native GPT                  Grok Build
          │                           │
     OpenAI native              Local Rust Bridge
          │                           │
       OpenAI                  xAI CLI Chat Proxy
```

Grok CLIをCodexから外部toolとして呼ぶのではない。GrokをCodexの推論モデルとして動かし、Shell、filesystem、MCP、Skills、subagents、Browser、Computer Use、permissions、session/thread context、Codex tool loopをCodex Harnessから利用できる状態を目指す。

BridgeはComputer Useやtool executionを実装しない。CodexとGrokの推論protocolだけを橋渡しする。

## 1. 最重要設計原則

### 1.1 GPTを壊さない

Grok追加のために既存GPT環境を犠牲にしない。Native GPT、ChatGPT login、Codex settings、MCP、Skills、projects、permissionsへV1.0 bridgeを介入させない。Grokを削除した場合、`Codex + GPT`が導入前と同じ状態へ戻ること。

### 1.2 Grok専用

V1ではClaude、Gemini、DeepSeek、Kimi、Ollama、LM Studio、OpenRouter、任意OpenAI-compatible provider、provider registry、provider marketplaceを実装しない。これは汎用LLM routerではなく、`Codex ⇄ Grok`専用bridgeである。

### 1.3 BridgeはAgent Harnessではない

Agent loopの所有者はCodexである。

```text
Grok tool request
  → Bridge Responses function call
  → Codex Harness tool execution
  → Bridge tool result translation
  → Grok
```

Bridgeはshell command、MCP、filesystem、Browser、Computer Use、permission decisionを実行しない。

## 2. 外部仕様として確認すべき前提

実装前に次をcurrent authoritative sourceで確認する。

- Codex custom model providerが`base_url`、`wire_api = "responses"`、authentication、HTTP headersをどの形で受け付けるか。
- Grok BuildのOAuth credential sourceとpermission contract。
- xAI公式Grok Buildが公開するCLI Chat Proxy direct-call contract。
- current upstream `https://cli-chat-proxy.grok.com/v1/responses`。
- current model catalog `https://cli-chat-proxy.grok.com/v1/models`。
- current headersとrequest schemaは`docs/r0-external-contracts.md`に固定する。

非公開endpoint探索やclient fingerprint偽装を行わない。

## 3. 実装フェーズ

### V1.0 Safe Provider Mode

GrokをCodex custom providerとして動かし、Native GPTの推論とresponseは変更しない。V1.1の共通picker reverse routeでは、`store: false`で再利用不能なGrok由来reasoning itemだけをNative GPT requestから除外する。通常のCodex Desktop model picker統合は必須にせず、明示したGrok profileから選択できればよい。

```text
Native GPT task → bridge（Grok reasoning分離のみ）→ OpenAI

Grok task → Codex custom provider → 127.0.0.1 bridge → xAI
```

### V1.1 Native Picker Mode

V1.0安定後の別slice。GPTとGrokをCodex CLIのmodel pickerとCodex Desktop GUIのmodel pickerの両方から選択できる状態を目指す。片方だけのpicker表示または選択成功ではV1.1完成としない。CLIとDesktopが同じcatalog/config producerを使うかはcurrent authoritative sourceと実runtimeで確認し、共有を推測しない。Desktop UIのcustom model filterやcatalog制約があり得るため、V1.0から分離する。

## 4. V1.0 Architecture

```text
┌─────────────────────────────────────────────┐
│ Codex                                       │
│ Agent loop / Permissions / Shell / MCP      │
│ Skills / Computer Use / Browser             │
└──────────────────┬──────────────────────────┘
                   │ Responses API
                   ▼
        loopback capability-scoped endpoint
                   │
┌──────────────────▼──────────────────────────┐
│ grok-codex-bridge                           │
│ Responses parser / normalized conversation │
│ Tool schema translator / stream state      │
│ OAuth credential reader / xAI client       │
└──────────────────┬──────────────────────────┘
                   │ Responses API
                   ▼
          verified xAI CLI proxy
```

## 5. 使用技術

Rust stableを使用する。依存候補は`tokio`、`axum`、`reqwest`、`serde`、`serde_json`、`toml`、`thiserror`、`anyhow`、`tracing`、`tracing-subscriber`、`clap`、`uuid`、`bytes`、`futures-util`、`eventsource-stream`、`secrecy`、`zeroize`。必要性を確認したcrateだけを追加し、TLSは可能ならrustlsを使う。

## 6. Binary

通常runtimeは単一のprebuilt binary `grok-codex-bridge`とする。Node.js、Python、uv、Docker、Cargoを通常runtime dependencyにしない。

## 7. Source構造

巨大な`main.rs`を作らない。責務が実装される時点で、CLI、config、server、router、error、Codex protocol、Grok transport/auth、translation、install lifecycleを独立moduleへ割り当てる。空moduleや将来用placeholderを先に量産しない。

## 8. Local Server

初期候補は`127.0.0.1:4545`。V1は`0.0.0.0`、`::`、LAN、remote accessを実装しない。IPv6を提供する場合も`::1`だけを許可する。

## 9. Local Caller Authentication

localhostを無認証にしない。install時にランダムcapability tokenを生成し、permission `0600`で保存する。endpointは次のようにcapability pathを含める。

```text
http://127.0.0.1:4545/_grok/<random-capability>/v1
```

capability token、完全なcapability URLをlogへ出さない。

## 10. Grok Credential

候補sourceは`$GROK_HOME/auth.json`、未設定時は`~/.grok/auth.json`。current official contractで確認してから実装する。credentialはread-onlyでin-place参照し、repo、Codex config、log、SQLite、JSON cache、environment dumpへ複製しない。

## 11. Credential Cache

Access tokenのmemory cacheは許容する。disk再保存、debug print、panic dumpは禁止する。secret保持型を使い、process exit時は可能な範囲でzeroizeする。credential fileのmtime変化を検出して再読込する。

## 12. OAuth Refresh

V1は独自OAuth refresh、browser OAuth、client identity再実装を行わない。401/403は、公式Grok login経路を再実行してretryする必要があることを明示errorとして返す。login commandの正確な形はcurrent official CLIで確認する。

## 13. xAI Upstream

V1はcurrent xAI公式Grok Buildが公開するResponses endpointと公式sourceで確認したheadersだけを使用する。bootstrap catalogはR0で確認したcurrent Responses modelsを含み、credentialed runtimeでは同じoriginの公式`/v1/models`から更新する。third-party routerの別model実装はreferenceに留める。

## 14. Codex-facing API

最低限の論理endpointは次の3つ。

```text
GET  /healthz
GET  /v1/models
POST /v1/responses
```

実runtimeではすべてcapability pathの下へ配置する。

## 15. `/healthz`

Credentialを読まず、upstream通信もしない。responseはservice/versionだけを返し、secret、username、home path、token有無を含めない。

```json
{"status":"ok","service":"grok-codex-bridge","version":"0.1.0"}
```

## 16. `/v1/models`

V1はadmitted model catalogを返す。source snapshot由来のbootstrap catalogを持ち、Phase B以降は公式session endpointから一回のbounded startup refreshまたは明示`catalog refresh`で更新できる。

```json
{"object":"list","data":[{"id":"grok-4.6","object":"model","owned_by":"xai"},{"id":"grok-4.5","object":"model","owned_by":"xai"}]}
```

refreshはResponses backend、non-empty model ID、xAI allowlisted inference originを満たすentryだけを採用する。empty、malformed、unauthorized、legacy-protocol、alternate-origin responseではlast-known-good catalogを置換しない。新しい公式modelは成功したrefresh後に利用可能にし、unknown model requestは明示errorにする。unattended polling loopとNative Codex picker統合はV1.0へ含めない。

## 17. Codex Responses API

Principal endpointは`POST /v1/responses`。処理は`Codex Responses request → strict typed validation / internal normalized representation → xAI Responses request`とする。current両端がResponses contractを使用するため、legacy Chat Completionsへの変換は行わない。

## 18. Internal Normalized Representation

Codex schemaとxAI schemaを無検証で透過させず、instructions、messages、tools、tool choice、model、content parts、tool calls、tool resultsを表す内部型を置く。TextとImageを区別し、assistantの複数tool callとtool resultの`call_id`を保持する。内部型はlossless validation/canonicalizationを所有し、別protocolを合成しない。

## 19. Instructions

Responsesの`instructions`は同じResponses fieldとして損失なく維持する。Codexから来たinstructionsを要約、再生成、書換えしない。

## 20. Text Message

Responses input messageのrole、content order、`input_text`を対応するResponses itemとして損失なく維持する。Codex assistant履歴の`phase`は`commentary`または`final_answer`として検証し、phaseを持たないxAI easy message境界で終端する。未対応content typeをsilent dropしない。

## 21. Images

`input_image`、image URL、`data:image/...;base64,...`、tool result imageを扱う。可能な限りdecode/re-encodeせず転送する。V1で対応不能なimage形式は黙って削除せず明示unsupported errorを返す。

## 22. Tool Definition

Codexから渡されたfunction toolのname、description、parameters schemaを内部型では損失なく維持する。Grok providerへの投影だけは、xAIがrequest全体を拒否するroot unionまたはnullable object schemaをplain object rootへ限定的に展開し、宣言型と矛盾する`enum`/`const` literalを除去する。これはCodex app toolの実行時argument validationを置換せず、Native GPT経路のschemaは書換えない。name変更、description要約を行わない。

## 23. Tool Choice

最低限`auto`、`none`、`required`、specific functionを維持する。

## 24. Tool Calls

Grokが返すResponses function call itemのtool name、tool call ID、arguments JSONを検証してそのまま維持する。`call_id`は次turnのtool result対応に必要なため失わない。

Native GPTで完了済みの`custom_tool_call`と`custom_tool_call_output`はCodex harnessが所有するforeign実行履歴としてschemaを検証し、Grok requestから除外する。これらをGrok function call stateへ混入させず、同じturnのassistant message textは会話履歴として維持する。

## 25. Tool Result

Responsesの`function_call_output`を、同一`call_id`を持つxAI Responses input itemとして維持する。textとimage resultを保持する。

## 26. Parallel Tool Calls

1turnの複数tool callを順序付きで保持する。単一tool call前提にしない。

## 27. Streaming

Codex側へResponses-compatible SSEを返す。upstream eventをstrictにparseし、最低限、次のlifecycle/event familyを正しい順序とschemaで維持する。

- `response.created`
- `response.output_item.added`
- `response.content_part.added`
- `response.output_text.delta`
- `response.output_text.done`
- `response.content_part.done`
- `response.function_call_arguments.delta`
- `response.function_call_arguments.done`
- `response.output_item.done`
- `response.completed`

## 28. Stream State Machine

SSE translationを場当たり的な条件分岐にしない。Created、OutputItemStarted、ContentStarted、Streaming、ContentDone、OutputItemDone、Completedを明示状態として持つ。tool callは独立output itemとして管理する。

## 29. Stable IDs

Bridge生成IDは`resp_<uuid>`、`msg_<uuid>`、`fc_<uuid>`等とし、同じresponse/stream内で安定させる。`sequence_number`は単調増加させる。

## 30. Reasoning

V1はhidden chain-of-thoughtを生成、推測、偽造しない。Grokが返したopaqueな暗号状態はbridge provenance envelopeで包み、Grokへ戻すときだけ復元する。Native GPTへはreasoning itemごと渡さない。公開可能なreasoning summaryだけをCodex表示へ維持する。

## 31. Hosted Search

V1はxAI hosted `web_search`、`x_search`を自動注入しない。Codex Harnessから提供されたtoolsを優先する。

## 32. Computer Use

BridgeにComputer Use固有codeを実装しない。必要なのはtool schema、tool call、screenshot/image、tool resultの保存である。期待flowは`Grok → Codex tool call → Codex Computer Use → result → Bridge → Grok`。

## 33. V1.0 Codex Configuration

Candidate configはcustom provider `base_url`、`wire_api = "responses"`、`supports_websockets = false`、`requires_openai_auth = false`と、Grok専用profileを組み合わせる。file location、profile syntax、field semanticsはcurrent Codex docs/CLIで確認してから生成する。

## 34. V1.0の意味

通常Codex→Native GPT→OpenAI経路へbridgeを入れない。明示Grok profileの時だけCodex→bridge→xAIへrouteする。まずこの経路を完全に成立させる。

## 35. V1.1 Native Picker Mode

V1.0完了後の別task。必要に応じて`openai_base_url`、merged model catalog等を調査するが、current V1.0 sourceへpassthrough routeを先に入れない。

## 36. V1.1 Routing

将来はrequestのmodel fieldだけでGrok routeとNative GPT passthroughを分ける。これはV1.1 scopeであり、V1.0では実装しない。

## 37. Native GPT Passthrough

V1.1で必要になった場合、GPT requestのsystem prompt、model、tools、JSON bytes、tool resultsを変更せず、xAI headerを付けず、prompt/bodyをlogしない。Response streamも透過させる。

## 38. Native Upstream Discovery

Native OpenAI upstreamを推測でhard-codeしない。既存Codex config/default behaviorを確認・保存できなければV1.1 installationをfail closedにする。

## 39. Merged Model Catalog

V1.1ではnative catalogを直接編集せず、copyを基にGrok entryを加えたgenerated catalogをruntime stateへ置く。Desktop表示は実機確認する。

## 40. Native Alias

Desktop制約で必要な場合だけV1.1最後の手段として検討する。visible GPT modelやactive slugを奪わず、mappingを明示保存し、uninstallで完全削除する。

## 41. Config Backup

Codex configを編集する前にtimestamp付き非上書きbackupを作る。これはinstaller実装時にexact targetと復旧経路を確認してから行う。

## 42. Config Editing

File全体をserializerで再生成せず、comments、formatting、custom sectionsを保持する。bridge-owned managed marker blockだけを編集する。

## 43. Atomic Write

Config更新はread、validate、temp write、fsync、atomic renameで行い、途中failureで破損configを残さない。

## 44. Runtime State

Runtime state候補はconfig、caller-token、generated catalog、backups、metadata-only logs。Grok credentialをここへコピーしない。実pathはinstaller設計時に確認する。

## 45. Logging Policy

DefaultはINFO。timestamp、request ID、route、model、HTTP status、duration、stream completion、error classだけを許可する。prompt、response body、tool arguments、tool results、OAuth token、Authorization header、caller token、repository contents、screenshotsをlogしない。

## 46. Debug Mode

`RUST_LOG=debug`でもsecretやrequest bodyを出さない。protocol diagnosticsはmetadataに限定する。

## 47. Credential Boundary

OpenAI credentialはOpenAIだけ、Grok credentialは検証済みxAI hostだけへ送る。module/type境界でcredentialの誤送信を困難にする。

## 48. Upstream Allowlist

Grok credential送信先hostnameをcurrent official CLI proxy hostへallowlistする。redirectで別hostへAuthorizationを転送しない。redirectはdisabledまたはsame-originに制限する。

## 49. No Stealth

Fingerprint spoofing、device spoofing、rate-limit evasion、account rotation、token farming、automated subscription creation、OAuth interception、TLS interception、hidden proxy fallbackを実装しない。

## 50. CLI Commands

V1完成時の最低限候補：`run`、`install`、`uninstall`、`status`、`doctor`、`auth status`、`catalog refresh`、`service install`、`service uninstall`。`catalog refresh`は公式session `/v1/models`からatomicにlast-known-good catalogを更新し、credentialやNative GPT catalogを変更しない。Phase ownershipと実装時点の必要性に合わせて追加する。

## 51. `doctor`

Defaultはnetwork quotaを消費せず、binary、bind、caller capability、credential presence/permission/schema、Codex config parse、backup、provider config、service stateを確認する。secret valueは表示しない。

## 52. Live Smoke Test

`doctor --live`等の明示指定時だけreal inferenceを実行し、quota消費を事前表示する。最小prompt候補は`Reply with exactly: BRIDGE_OK`。

## 53. Error Mapping

401/403、429、5xx、translation failureを区別する。認証failureは公式login経路、rate limitはsubscription limit、5xxはtemporary unavailableとして返す。未対応Responses item typeはtypeを示す明示errorとし、silent dropしない。

## 54. Timeout

初期候補はconnect timeout 10秒、stream idle timeout 300秒。長いagent turnを通常request timeoutで一律に切らない。値は実測とcurrent upstream contractで確定する。

## 55. Retry

Connect failure、502、503等だけをlimited retry候補とする。upstream到達が不明なtool-call inference requestを無条件再送せず、duplicate execution riskを避ける。

## 56. Tests

Unit/integration testはupstream mockを使い、CIでxAI quotaを消費しない。fixtureはsimple text、streamed text、single/parallel tool calls、tool result、image input/result、malformed tool JSON、401、429、5xx、stream interruptionを含む。

## 57. Tool Loop Integration Test

Mock Grokがshell/read-file等のtool callを返し、Codexがtoolを実行し、次turnで結果がbridgeを通ってmodelへ戻ることを確認する。Bridge自身がtoolを実行していないことも確認する。

## 58. Real Grok Tool Test

Scratch repositoryと固有markerを使い、GrokがCodex toolsでmarkerを発見できることを明示live testで確認する。推測回答を成功としない。

## 59. Computer Use Acceptance Test

Codex runtimeでComputer Useが利用可能な環境だけで、安全なlocal app observationを行う。成功条件はGrok→tool selection→Codex Computer Use→screenshot/result→Grok→final answer。BridgeにComputer Use codeが存在しないこと。

## 60. GPT Regression Test

V1.1で最重要。導入前後でNative GPT model/login/MCP/Skills/approval/Computer Use/repo access/streamingが保たれ、request/response bytesがbridgeで変更されないことを確認する。V1.0ではGPT config非変更を確認する。

## 61. Kill Switch

`GROK_CODEX_BRIDGE_DISABLE=1`でGrok routeを拒否できるようにする。V1.1ではNative GPT passthroughを維持してよい。

## 62. Uninstall

Service stop、managed config removal、original config restoration、generated catalog removal、caller token removalを所有する。Grok credential、ChatGPT credentials、Codex conversations、projects、MCP、Skills、`AGENTS.md`へ触らない。

## 63. launchd

macOS first。LaunchAgentはuser domainに置き、loopbackだけへbindし、crash restartを許容できる。stdout/stderrへsecretを出さない。source buildとinstalled runtimeは別境界とする。

## 64. V1 Non-goals

GUI、menu bar app、automatic updater、provider plugins、cloud sync、remote/LAN access、OAuth login UI、billing/quota dashboard、prompt history、database、web administrationを実装しない。

## 65. 参考source

1. OpenAI Codex official repository/documentation: Responses contract、custom provider、model catalog、config semantics、tool loop。
2. xAI Grok Build official source: auth handling、Responses schema、model catalog、tool calls、images、stream parser、CLI proxy contract。
3. `codex-router`: Grok OAuth route、Responses compatibility、config backup、caller auth、coexistence、rollbackのreference。architectureを移植しない。

## 66. Licensing

Reference implementationのcodeをcopy/modifyする前にlicenseとnoticeを確認し、必要なattributionを保持する。設計思想とprotocol behaviorだけを参考にし、自前実装を優先する。Repo全体の公開licenseはユーザーが別途選択するまで未決定。

## 67. 実装順序

順序を変更しない。

- Phase A: Rust project skeleton、HTTP server、healthz、config、logging。
- Phase B: credential parser、xAI client、official model catalog refresh、simple request、xAI SSE parser。
- Phase C: Responses input parser、lossless text normalization、Responses SSE validation。
- Phase D: function tools、tool calls、tool results、parallel calls。
- Phase E: image input/result、Computer Use-compatible image preservation。
- Phase F: Safe Provider installer、doctor、uninstall、launchd。
- Phase G: real Codex + Grok live test。ここでV1.0完成。
- Phase H: native model catalog investigation。
- Phase I: GPT transparent passthrough。
- Phase J: merged picker。ここでV1.1完成。

現在のCAO scopeはPhase A〜Gだけである。

## 68. V1.0 Acceptance Criteria

- [ ] Single prebuilt Rust binaryで通常動作する。
- [ ] Loopback only。
- [ ] Local caller authenticationがある。
- [ ] Credential fileをread-onlyで扱い、OAuth credentialを複製しない。
- [ ] Current xAI official CLI proxy contractだけを使う。
- [ ] 公式model catalogをbounded refreshでき、failure時にlast-known-goodを保持する。
- [ ] Simple textとstreamingが動く。
- [ ] Function call、tool result、parallel toolsが動く。
- [ ] Image inputとtool-result imageが動く。
- [ ] GrokからCodex shell toolとMCPを使える。
- [ ] 利用可能環境でComputer Use protocol pathが動く。
- [ ] Native GPT configurationを変更しない。
- [ ] Uninstallでbridge-owned変更を戻せる。
- [ ] Prompt、response、tool content、credentialをlogしない。

## 69. V1.1 Acceptance Criteria

V1.0とは別taskで扱う。

- [ ] Native GPT modelsが残る。
- [ ] Native GPTとGrokをCodex CLI model pickerでそれぞれ独立選択できる。
- [ ] Native GPTとGrokをCodex Desktop GUI model pickerでそれぞれ独立選択できる。
- [ ] GPT trafficがxAIへ行かず、Grok trafficがOpenAI inferenceへ行かない。
- [ ] GPT request/response streamを変更しない。
- [ ] GrokがCodex toolsとComputer Useを利用し続ける。
- [ ] Complete rollbackが動く。

## 70. Implementation Rule for Codex

Scopeを拡張しない。不明なprotocol fieldは次の順序で確認する。

1. Current official Codex source/documentation/runtime schema。
2. Current official Grok Build source/documentation。
3. `codex-router` reference implementation。
4. 実際に取得したlocal fixture。

推測実装、silent drop、未確認fallbackは禁止する。対応不能なfieldは明示errorにする。

## 71. 最終設計思想

これは新しいAI AgentやGrok CLI wrapperを作るprojectではない。

```text
Grok + Codex Harness

Model  = 推論
Codex  = 身体
Bridge = 神経接続
```

Bridgeは小さく、単純で、監査可能で、削除すれば元に戻るものにする。まずV1.0でGrok→Codex tool loop→Computer Useまで成立させ、その後に限ってV1.1 picker統合を検討する。
