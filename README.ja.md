# grok-codex-bridge

[English](README.md)

**Native GPTを置き換えず、Codexハーネスの中でGrokを動かすための、Rust製ネイティブResponses-to-Responsesブリッジです。**

`grok-codex-bridge` はApple Silicon搭載macOS向けの、スタンドアロンかつループバック専用のプロバイダーブリッジです。エージェントループ、ツール、権限、MCPサーバー、Skills、セッション状態は引き続きCodexが担当します。本プロジェクトが担当するのは、ローカルのプロバイダー境界、Responses transportの許容的なprovider projection、Codexが消費するSSE抽出、bridge側のGrok credential境界、xAIへの上流接続です。bridgeは公式credentialを読み取り専用で検査し、期限切れ時だけ公式Grok CLIをboundedな更新トリガーとして起動することがあります。credential自体の更新は公式CLIが所有します。

Codexプラグイン、汎用LLMルーター、エージェントハーネスではありません。

## Codexにインストールを任せる

このリポジトリには、coding agentが守る安全境界とライフサイクル契約を [AGENTS.md](AGENTS.md) に収録しています。Codex自身にソースを確認させ、ネイティブ実行ファイルをbuildし、安全側のV1.0分離プロファイルをinstallさせる場合は、リポジトリをcloneしてrootからCodexを起動します。

```sh
git clone https://github.com/mlabo-org/grok-codex-bridge.git
cd grok-codex-bridge
codex
```

起動したCodexへ、目的に応じて次のどちらかを依頼してください。

### V1.0分離プロファイルまでinstallする

```text
AGENTS.mdを最後まで読み、その契約に従ってください。このMacへV1.0の分離型
grok-bridgeプロファイルをbuild・installしてください。platformと前提条件を確認し、
既存差分を保持し、./scripts/materialize-macos.shとrepo所有のlifecycle commandだけを
使用して、最小のprimary-path checkを実行してください。実験的V1.1統合pickerの有効化、
Codex本体binary・Codex設定・Grok認証・LaunchAgent fileの直接編集、commit、push、
publishは行わないでください。Apple Silicon搭載macOSでない場合、または必要な
authoritative inputを確認できない場合は停止して不足境界を説明してください。
```

### 実験的V1.1統合ピッカーまで自動でinstallする

build、V1.0分離install、Native GPT/Grok統合ピッカー有効化までを、Codexへ一仕事で任せる場合はこちらを使用します。

```text
AGENTS.mdを最後まで読み、その契約に従ってください。このMacでV1.0分離型
grok-bridgeプロファイルをbuild・installし、同じjobの中で実験的V1.1 Native GPT/Grok
統合pickerの有効化まで続けてください。picker有効化前に、現在のauthoritative Native
Codex catalogと、実際に有効なfirst-party Responses upstreamを、credentialの読み取り・
copy・表示を行わずに特定してください。./scripts/materialize-macos.shとrepo所有のnative
lifecycle commandだけを使用し、既存差分と完全なrollback境界を保持してください。
install済みservice、Native/Grok統合catalog、Grokの272,000-token context metadataを、
最小のprimary-path checkで確認してください。Native Codex入力を推測すること、Codex
本体binaryへのpatch、Codex設定・Grok認証・LaunchAgent fileの直接編集、commit、push、
publishは禁止します。authoritative inputを確認できない場合はpicker有効化前に停止し、
不足内容を説明してください。成功時は必要なCLI/Desktopの完全再起動と、pickerの正確な
rollback commandを報告してください。
```

この経路はrepoが所有するV1.1導入処理をすべて自動化します。有効化したcatalogとprovider stateをCodexへ読み込ませるため、完了後の新規Codex CLIプロセス起動またはDesktop完全再起動だけは必要です。

初回installはCodexから実行できます。すでに稼働中のブリッジを、Grokを使っているCodexセッションから停止・差し替えしないでください。詳細は [既存インストールの更新](#既存インストールの更新) を参照してください。

## 謝辞

本プロジェクトは、[duolahypercho/codex-router](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174) から多くの重要な知見を得ています。その実装とドキュメントは実現可能性を確認するうえで大切な先行事例であり、公式Grok認証情報の鮮度、プロバイダー境界に閉じるメタデータ、許容的なResponses/SSE transport投影、ネイティブモデルピッカー統合、可逆な有効化方式の設計判断に大きく役立ちました。MIT Licenseでこの成果を公開された作者とコントリビューターの皆様に、心より敬意と感謝を表します。

本リポジトリには `codex-router` のソースコードをコピーしていません。`grok-codex-bridge` は、LiteLLM/Chatの多段変換ではなく直接的なResponses-to-Responses転送を行う、独立したRust実装です。

## Rustを採用した理由

通常運用を、Python、Node.js、JITランタイムに依存しない単一のビルド済みネイティブ実行ファイルにするため、ブリッジ全体をRustで実装しています。Rustの強い型、明示的なエラー処理、メモリ安全な並行処理、決定的なリリース生成も、プロトコル境界とライフサイクル境界に適しています。

通常利用時にオンデマンドコンパイルは行いません。Cargoは開発と構築にだけ使用し、ランチャーは生成済みバイナリを直接実行します。バイナリが存在しない、またはソースより古い場合は安全側に停止します。

## アーキテクチャ

```text
Codexハーネス
  エージェントループ · ツール · 権限 · MCP · Skills · サブエージェント · セッション
        |
        | capabilityで保護されたResponsesリクエスト
        v
grok-codex-bridge（Rust製ネイティブ実行ファイル）
  ローカル認証 · provider projection · Codex向けSSE抽出
        |
        | 接続先を固定したResponses転送
        v
Grok / xAI
```

ブリッジ自身はツールを実行しません。validな関数定義、順序付きツール呼び出しと結果、テキスト、画像URLとdata URI、reasoning summary、必要なResponses制御値を保持し、実行責任はCodexに残します。xAIがrequest全体を拒否するfunction schemaはGrokへの投影からのみ省略し、Codex側のcatalog、tool_search履歴、Native GPT経路は元のtoolを保持します。GPT/Grok切替時はproviderで再生できないitem IDとreasoning stateだけを除外し、tool call/outputを結ぶ`call_id`を保持します。Grokが有用なCodex向けeventのあとterminal markerなしで接続を閉じた場合、bridgeは出力itemを捏造せず、Codexが要求する`response.completed`だけを合成します。`response.failed`または`response.incomplete`を既に受け取っている場合は合成しません。

## 現在の状態

| 対象 | 状態 |
| --- | --- |
| V1.0 分離型 `grok-bridge` プロファイル | 実装済み・Codex CLIで検証済み |
| Rustネイティブビルドと可逆なユーザーサービス | 実装済み・検証済み |
| V1.1 Native GPT/Grok統合モデルピッカー | 実装済み・CLIでの切替を検証済み。supportedなmessage/function/tool-search履歴の双方向切替をbridge境界で保持 |
| V1.1 Skillsメタデータ予算 | Grokカタログに272,000トークンを設定し、Codex標準の2%計算を使用 |
| Desktopピッカーと最終rollback受け入れ | 最終検証待ち |
| 公開リリースバイナリ | 提供しません。ソースからbuild・materializeしてください |

V1.0は保守的な公開ルートです。Codexの分離プロファイルを使い、Native GPT設定には触れません。V1.1はDesktopと最終rollbackの受け入れが完了していないため、現時点では実験的機能です。

## 主な機能

- `store: false`と全入力履歴を使う、Codex ResponsesからxAI Responsesへの許容的なprovider projection。旧Chat Completions形式への変換は行いません。
- text、reasoning summary、function call、terminal/usageをCodex向けに抽出するSSE処理。unknownな補助eventでstreamを終了させません。有用なeventのあと`response.completed`なしでGrokが閉じた場合は、そのlifecycle markerだけを合成し、Codexが `stream closed before response.completed` としてターンを落とさないようにします。
- 画像をダウンロード・再エンコードせず、順序付き関数呼び出し/結果とテキスト・画像混在入力を保持。
- 公式Grokセッションcredentialをbridge側では読み取り専用で利用し、変更時はゼロ化対応メモリキャッシュを再読み込みします。provider request中に期限切れを検出した場合だけ、公式CLI自身のsilent OIDC refreshを促す非対話起動を一度だけboundedに行います。bridgeはrefresh tokenを扱わず、OAuthを実装せず、credential fileを書き換えません。
- rustlsで公式xAI接続先に固定し、リダイレクトを禁止。認証、レート制限、HTTP状態、stream障害を型付きで処理。
- Grokモデルをカタログで許可し、メタデータだけのlast-known-good状態をatomicに保存。
- ループバック専用listenerとcapability保護されたroute。不正なcapabilityには `404` を返します。
- install、LaunchAgentサービス、診断、ピッカー有効化、設定の完全復元を可逆に管理。
- request path、capability、認証情報、response bodyを残さないメタデータ限定ログ。
- Grok選択セッションからも公式Codexサブエージェントを使える。spawn時に `model` / `reasoning_effort` を省略すると、親のGrokではなくCodexの `[agents]` デフォルトが使われる。

## 必要環境

- Apple Silicon搭載macOS。
- ソースからビルドする場合は [rust-toolchain.toml](rust-toolchain.toml) で固定されたRust 1.95.0。
- 公式Grok CLIと、live Grokリクエストに使用する公式Grokログイン。
- 現行のCodex CLI。

Intel Mac、Linux、Windows向けのビルド済み成果物は現在提供していません。

## クイックスタート：V1.0分離プロファイル

リポジトリルートでネイティブ実行ファイルを一度生成し、リポジトリ付属ランチャーを実行します。

```sh
./scripts/materialize-macos.sh
./scripts/grok-codex.sh
```

初回はブリッジをインストールし、必要に応じてユーザーサービスを起動して、分離された `grok-bridge` プロファイルでCodexを開きます。2回目以降は導入済みネイティブ実行ファイルを再利用します。引数はCodexへそのまま渡されます。

```sh
./scripts/grok-codex.sh --version
./scripts/grok-codex.sh --activate-only
```

このランチャーはリポジトリ内での利用を前提とします。shell scriptだけを移動またはsymlinkして使う方法は、配布方式としてサポートしていません。

## 実験的V1.1統合ピッカー

V1.1ではNative GPTと許可済みGrokモデルを同じCodexモデルピッカーで選べる統合カタログを公開します。Nativeモデルの通信は取得済みのCodex公式上流に固定し、Grok通信だけをブリッジ経由でxAIへ送ります。

最初にネイティブブリッジを生成してインストールします。

```sh
./scripts/materialize-macos.sh
./dist/aarch64-apple-darwin/grok-codex-bridge install
```

次に、有効化前に取得した現在のNative Codex公式カタログと、実際に使用されている公式Responses base URLを指定してピッカーを有効化します。以下は標準的なChatGPT認証Codex環境の例です。

```sh
CODEX_DIR="${CODEX_HOME:-"$HOME/.codex"}"

./dist/aarch64-apple-darwin/grok-codex-bridge picker install \
  --native-catalog "$CODEX_DIR/models_cache.json" \
  --native-upstream-base-url "https://chatgpt.com/backend-api/codex" \
  --grok-overlay "$PWD/Grok.md"
```

`--native-catalog` は実行時に既存の絶対パスへ解決される必要があります。利用中のCodex認証ルートで別の公式上流が有効な場合、例のURLをそのまま流用しないでください。

有効化後は新しいCodex CLIプロセスを起動します。Codex Desktopで試す場合は、完全終了してから再起動してください。

許可済みGrokカタログエントリ（bootstrapの `grok-4.5` / `grok-4.6` を含む）は272,000トークンのコンテキストウィンドウを公開します。これによりCodexは、不明なwindow向けの小さなfallbackではなく、Nativeモデルと同じ標準2%のSkills説明予算計算を適用します。

### Grok.md overlay

[`Grok.md`](Grok.md) は Grok 専用実行 overlay の正本です。`picker install` がこのファイルをディスクから読み、生成カタログの許可済み Grok 行の `base_instructions` へそのまま入れます。Codex が消費するのは生成カタログです。Native GPT 行には届きません。本文はコンパイル成果物へ焼き込まず、HTTP 毎に再読込もしません。

`--grok-overlay` を省略できるのは、カレントディレクトリに `Grok.md` があるときだけです。overlay を直したあとは `picker install` を再実行し、新しい Codex CLI プロセスを起動するか Desktop を完全再起動してください。既存の Grok セッションは、起動時のカタログのままです。

この overlay は第二の憲法ではなく、仕事の伴走契約です。宣言した操作を同じターンでやり切り、必要なツールを呼んだあと、ツール呼び出しや進捗報告だけで終わらずユーザー向け本文まで出させます。統合ピッカーでカタログを書き直して再起動したセッションでは、Grok が実際に読むのがこの経路です。

### 公式サブエージェント

サブエージェントの起動はCodexが所有します。bridgeはprovider protocolを変換するだけで、workerを起動しません。

V1.1統合ピッカーのliveセッションで確認済みです。

- Grok親から公式Codexサブエージェント（`spawn_agent` / `wait_agent` / `close_agent`）を起動できる。
- spawn時に `model` または `reasoning_effort` を省略すると、Codex設定の `[agents].default_subagent_model` と `[agents].default_subagent_reasoning_effort` が使われる。親のGrokモデルや推論深度にはならない。
- 子をGrokで動かすには、`model` を許可済みカタログID（`grok-4.6` や `grok-4.5`）へ明示する。
- 推論深度も `reasoning_effort` を明示する。現行のGrokカタログは `low` / `medium` / `high` / `xhigh` を公開する。`grok-4.5` の `xhigh` まで確認済み。

### 現在確認されているresume制限

Grokを選択した状態で終了したセッションを直接resumeすると、次の警告が出る場合があります。

```text
MCP startup interrupted. The following servers were not initialized: codex_apps
```

現在の証拠では、ブリッジ通信や `codex_apps` handshakeではなく、Codex TUIのresume/MCP起動境界で発生しています。Codex側の挙動が解消されるまでは、Native GPTモデルを明示してresumeし、起動後にGrokへ切り替えてください。

```sh
codex resume <SESSION_ID> -m <NATIVE_GPT_MODEL>
```

## ライフサイクルとrollback

### 既存インストールの更新

このブリッジを使っているCodexセッションは、ローカルのループバックサービスに依存します。対象は分離型 `grok-bridge` プロファイルと、V1.1ピッカーでGrokモデルを選んでいる場合です。そのセッションの中からサービスを停止したり、導入済みバイナリを差し替えたりすると、モデル通信が切れます。再ロードまで完了しないと service は `not_loaded` のまま残り、サービスを起動し直すまでCodexからGrokへ届きません。

生成と導入済みバイナリの差し替えは、このブリッジを使っていないセッションから実行してください。この手順の想定オペレーターはGrok Buildです。Native GPTモデルのCodexセッションからでも実行できます。

新しい実行ファイルを生成したあとは、repo所有の差し替えスクリプトで導入済みbinaryを置換してください。このスクリプトはサービスを停止し、導入済みbinaryを入れ替え、サービスを再起動して `doctor` を実行します。

```sh
./scripts/materialize-macos.sh
./scripts/replace-installed-bridge.sh ./dist/aarch64-apple-darwin/grok-codex-bridge
```

`service status` が `service loaded` を返したら、新しいCodex CLIプロセスを起動するか、Desktopを完全再起動してクライアントをつなぎ直してください。

以下のコマンドはすべて生成済み実行ファイルを直接使います。

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge doctor
./dist/aarch64-apple-darwin/grok-codex-bridge auth status
./dist/aarch64-apple-darwin/grok-codex-bridge service status
```

統合ピッカー状態だけを削除し、ピッカー有効化直前のCodex設定を完全復元します。

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge picker uninstall
```

ユーザーサービスを停止してから、ブリッジ所有のインストールを削除します。

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge service uninstall
./dist/aarch64-apple-darwin/grok-codex-bridge uninstall
```

ライフサイクルmanifestが所有するのは、ブリッジが作成または置換したファイルだけです。完全uninstallでも、Codex本体設定、公式Grok認証状態、Native GPT設定は削除しません。

## モデルカタログと認証情報

ブリッジは、権威あるcredential fileを次の順序で解決します。

1. `GROK_AUTH_PATH` が設定されている場合はそのfile。
2. `GROK_HOME` が設定されている場合は `GROK_HOME/auth.json`。
3. それ以外は `~/.grok/auth.json`。

選択したfileはsymlinkを追跡せず、読み取り専用で開きます。`GROK_AUTH_PATH`はcredential fileを選び、`GROK_HOME`は更新トリガーに使う公式CLIのhomeを選びます。`GROK_AUTH_PATH`が公式Grok homeの外を指す場合は、対応する公式CLIを解決できるよう `GROK_HOME` も設定してください。`GROK_HOME`が未設定で `HOME` が利用できる場合、helperは `~/.grok/bin/grok` です。

公式session recordに `expires_at` があればその時刻を使います。無い場合はparserのfallbackとして `create_time + 30日` を使います。これは公式Grok sessionの有効期間を保証する値ではありません。`auth status` はcredentialの有無だけを表示し、credentialや有効期限は表示しません。

次の復旧経路は、Responses provider requestで期限切れを検出した場合だけ動作します。bridgeは公式 `bin/grok models` をstdin/stdout/stderr切断、7秒timeoutで一度だけ起動し、その後最大60秒、権威fileの再読込を待ちます。bridgeはcredentialを事前更新せず、refresh tokenを読まず、OAuthや対話loginを実装せず、`auth.json`を書き換えません。公式processがfileを更新できなければ、そのrequestは認証errorになります。

公式loginが期限切れまたは失われた場合は、bridgeを使っていない環境で公式device flowを実行します。

```sh
GROK_HOME_DIR="${GROK_HOME:-"$HOME/.grok"}"
"$GROK_HOME_DIR/bin/grok" login --device-auth
```

deviceまたはbrowserの確認は、CLIが表示した公式ページだけで完了してください。device codeをchat、log、repositoryへ貼らないでください。完了後、credentialを表示せずにbridgeを確認します。

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge auth status
./dist/aarch64-apple-darwin/grok-codex-bridge service status
```

`catalog refresh` は期限切れcredentialの復旧経路とは別物です。現在利用できるcredentialが必要で、更新helperは起動せず、last-known-goodのmodel catalogだけを更新します。checked-in configは絶対pathがplaceholderのtemplateなので、そのまま実行できません。未追跡のlocal configへコピーし、placeholderを実際の絶対pathへ置き換えてから実行します。

```sh
cp ./docs/bridge-config.example.toml ./bridge-config.local.toml
# ./bridge-config.local.toml のmachine-localな絶対pathを編集する。
./dist/aarch64-apple-darwin/grok-codex-bridge catalog refresh \
  --config ./bridge-config.local.toml
```

`refresh_on_start` はservice起動時だけに効きます。明示的な `catalog refresh` commandは常に一度だけcatalog requestを実行します。local configは未追跡のまま保持し、認証情報やruntime固有pathをcommitしないでください。

## セキュリティ境界

- listenerはloopbackだけにbindします。LAN公開はV1の対象外です。
- caller capabilityはローカルrouteに置き、service logには記録しません。
- 認証情報は公式の権威あるファイルとゼロ化対応メモリキャッシュにだけ保持し、catalogやCodex stateへコピーしません。
- Grok通信は公式xAI originに限定し、rustlsを使用してredirectを追跡しません。
- catalog writeと管理対象設定の変更はatomicかつ可逆です。
- 認証情報、token、`.env`、秘密鍵、runtime state、session log、生成catalog、マシン固有パスをGit履歴へ入れてはいけません。
- crateは crates.ioへの誤公開を防ぐため `publish = false` を宣言しています。

## 開発

ソースtestを実行します。

```sh
cargo test --locked
```

開発専用でソースから実行する場合：

```sh
cargo run -- --version
```

`cargo run` は通常runtime routeではありません。release binaryを生成し、直接実行してください。

```sh
./scripts/materialize-macos.sh
./dist/aarch64-apple-darwin/grok-codex-bridge status
```

製品scopeと受け入れcontractは [docs/spec-v0.1.md](docs/spec-v0.1.md)、配布要件は [docs/distribution-contract.md](docs/distribution-contract.md) で定義しています。

## ソース構成

```text
src/cli.rs                           CLI境界
src/config.rs                        version付きruntime設定
src/credential.rs                    読み取り専用Grok認証境界
src/catalog.rs                       atomicなmetadata-onlyモデルカタログ
src/native.rs                        取得済みfirst-party Native GPT上流route
src/grok.rs                          xAI接続先を固定したtransport
src/protocol.rs                      Responses provider projectionとSSE抽出
src/server.rs                        capability保護されたloopback service
src/lifecycle.rs                     可逆なinstallとrollbackの所有者
src/picker.rs                        Native GPT/Grok統合catalog生成
src/picker_activation.rs             pickerのatomicな公開と有効化
src/launchd.rs                       型付きuser LaunchAgent境界
scripts/materialize-macos.sh         決定的macOS arm64 materialization
scripts/grok-codex.sh                V1.0分離プロファイルランチャー
scripts/replace-installed-bridge.sh  導入済みbinaryの差し替え
```

## ライセンス

[MIT License](LICENSE) で公開します。
