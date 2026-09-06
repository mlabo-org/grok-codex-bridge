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
- Rust bridgeはloopback endpoint、caller capability、Responsesの投影とstream変換、Native/Grok catalogの継続的な統合、Grok upstream client、read-only credential検査、service/lifecycle操作を所有する。
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

RustとSwiftの中間出力は処理専用の一時directoryへ置き、終了時に削除する。Cargoにはこの出力先を明示し、同じ場所から成果物を取り出す。環境の `CARGO_TARGET_DIR` やrepo内の古い `target/` を成果物の取り出し元にしない。両成果物の検証を終えてから `dist/` へ配置する。

`grok-codex-bridge install`は、materialized Rust binary、launcher app、runtime config、bootstrap catalog、caller capability、manifestをinstall rootへ原子的に配置し、Codexのisolated `grok-bridge.config.toml`とLaunchAgentを可逆的に用意する。既存のprofileとLaunchAgentは非上書きbackupへ保存する。

source更新は通常の環境切り替えとは別の明示操作である。更新時は新しいRust binaryとSwift launcherを検証し、Desktop停止要求より前にserviceを停止して原子的に交換し、serviceが `loaded` へ収束したことを確認する。失敗時は旧binary、旧launcher、service状態を復元し、Desktop停止へ進まない。Grok modeを公開または維持する交換はservice停止前にnew binaryの`auth ensure`を完了する。Native compatibilityを明示した退避交換はGrok credentialをread、refresh、loginせず、pair検証とrollbackだけを同じ交換経路で行う。

## 5. 日常のNative/Grok環境切り替え

日常操作は、install済みbinaryの `mode grok` または `mode native` を入口とする。両モードは同じpicker provider identityを保持し、保存済みtaskのprovider、model、SQLite、rollout、sessionデータを書き換えない。

- `mode grok`: Grok catalogを更新し、Grok rowsを選択可能にし、Grok slugをxAIへ、Native slugをNative upstreamへrouteする。
- `mode native`: Grok rowsをpickerの選択肢から隠し、保存済みGrok slugのmetadataだけをNative fallbackへruntime変換して継続可能にする。保存値そのものは変更しない。
- Native modeからGrokを選択できてはならない。ただし既存taskの保存済みGrok参照を壊してはならない。
- Grok modeへ戻すと、保存済みGrok taskはGrok routeへ戻り、Native taskはNative routeを維持する。

切り替えはCodex Desktopを安全に終了し、約15〜20秒を目安に再起動する。launcherはDesktop終了後もRust coordinatorを待ち続ける。picker mutationが成功し、ChatGPT.appのrelaunch requestも成功した時点でcoordinatorは完了を記録する。picker mutationが失敗した場合は所有するrollbackを完了し、entry時にDesktopが動いていたときは復元済み状態でrelaunchを一度試みたうえで失敗を返す。mutationとrelaunchの両方が失敗した場合は両方のfailureを遷移結果へ残す。

## 6. Merged pickerと保存task互換性

pickerはNative catalogのコピーへ、Grok catalogから許可されたmodel rowsを追加したgenerated catalogである。Native catalog原本は編集しない。Grok rowsには `Grok.md` の本文を `base_instructions` として注入する。

常駐serviceは、Codex所有の `models_cache.json` を起動時と1時間間隔でmetadataだけ確認し、変更時だけ内容とSHA-256を再検証してmerged pickerを再生成する。この監視はrequest処理から独立しており、各requestへcatalog検査を追加しない。Grok modeでは公式Grok catalogもservice起動時と1時間間隔で更新し、同じ再生成経路へ連鎖させる。したがって新しいNative GPT rowと、`grok-` identifier契約を満たす新しいGrok rowは、手動のmode切替や `catalog refresh` なしで統合pickerへ入る。

再生成時はgenerated catalog、Native route state、managed-state identityを一つのrecoverable operationとして更新し、実行中serviceのNative allowlistとGrok allowlistも同じ結果へ切り替える。入力が不正、部分書込み中、またはmanaged stateと競合する場合はlast-known-goodを維持し、次の1時間周期で再試行する。catalog同期だけを理由にChatGPT.appを強制終了または自動再起動しない。

Grok一覧の取得成功後は、次のNative監視周期を待たずに同じ同期処理を実行する。取得した候補だけで実行中Grok allowlistを先行更新しない。要求の振り分けはNative/Grok両一覧を同じ公開状態から読み、同期が失敗した場合は以前の振り分けを維持する。同期結果の `changed` はcatalog・routeだけでなく、設定とmanaged stateの変更も含む。

Codex configのmanaged blockは `model_provider = "grok_codex_picker"`、generated catalog path、loopback provider、caller headerを所有する。provider identityはNative modeでも削除・改名・置換しない。これにより保存済みprovider/model値をそのまま認識できる。

Native compatibility catalogでは、Grok slugを非選択の `visibility = "hide"` metadataとして残し、Native fallback modelへrouteする。これはpicker表示用のデータ変換であり、task DB、SQLite、rollout、session履歴の書き換えではない。

## 7. Responsesとtool境界

bridgeは `POST /v1/responses` と、Native補助APIの必要なResponses経路をloopbackで提供する。Grok requestではvalidなtext、画像、function call/output、tool call ID、`call_id`、必要なtool schemaを保持する。function/tool-search outputの`call_id`が空・欠損、または先行する対応callが存在しない場合は`invalid_request_error`で拒否し、結果だけを削除して正常な履歴として送らない。provider固有で再利用不能なforeign artifactだけをitem単位で除外し、非objectなど構造不正なinput itemは削除せず`invalid_request_error`で拒否する。GrokからNativeへ戻す混在履歴では、Grok reasoningと、それに付随するprovider固有の`web_search_call`実行記録をNative upstreamへ送らず、tool call/outputを結ぶ`call_id`は保持する。Codex履歴で完了済みparallel tool batchの途中にassistant messageがある場合は、message本文、call順、output順、`call_id`を保持したままxAI projection内だけでそのmessageをbatch直前へ移動し、Codexの保存履歴は変更しない。

Codex側へはResponses-compatible SSEを返す。text、reasoning summary、function call、terminal/usageを投影する。既知eventはそのevent種別に必要なresponse ID、item ID、payloadを検証し、欠損を空文字やindex既定値で有効化しない。unknown補助eventは内容を捏造せずpassthroughする。Grok upstreamはdownstreamへresponse内容を確定する前のtransport確立またはbody streamのtransport failureに限り最大3回再試行する。一方、Native Responsesとcompactでは、output前の接続・timeout・response header前の切断、初期body streamのtransport failure、およびHTTP 429・502・503・504を、送信と初期body待ちで共有する最大3回の再試行枠で扱う。backoffは1秒、2秒、4秒とし、最初の送信から60秒の共通期限を送信処理と成功応答の初期body待ち自体にも適用する。期限後は新たな送信を開始しない。Retry-Afterが残り期限内に収まる場合はその待ち時間を尊重する。Nativeのresponse output開始後にrequestを再実行せず、正常に続くstreamをこの初期応答期限で打ち切らない。終了markerが欠けた場合もoutput itemを捏造せず、必要な `response.completed` だけを合成する。

Native compatibilityでは、保存済みGrok slugをNative fallbackへ変換する処理をResponsesとcompactの両方に適用する。compactへResponses専用の履歴sanitizerを追加しない。model変更がないcompact要求は元のbody bytesを保持し、変更時も元のContent-Encodingを維持する。

要求JSONは振り分けとprovider変換で解析結果を共有する。Native向けの変更が不要な場合は元のHTTP bodyを維持する。Grok向け送信本文は一度投影・serializeし、早期再試行では同じbytesを再利用する。parallel tool batchの並べ替えではassistant messageをまとめて前置し、messageごとの中間挿入を行わない。

Native routeではxAI headerを送らず、caller capability headerもNative upstreamへ転送しない。画像生成、画像編集、search等のNative-owned requestはGrok catalogへ誤分類しない。

## 8. Configとruntime state

install rootのruntime configは、version、loopback bind、caller capability file、Grok catalog cacheを含む。bindは `127.0.0.1` または `::1`だけを許可し、port zero、LAN bind、remote accessを拒否する。

Codexの既存 `config.toml` はmarker-delimited managed blockだけを更新し、無関係なtable、comment、format、user設定を保持する。更新はread、validate、private backup、temp write、atomic renameの順で行う。partial marker、競合managed value、symlink、unsafe permissionはfail closedする。

生成catalog、Native route state、managed state、backup identityはinstall rootで管理し、各artifactのpath、size、SHA-256を記録する。Native catalogはCodex所有のread-only inputであり、常駐serviceは変更検知と読み取りだけを行う。更新・uninstallで削除または復元しない。

## 9. Credentialと認証

Grok credentialは公式のauthoritative fileをread-onlyで検査する。token、refresh token、authorization header、完全なcapability URLをrepo、config、log、SQLite、snapshot、環境ダンプへ複製しない。memory cacheを使う場合もzeroize可能な型を用い、diskへ再保存しない。

Responses requestでcredentialのmissing、incomplete、hard expiryを検出した場合だけ、公式 `bin/grok models` を切断stdio・7秒timeoutで一度起動し、最大60秒read-only再読込する。明示 `auth ensure` は必要時だけ公式OAuth経路へ委譲する。malformedまたはunsafe credentialは自動loginせずfail closedする。`auth status`、`doctor`、`catalog refresh`は認証更新helperを起動しない。

同じservice内の同時Responses requestは進行中の認証更新を共有し、成功・失敗どちらでも待機中の各requestからhelperを重複起動しない。待機は各requestの期限内で終了する。Native compatibilityへのruntime交換では、最終診断にも `doctor --native-compatibility` を使い、Grok credential pathの解決・検査・読込を省く。この診断でもruntime成果物とserviceの検査は省略しない。

Native modeのrouting判断とNative compatibilityを明示したsource runtime交換はGrok credentialの内容に依存しない。Grok modeへの移行またはGrok modeを公開・維持する更新でcredentialが必要な場合だけ、明示的なauth境界を通る。

## 10. LaunchAgentと停止順序

LaunchAgentはuser domainへ配置し、インストール済みRust binaryとinstall rootのconfigを直接起動する。この常駐processがprovider endpointとcatalog同期を一緒に所有し、別watcher scriptや手動cronを要求しない。stdout/stderrへsecretを出さない。service install/uninstallはlaunchctl受付だけで成功にせず、bounded pollで `loaded` または `not_loaded` へ収束してから返す。

picker activationは、既存service状態を確認し、必要なら停止、picker artifacts公開、service起動、状態検証を一つの可逆操作として行う。公開または起動に失敗した場合は、generated artifacts、config、serviceを以前の状態へ戻す。Desktop切り替えでは、app終了前にnew runtimeの検証、必要なpair交換、service収束を完了させる。app終了後のpicker mutationがrollbackして失敗した場合は、entry時に動いていたDesktopを復元済み状態でrelaunchしてからfailureを返す。

## 11. Full uninstall

`scripts/uninstall-native.rb` は、橋を撤去して公式OpenAI接続へ戻す経路である。橋の推論・mode実装は変更せず、導入済みCLIの `doctor --native-compatibility` と `uninstall` を使用する。既定の `--check` は読み取りのみ、`--execute` は撤去、旧provider名の直接接続設定、公式OAuth実推論確認を行い、`--handoff` はCodex内からTerminalへの外部実行と導入済み再起動ツールへの引渡しを行う。撤去後に公式App Serverの `config/batchWrite` を使い、user configのversion一致を条件として `grok_codex_picker` と `grok_bridge` をChatGPT OAuth・Responses・WebSocket対応の公式OpenAI直接接続として定義する。既定provider/modelや無関係な設定は変更せず、旧provider名が欠落していることによる過去taskの再開失敗を防ぐ。Grok modelの互換metadata、橋の待受・認証headerは残さない。ソースとOAuth認証情報は保持し、会話DB・rolloutを直接変換または削除しない。

撤去の受け入れは、所有されたruntime/profile/LaunchAgentの撤去または元の状態への復元、bridge待受の消失、実際の設定から起動した組み込みOpenAI provider、旧provider名が公式OpenAIへ直接接続すること、bridge catalog/URL設定の不在、公式App Serverのephemeral threadでのChatGPT OAuth実推論完了とする。既存の無関係な設定の復元・保全はnative uninstallの所有経路に任せる。再起動の引渡しだけでは画面の復帰完了としない。実行中の旧provider sessionが切れるため、Codex内からの撤去は外部引渡しを使う。

full uninstallは日常の `mode native` と別の明示的な破壊境界である。順序はservice停止、picker managed blockと生成artifactの復元・削除、isolated profileとLaunchAgentの原状復帰、install rootのmanifest-owned tree削除とする。

uninstallはGrok credential、ChatGPT credential、Codex task/session、SQLite、rollout、projects、MCP、Skills、Browser設定、`AGENTS.md`、Native catalogを変更しない。保存済みtaskのprovider/model参照は自動変換・削除しない。旧provider名の解決とGrok modelの推論互換性は別であり、保存済みmodelがGrokの場合は利用者がGPTを選択する必要がある。直接接続設定だけで全履歴の表示や推論互換性を保証したとは扱わない。

manifest、backup、picker state、managed config identityが読めない、改変されている、または対象がsymlinkの場合は削除せず停止する。rollback不能な状態で部分削除を進めない。picker artifactの削除途中で失敗した場合は、事前に保持したbridge所有artifact、managed config、必要なexact backupを復元して再試行可能な状態へ戻す。補償復元自体も失敗した場合は、元の削除失敗と復元失敗の両方を報告する。

## 12. Securityとlogging

loopback caller capabilityを要求し、capability tokenはprivate permissionで保存する。Grok credentialはallowlistされた公式upstreamだけへ送信し、redirectで別hostへAuthorizationを転送しない。fingerprint偽装、rate-limit回避、token farming、OAuth interception、hidden fallback、LAN公開は実装しない。

通常ログに許可するのはtimestamp、request ID、route、model slug、HTTP status、duration、stream completion、error classだけである。prompt、response body、tool arguments/results、画像、credential、authorization header、caller token、session本文を記録しない。debug loggingでもこの制限を緩めない。

## 13. CLIの製品経路

現行CLIは次の責務を持つ。

- `run --config FILE`: installed serviceのloopback providerを起動
- `install`: materialized binary、Swift launcher、config、profile、LaunchAgentを可逆導入
- `doctor`: manifest、binary、capability、runtime config、profile、backup、LaunchAgent、credential、serviceを検査。明示 `--native-compatibility` 時だけGrok credential検査を行わない
- `auth status` / `auth ensure`: credentialの状態確認と明示的な公式認証委譲
- `catalog refresh --config FILE`: credential更新を起動せず、last-known-good catalogを更新
- `service install` / `service uninstall` / `service status`: LaunchAgentを収束確認付きで操作
- `picker install` / `picker uninstall`: merged pickerの公開・復元
- `mode grok` / `mode native`: installed runtimeによる双方向環境切り替え
- `switch`: runtime交換をDesktop停止前に完了し、Desktop停止後のpicker更新、rollback、再起動をlauncherへ引き渡す
- `uninstall`: full uninstallの明示境界

通常のmode切り替えはsource build、install、credential file直接編集、session migrationを行わない。

## 14. 受け入れ条件

- [ ] Rust bridgeとSwift launcherをsourceからローカルmaterializeできる。
- [ ] install後はrepo、Cargo、Swift compiler、`target/`、`dist/`なしでserviceとmode切り替えが動く。
- [ ] install rootへbinary、launcher、overlay snapshot、config、catalog、capability、manifestが揃う。
- [ ] `Grok.md`はrepo正本専用で、runtimeは別名snapshotを使う。
- [ ] Native modeでGrok rowsが選択不可になり、保存済みGrok taskはNative fallbackで継続できる。
- [ ] Grok credentialがmissingまたはexpiredでも、Native compatibilityを明示したsource更新とNative modeへの退避が認証更新なしで完了する。
- [ ] Grok modeでGrok rowsが復帰し、保存済みNative taskはNative routeを維持する。
- [ ] `models_cache.json` に追加されたNative modelが手動mode切替なしでmerged pickerと実行中Native routeへ反映される。
- [ ] 公式Grok catalogに追加された将来の `grok-` modelが定期更新後にmerged pickerと実行中Grok allowlistへ反映される。
- [ ] 保存済みprovider/model値、SQLite、rollout、session/task historyを変更しない。
- [ ] Native upstreamとGrok upstreamのcredential・header・routingが混線しない。
- [ ] 構造不正なinput itemと必須fieldが欠けた既知SSE eventをfail closedし、unknown補助eventだけをpassthroughする。
- [ ] service状態がbounded pollで収束し、更新失敗時に旧runtimeへrollbackできる。
- [ ] picker mutation失敗時はrollback後にentry時のDesktop起動状態を復元し、mutationとrelaunchの両failureを失わない。
- [ ] full uninstallがbridge-owned stateだけを復元・削除する。
- [ ] prompt、response、tool content、credentialがlogやruntime stateへ漏れない。

## 15. 非対象

汎用LLM router、別provider marketplace、LAN/remote access、cloud sync、独自OAuth UI、billing/quota dashboard、bridge binaryまたはapp bundle自体の自己更新、Codex session database migration、保存履歴の恒久的なprovider/model変換、Codex agent harnessやtool executorの再実装は対象外である。
