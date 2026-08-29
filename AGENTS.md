# Grok Codex Bridge Local Constitution

このファイルは、このrepo配下におけるCodexの局所`AGENTS.md`であり、`grok-codex-bridge`のsource、責務、runtime、導入、検証境界を定義するSSOTである。
本書は助言集ではなく、既定動作、禁止事項、正本選択、実装境界、検証条件を拘束する運用契約として扱う。
上位の`AGENTS.md`、システム指示、開発者指示、現在のユーザー明示要求と競合する場合は、Codexの優先順位規則に従う。
より深い階層に別の`AGENTS.md`が存在する場合、そのスコープではより局所のファイルを優先する。

## Source And Runtime Boundary

- このrepoは、CodexとGrokを接続するRust provider bridgeとmacOS native switch launcherの現役source-of-truthである。Codex plugin、Skill、汎用LLM router、agent harnessとして扱わない。
- `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`scripts/`、`Grok.md`をexecutable sourceとする。`docs/spec.md`を製品意図、scope、runtime contract、acceptance contractのauthoritative sourceとする。
- `Grok.md`というfilenameは、Grok modelへ渡す稼働中constitution overlayのSSOTだけに予約する。別用途で同名fileを作らず、本文をRust source、Swift source、README、生成catalogへ手書き複製しない。materializationは同じbytesをlauncher resourceの別名`grok-codex-bridge-overlay.md`へsnapshotとしてcopyする。
- `README.md`と`README.ja.md`は公開利用者向け説明であり、protocolまたは製品scopeのauthorityではない。
- `target/`はCargo build cache、`dist/`はlocal materialized artifactであり、authoritative sourceまたはGit管理対象として編集しない。
- active runtimeは`$HOME/Library/Application Support/grok-codex-bridge/`配下へinstallされたRust executable、Swift app bundle、overlay snapshot、config、stateで構成する。明示的なinstallまたはactivation前にrepoの存在だけでactive runtimeとみなさない。

## Responsibility Ownership

- Codexはagent loop、permission、tool call、shell、filesystem、MCP、Skills、Browser、Computer Use、task/session stateを所有する。
- bridgeはloopback provider endpoint、Codex protocol parse、Grok protocol変換、streaming変換、local caller authentication、credential fileのread-only検査とzeroizing cache、許可された再認証trigger、Grok upstream client、merged picker catalogとrouting stateを所有する。Codex task/sessionの保存済みprovider、model、SQLite、rollout本文は所有せず変更しない。
- Swift launcherはChatGPT.appの終了後にも生存し、Rust switch coordinatorを起動して終了まで待つLaunchServices境界だけを所有する。graceful quit、service/config/picker切替、ChatGPT.app再起動はRust coordinatorが所有する。launcherはprotocol変換、tool実行、credential保存を所有しない。
- Grok upstreamはmodel inferenceとupstream authentication contractを所有する。公式CLIが行うcredential file更新、browser login、OIDC refreshはbridgeの所有外であり、bridgeはGrok agent harnessまたはtool executorを再実装しない。
- `scripts/materialize-macos.sh`はRust bridge、Swift launcher、別名overlay snapshotの対応する一組をbuildして`dist/aarch64-apple-darwin/`へ配置し、architectureとsignatureを確認する。install、常駐化、mode切替を所有しない。
- `scripts/grok-codex.sh`はsource checkoutからの初回installまたはmaterialized pair更新とmode handoffだけを所有する。install後の日常切替はinstalled binaryの`mode`が所有する。

## Protocol, Credential, And State Contracts

- Codexからbridgeへのhandoffは、現在のCodex authoritative sourceと実runtimeで確認したprotocolだけを実装する。過去の会話、README、推測したOpenAI互換性をprotocol authorityにしない。
- bridgeからGrokへのhandoffは、現在のxAI authoritative sourceまたは観測可能な公式client contractで確認したendpoint、headers、streaming semanticsだけを実装する。private endpoint探索、fingerprint偽装、未確認fallbackを追加しない。
- credentialは選択されたauthoritative fileをin-placeかつread-onlyで検査する。Responses provider requestで選択recordのhard expiry、missing、incompleteを検出した場合は、公式`bin/grok models`をstdin/stdout/stderr切断・7秒timeoutで一度だけ起動し、authoritative fileを最大60秒read-only再読込してよい。明示`auth ensure`も同じ非対話更新を先に試し、それでもmissing、incomplete、expiredなら公式`bin/grok login --oauth`をstdin/stdout/stderr切断で一度起動し、最大5分だけbrowser完了を待ってread-only再検査する。malformed、ambiguous、unsafeなcredentialではloginを起動せずfail closedする。`auth status`、`doctor`、`catalog refresh`は再認証triggerを起動しない。
- source runtime交換では、Grok modeを公開または維持する場合だけservice停止前にnew Rust binaryの`auth ensure`を実行する。`mode native`または`switch --native-compatibility`が所有するNative退避交換はGrok credentialをread、refresh、loginせず、pair検証、service収束、失敗時rollbackは通常交換と同じ経路で完了する。bridgeはcredential fileを直接書き換えず、repo、Codex config、log、SQLite、cache、environment dumpへ複製しない。
- Grok modeはNativeとGrokのpicker rowを公開し、model slugによりNative OpenAIとGrok upstreamを分離する。Native modeはGrok rowを新規選択不能にするが、provider/resolver/compatibility metadataを保持し、保存済みGrok taskをNative OpenAIで継続可能にする。
- mode切替は保存済みtaskのprovider/model、SQLite、rolloutを変換または書換しない。Native modeはuninstallではなく可逆的なcompatibility modeであり、Grokへ戻すときは`mode grok`を使用する。

## Agent Build, Install, Update, And Switching Route

- このrepoを受け取ったagentは、最初に`git status --short`、`README.md`または`README.ja.md`、`rust-toolchain.toml`、`scripts/materialize-macos.sh`を確認する。既存差分がある場合は保持し、競合するfileを上書きしない。
- build前提はApple Silicon macOS、pinned Rust 1.95.0 toolchain、`swiftc`、`codesign`である。build要求ではrepo rootから`./scripts/materialize-macos.sh`を一度実行する。Rust executableだけをlauncherなしでinstallまたはhandoffしてはならない。normal operationへ`cargo run`または`target/`内binaryを渡さない。
- sourceからのinstallまたはactivationはuserがそのactionを明示した場合だけ、Native GPT taskまたはTerminalから`./scripts/grok-codex.sh grok`または`./scripts/grok-codex.sh native`を使用する。scriptはcompileしない。materialized pairがmissingまたはstaleなら停止するため、先にmaterializationを完了する。
- source更新ではmaterialization後に`./scripts/grok-codex.sh grok|native`を使う。明示的なpair交換要求では`./scripts/replace-installed-bridge.sh ./dist/aarch64-apple-darwin/grok-codex-bridge "./dist/aarch64-apple-darwin/Grok Codex Switch.app"`を使う。Rust executableとSwift launcherを別versionのまま交換しない。
- install後の日常切替は`$HOME/Library/Application Support/grok-codex-bridge/bin/grok-codex-bridge mode grok`または`mode native`を直接使う。この経路はrepo、`target/`、`dist/`、source `Grok.md`、replacement script、compilerを探索せず、source checkoutを移動または削除した後もinstalled treeだけで完結する。
- mode切替は通常約15〜20秒かかる。ChatGPT.appをforce quitせず、launcherによるgraceful quit、bounded app-server停止、service/picker収束、automatic relaunchを待つ。macOSの通常のquit確認dialogを意図的に発生させる操作を追加しない。
- lifecycleを個別操作する明示要求では、materializedまたはinstalled binaryが公開する`install`、`service install|uninstall|status`、`picker install|uninstall`、`doctor`、`auth status|ensure`、`mode grok|native`、`uninstall`だけを使用する。Codex本体binary、Codex config、Grok auth、LaunchAgent plistを直接編集せず、`launchctl`を直接呼ばない。
- `--native-catalog`など絶対pathを要求するruntime引数は実行環境で解決する。README、handoff、commitへ個人固有の絶対path、credential、token、capability、session IDを記録しない。repo内file参照は相対pathで記述する。
- full uninstallはNative modeとは別の明示的な破壊境界である。保存済みtaskがbridge provider/modelを参照している間は、明示されたdata migrationなしにfull uninstallを実行しない。許可されたfull uninstallではbinaryのpreflightを通し、picker rollback、service停止、manifest-owned install removalを一つの所有経路に任せ、manifest外を削除または復元しない。
- build、install、activation、mode switching、full uninstall、commit、push、releaseは別actionである。あるactionの明示要求から隣接actionの権限を推測しない。

## Security And Stop Conditions

- network listenerは`127.0.0.1`または`::1`だけにbindする。`0.0.0.0`、LAN公開、remote accessを実装しない。
- caller authenticationが未実装または失敗する状態でprovider endpointを提供しない。
- token、credential、authorization header、capability URLをlog、panic、test fixture、snapshot、Git historyへ書き込まない。
- current protocol、credential source、consumer config、source/runtime境界のいずれかを確認できない場合、そのintegration実装を停止する。推測したadapter、fallback、defaultで補わない。

## Development And Verification

- source編集前に`git status --short`、`Cargo.toml`、対象call path、現在のmaterialized binary有無を確認し、既存差分を保持する。
- Rust source変更のminimum semantic acceptance bundleは、影響するfocused testとprimary path evidenceだけで構成する。現在のscaffold全体に対しては`cargo test --locked`を使用する。
- runnable binary、Swift launcher、materialization経路を変更した場合は`./scripts/materialize-macos.sh`を実行し、materialized Rust executableのdirect `--version`、両executableのmacOS arm64 Mach-O、FileProvider metadataを除去したinstall-equivalent staging copyのlauncher codesign、source `Grok.md`と別名resource snapshotのbyte equalityを一つのbundleで確認する。
- live install、mode往復、picker visibility、service stateはactivationが現在明示的に許可された場合だけ確認する。source-only要求をruntime mutationへ拡張しない。
- admitted bundleが成功した時点でverificationを終了する。追加review、broad matrix、重複buildを行わない。
- この`AGENTS.md`を作成または実質変更した場合は、final report、commit、runtime反映の前に`agents-md-clarifier`を一度適用する。

## Distribution, GitHub, And Release Boundary

- 配布物はsourceだけである。compiled Rust executable、compiled Swift launcher、`.app` bundle、release archiveをGit repositoryまたはGitHub Releasesへ置かない。利用者またはinstall担当agentが各自のApple Silicon macOS環境でbuildする。
- sourceをportableに保ち、secret、credential、個人固有の絶対path、runtime state、generated binaryをGit管理へ入れない。
- commit、remote作成、push、GitHub repository作成、release、license変更、crate publicationは別actionであり、現在の明示要求または個別承認なしに実行しない。
- crates.io publicationは`Cargo.toml`の`publish = false`でfail closedにする。公開方針が決まるまで解除しない。
