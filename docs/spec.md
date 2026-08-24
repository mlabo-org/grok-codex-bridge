# Grok Codex Bridge 製品仕様

Status: `ACTIVE_PRODUCT_SPEC`

この文書は、現在の `grok-codex-bridge` 実装に対応する製品仕様である。Rust bridge、Swift launcher、merged picker、インストール、更新、日常の環境切り替え、完全アンインストールの責務と受け入れ条件を定義する。実装とこの文書が衝突する場合、実行可能な正本を確認してから両者を同じ変更で修正する。

## 1. 製品の目的

Codexのagent loop、権限、Shell、filesystem、MCP、Skills、Browser、Computer Use、subagent、会話とtask/session状態を保持したまま、同じCodex環境からNative OpenAIモデルとGrokモデルを選択できるようにする。

Grok Codex Bridgeはagent harnessでもtool executorでもない。Codexがtoolを実行し、bridgeはCodex Responses protocolとGrok Responses protocolの境界だけを担当する。

```text
Codex harness
  ├─ Native model ── first-party OpenAI Responses
  └─ Grok model ──── loopback bridge ─── xAI Grok upstream
```

## 2. 責務境界

- Codexはagent loop、session/task assembly、permissions、tool execution、MCP、Skills、Browser、Computer Use、保存履歴を所有する。
- Rust bridgeはloopback endpoint、caller capability、Responsesの投影とstream変換、Grok catalog、Grok upstream client、read-only credential検査、service/lifecycle操作を所有する。
- Swift `Grok Codex Switch.app`は、Codex Desktopが終了した後も生き残るLaunchServices launcherである。Rust switch coordinatorを起動して終了まで待ち、graceful quit、service/config/picker変更、Desktop再起動そのものはRust coordinatorが担当する。
- xAI upstreamはGrok推論とupstream認証契約を所有する。bridgeはGrokのagent harnessやtool executorを再実装しない。
- `Grok.md`は稼働中Grokモデルへ注入するoverlay本文の唯一のrepo正本である。別用途でこの予約名を使わない。

## 3. 配布と実行単位

配布するのはsourceであり、コンパイル済みbinaryではない。利用者または導入agentがmacOS arm64環境でRust bridgeとSwift launcherをローカルコンパイルする。

```text
source checkout
  ├─ Rust source: src/, Cargo.toml, Cargo.lock
  ├─ Swift source: scripts/macos-switch-launcher/
  ├─ Grok.md                 # Grok憲法・overlayのrepo正本
  └─ scripts/materialize-macos.sh
        ↓ local materialization and install
~/Library/Application Support/grok-codex-bridge/
  ├─ bin/grok-codex-bridge
  ├─ bin/Grok Codex Switch.app
  │   └─ Contents/Resources/grok-codex-bridge-overlay.md
  ├─ config/bridge.toml
  ├─ state/models.json
  ├─ secrets/caller-capability
  └─ install-manifest.json
```

通常実行ではCargo、Swift compiler、source checkout、`target/`、`dist/`を参照しない。`Grok.md`はmaterialization時に内容を別名のruntime snapshotへコピーする。snapshotの名前は `grok-codex-bridge-overlay.md` とし、`Grok.md`という名前をruntimeの別用途に再利用しない。

## 4. Materialize、install、updateの境界

`scripts/materialize-macos.sh`はRust executableとSwift launcher appを生成し、署名検証まで行う。通常runtimeはmaterialized成果物を直接実行する。build-on-first-use、`cargo run`、arbitrary Cargo cache探索、interpreted fallbackは禁止する。

`grok-codex-bridge install`は、materialized Rust binary、launcher app、runtime config、bootstrap catalog、caller capability、manifestをinstall rootへ原子的に配置し、Codexのisolated `grok-bridge.config.toml`とLaunchAgentを可逆的に用意する。既存のprofileとLaunchAgentは非上書きbackupへ保存する。

source更新は通常の環境切り替えとは別の明示操作である。更新時は新しいRust binaryとSwift launcherを検証し、serviceを停止してから原子的に交換し、serviceが `loaded` へ収束したことを確認する。失敗時は旧binary、旧launcher、service状態を復元する。

## 5. 日常のNative/Grok環境切り替え

日常操作は、install済みbinaryの `mode grok` または `mode native` を入口とする。両モードは同じpicker provider identityを保持し、保存済みtaskのprovider、model、SQLite、rollout、sessionデータを書き換えない。

- `mode grok`: Grok catalogを更新し、Grok rowsを選択可能にし、Grok slugをxAIへ、Native slugをNative upstreamへrouteする。
- `mode native`: Grok rowsをpickerの選択肢から隠し、保存済みGrok slugのmetadataだけをNative fallbackへruntime変換して継続可能にする。保存値そのものは変更しない。
- Native modeからGrokを選択できてはならない。ただし既存taskの保存済みGrok参照を壊してはならない。
- Grok modeへ戻すと、保存済みGrok taskはGrok routeへ戻り、Native taskはNative routeを維持する。

切り替えはCodex Desktopを安全に終了し、約15〜20秒を目安に再起動する。launcherはDesktop終了後もRust coordinatorを待ち続ける。picker/service変更とChatGPT.appのrelaunch requestが成功した時点でcoordinatorは完了を記録し、遷移ログへ結果を残す。

## 6. Merged pickerと保存task互換性

pickerはNative catalogのコピーへ、Grok catalogから許可されたmodel rowsを追加したgenerated catalogである。Native catalog原本は編集しない。Grok rowsには `Grok.md` の本文を `base_instructions` として注入する。

Codex configのmanaged blockは `model_provider = "grok_codex_picker"`、generated catalog path、loopback provider、caller headerを所有する。provider identityはNative modeでも削除・改名・置換しない。これにより保存済みprovider/model値をそのまま認識できる。

Native compatibility catalogでは、Grok slugを非選択の `visibility = "hide"` metadataとして残し、Native fallback modelへrouteする。これはpicker表示用のデータ変換であり、task DB、SQLite、rollout、session履歴の書き換えではない。

## 7. Responsesとtool境界

bridgeは `POST /v1/responses` と、Native補助APIの必要なResponses経路をloopbackで提供する。Grok requestではvalidなtext、画像、function call/output、tool call ID、`call_id`、必要なtool schemaを保持する。provider固有で再利用不能なforeign artifactだけをitem単位で除外する。

Codex側へはResponses-compatible SSEを返す。text、reasoning summary、function call、terminal/usageを投影する。unknown補助eventでstream全体を捨てず、終了markerが欠けた場合もoutput itemを捏造せず、必要な `response.completed` だけを合成する。

Native routeではxAI headerを送らず、caller capability headerもNative upstreamへ転送しない。画像生成、画像編集、search等のNative-owned requestはGrok catalogへ誤分類しない。

## 8. Configとruntime state

install rootのruntime configは、version、loopback bind、caller capability file、Grok catalog cacheを含む。bindは `127.0.0.1` または `::1`だけを許可し、port zero、LAN bind、remote accessを拒否する。

Codexの既存 `config.toml` はmarker-delimited managed blockだけを更新し、無関係なtable、comment、format、user設定を保持する。更新はread、validate、private backup、temp write、atomic renameの順で行う。partial marker、競合managed value、symlink、unsafe permissionはfail closedする。

生成catalog、Native route state、managed state、backup identityはinstall rootで管理し、各artifactのpath、size、SHA-256を記録する。Native catalogはCodex所有のread-only inputであり、更新・uninstallで削除または復元しない。

## 9. Credentialと認証

Grok credentialは公式のauthoritative fileをread-onlyで検査する。token、refresh token、authorization header、完全なcapability URLをrepo、config、log、SQLite、snapshot、環境ダンプへ複製しない。memory cacheを使う場合もzeroize可能な型を用い、diskへ再保存しない。

Responses requestでcredentialのmissing、incomplete、hard expiryを検出した場合だけ、公式 `bin/grok models` を切断stdio・7秒timeoutで一度起動し、最大60秒read-only再読込する。明示 `auth ensure` は必要時だけ公式OAuth経路へ委譲する。malformedまたはunsafe credentialは自動loginせずfail closedする。`auth status`、`doctor`、`catalog refresh`は認証更新helperを起動しない。

Native modeのrouting判断はGrok credentialの内容に依存しない。Grok modeへの移行または更新でcredentialが必要な場合だけ、明示的なauth境界を通る。

## 10. LaunchAgentと停止順序

LaunchAgentはuser domainへ配置し、インストール済みRust binaryとinstall rootのconfigを直接起動する。stdout/stderrへsecretを出さない。service install/uninstallはlaunchctl受付だけで成功にせず、bounded pollで `loaded` または `not_loaded` へ収束してから返す。

picker activationは、既存service状態を確認し、必要なら停止、picker artifacts公開、service起動、状態検証を一つの可逆操作として行う。公開または起動に失敗した場合は、generated artifacts、config、serviceを以前の状態へ戻す。Desktop切り替えでは、app終了前に新runtimeの検証と必要なservice準備を完了させる。

## 11. Full uninstall

full uninstallは日常の `mode native` と別の明示的な破壊境界である。順序はservice停止、picker managed blockと生成artifactの復元・削除、isolated profileとLaunchAgentの原状復帰、install rootのmanifest-owned tree削除とする。

uninstallはGrok credential、ChatGPT credential、Codex task/session、SQLite、rollout、projects、MCP、Skills、Browser設定、`AGENTS.md`、Native catalogを変更しない。保存済みtaskにGrok provider/model参照が残る場合、それを自動変換・削除せず、Grok環境が存在しないことによる参照不能を明示的なデータ移行なしに解消したとは扱わない。

manifest、backup、picker state、managed config identityが読めない、改変されている、または対象がsymlinkの場合は削除せず停止する。rollback不能な状態で部分削除を進めない。

## 12. Securityとlogging

loopback caller capabilityを要求し、capability tokenはprivate permissionで保存する。Grok credentialはallowlistされた公式upstreamだけへ送信し、redirectで別hostへAuthorizationを転送しない。fingerprint偽装、rate-limit回避、token farming、OAuth interception、hidden fallback、LAN公開は実装しない。

通常ログに許可するのはtimestamp、request ID、route、model slug、HTTP status、duration、stream completion、error classだけである。prompt、response body、tool arguments/results、画像、credential、authorization header、caller token、session本文を記録しない。debug loggingでもこの制限を緩めない。

## 13. CLIの製品経路

現行CLIは次の責務を持つ。

- `run --config FILE`: installed serviceのloopback providerを起動
- `install`: materialized binary、Swift launcher、config、profile、LaunchAgentを可逆導入
- `doctor`: manifest、binary、capability、runtime config、profile、backup、LaunchAgent、credential、serviceを検査
- `auth status` / `auth ensure`: credentialの状態確認と明示的な公式認証委譲
- `catalog refresh --config FILE`: credential更新を起動せず、last-known-good catalogを更新
- `service install` / `service uninstall` / `service status`: LaunchAgentを収束確認付きで操作
- `picker install` / `picker uninstall`: merged pickerの公開・復元
- `mode grok` / `mode native`: installed runtimeによる双方向環境切り替え
- `switch`: Desktop停止、picker更新、runtime交換、再起動をlauncherへ引き渡す
- `uninstall`: full uninstallの明示境界

通常のmode切り替えはsource build、install、credential file直接編集、session migrationを行わない。

## 14. 受け入れ条件

- [ ] Rust bridgeとSwift launcherをsourceからローカルmaterializeできる。
- [ ] install後はrepo、Cargo、Swift compiler、`target/`、`dist/`なしでserviceとmode切り替えが動く。
- [ ] install rootへbinary、launcher、overlay snapshot、config、catalog、capability、manifestが揃う。
- [ ] `Grok.md`はrepo正本専用で、runtimeは別名snapshotを使う。
- [ ] Native modeでGrok rowsが選択不可になり、保存済みGrok taskはNative fallbackで継続できる。
- [ ] Grok modeでGrok rowsが復帰し、保存済みNative taskはNative routeを維持する。
- [ ] 保存済みprovider/model値、SQLite、rollout、session/task historyを変更しない。
- [ ] Native upstreamとGrok upstreamのcredential・header・routingが混線しない。
- [ ] service状態がbounded pollで収束し、更新失敗時に旧runtimeへrollbackできる。
- [ ] full uninstallがbridge-owned stateだけを復元・削除する。
- [ ] prompt、response、tool content、credentialがlogやruntime stateへ漏れない。

## 15. 非対象

汎用LLM router、別provider marketplace、LAN/remote access、cloud sync、独自OAuth UI、billing/quota dashboard、automatic updater、Codex session database migration、保存履歴の恒久的なprovider/model変換、Codex agent harnessやtool executorの再実装は対象外である。
