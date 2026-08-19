# grok-codex-bridge

[English](README.md)

**Native GPTを置き換えず、Codexハーネスの中でGrokを動かすための、Rust製ネイティブResponses-to-Responsesブリッジです。**

`grok-codex-bridge` はApple Silicon搭載macOS向けの、スタンドアロンかつループバック専用のプロバイダーブリッジです。エージェントループ、ツール、権限、MCPサーバー、Skills、セッション状態は引き続きCodexが担当します。本プロジェクトが担当するのは、ローカルのプロバイダー境界、厳密なResponsesプロトコル変換、検証付きSSEストリーミング、Grok認証情報の読み取り専用利用、xAIへの上流接続だけです。

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

本プロジェクトは、[duolahypercho/codex-router](https://github.com/duolahypercho/codex-router/tree/9995c77278608640759982c98ec5bdaeb371c174) から多くの重要な知見を得ています。その実装とドキュメントは実現可能性を確認するうえで大切な先行事例であり、公式Grok認証情報の鮮度、プロバイダー境界に閉じるメタデータ、xAI Responses/SSEイベント体系、ネイティブモデルピッカー統合、可逆な有効化方式の設計判断に大きく役立ちました。MIT Licenseでこの成果を公開された作者とコントリビューターの皆様に、心より敬意と感謝を表します。

本リポジトリには `codex-router` のソースコードをコピーしていません。`grok-codex-bridge` は、LiteLLM/Chatの多段変換ではなく直接的なResponses-to-Responses転送を行う、独立したRust実装です。

## Rustを採用した理由

通常運用を、Python、Node.js、JITランタイムに依存しない単一のビルド済みネイティブ実行ファイルにするため、ブリッジ全体をRustで実装しています。Rustの強い型、明示的なエラー処理、メモリ安全な並行処理、決定的なリリース生成も、プロトコル境界とライフサイクル境界に適しています。

通常利用時にオンデマンドコンパイルは行いません。Cargoは開発と構築にだけ使用し、ランチャーは生成済みバイナリを直接実行します。バイナリが存在しない、またはソースより古い場合は安全側に停止します。

## アーキテクチャ

```text
Codexハーネス
  エージェントループ · ツール · 権限 · MCP · Skills · セッション
        |
        | capabilityで保護されたResponsesリクエスト
        v
grok-codex-bridge（Rust製ネイティブ実行ファイル）
  ローカル認証 · リクエスト正規化 · SSE検証
        |
        | 接続先を固定したResponses転送
        v
Grok / xAI
```

ブリッジ自身はツールを実行しません。関数定義、順序付きツール呼び出しと結果、テキスト、画像URLとdata URI、reasoning item、対応済みResponses制御値を保持し、実行責任はCodexに残します。

## 現在の状態

| 対象 | 状態 |
| --- | --- |
| V1.0 分離型 `grok-bridge` プロファイル | 実装済み・Codex CLIで検証済み |
| Rustネイティブビルドと可逆なユーザーサービス | 実装済み・検証済み |
| V1.1 Native GPT/Grok統合モデルピッカー | 実装済み・CLIでの切替と履歴継承を検証済み |
| V1.1 Skillsメタデータ予算 | Grokカタログに272,000トークンを設定し、Codex標準の2%計算を使用 |
| Desktopピッカーと最終rollback受け入れ | 最終検証待ち |
| 公開リリースバイナリ | 提供しません。ソースからbuild・materializeしてください |

V1.0は保守的な公開ルートです。Codexの分離プロファイルを使い、Native GPT設定には触れません。V1.1はDesktopと最終rollbackの受け入れが完了していないため、現時点では実験的機能です。

## 主な機能

- Codex ResponsesからxAI Responsesへの厳密な正規化。旧Chat Completions形式への変換は行いません。
- 安定したID、座標、シーケンス番号、完了テキスト、ツール引数、終端状態をイベント単位で検証するSSE処理。
- 画像をダウンロード・再エンコードせず、順序付き関数呼び出し/結果とテキスト・画像混在入力を保持。
- 公式Grokセッション認証情報を読み取り専用で利用し、変更時はゼロ化対応メモリキャッシュを再読み込み。
- rustlsで公式xAI接続先に固定し、リダイレクトを禁止。認証、レート制限、HTTP状態、stream障害を型付きで処理。
- Grokモデルをカタログで許可し、メタデータだけのlast-known-good状態をatomicに保存。
- ループバック専用listenerとcapability保護されたroute。不正なcapabilityには `404` を返します。
- install、LaunchAgentサービス、診断、ピッカー有効化、設定の完全復元を可逆に管理。
- request path、capability、認証情報、response bodyを残さないメタデータ限定ログ。

## 必要環境

- Apple Silicon搭載macOS。
- ソースからビルドする場合は [rust-toolchain.toml](rust-toolchain.toml) で固定されたRust 1.95.0。
- live Grokリクエストに使用する公式Grokログイン。
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
  --native-upstream-base-url "https://chatgpt.com/backend-api/codex"
```

`--native-catalog` は実行時に既存の絶対パスへ解決される必要があります。利用中のCodex認証ルートで別の公式上流が有効な場合、例のURLをそのまま流用しないでください。

有効化後は新しいCodex CLIプロセスを起動します。Codex Desktopで試す場合は、完全終了してから再起動してください。

統合されたGrok 4.5/4.6エントリは272,000トークンのコンテキストウィンドウを公開します。これによりCodexは、不明なwindow向けの小さなfallbackではなく、Nativeモデルと同じ標準2%のSkills説明予算計算を適用します。

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

`./scripts/materialize-macos.sh`、導入済みバイナリの更新、`service install` は、このブリッジを使っていないセッションから実行してください。この手順の想定オペレーターはGrok Buildです。Native GPTモデルのCodexセッションでも同様に生存します。`service status` が `service loaded` を返したら、新しいCodex CLIプロセスを起動するか、Desktopを完全再起動してクライアントをつなぎ直してください。

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

ブリッジは、設定された `GROK_AUTH_PATH`、絶対パスの `GROK_HOME`、またはGrok公式の既定homeから公式Grokセッション認証情報を検出します。選択した認証ファイルはsymlinkを追跡せず、読み取り専用で開きます。ログイン、token refresh、認証情報の修復は公式Grokフローの責任です。

サーバーを起動せず、公式カタログを一度だけ取得します。

```sh
./dist/aarch64-apple-darwin/grok-codex-bridge catalog refresh \
  --config ./docs/bridge-config.example.toml
```

checked-in exampleは意図的に起動時refreshを無効化しています。live refreshを行う場合は未追跡のローカルファイルへコピーし、placeholderを有効な絶対runtime pathへ置き換え、そのファイルで設定を有効にしてください。認証情報やマシン固有パスは絶対にcommitしないでください。

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
src/cli.rs                    CLI境界
src/config.rs                 version付きruntime設定
src/credential.rs             読み取り専用Grok認証境界
src/catalog.rs                atomicなmetadata-onlyモデルカタログ
src/grok.rs                   xAI接続先を固定したtransport
src/protocol.rs               Responses正規化とSSE検証
src/server.rs                 capability保護されたloopback service
src/lifecycle.rs              可逆なinstallとrollbackの所有者
src/picker.rs                 Native GPT/Grok統合catalog生成
src/picker_activation.rs      pickerのatomicな公開と有効化
src/launchd.rs                型付きuser LaunchAgent境界
scripts/materialize-macos.sh  決定的macOS arm64 materialization
```

## ライセンス

[MIT License](LICENSE) で公開します。
