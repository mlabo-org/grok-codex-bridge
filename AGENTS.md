# Grok Codex Bridge Local Constitution

このファイルは、このrepo配下におけるCodexの局所`AGENTS.md`であり、`grok-codex-bridge`のsource、責務、runtime、検証境界を定義するSSOTである。
本書は助言集ではなく、既定動作、禁止事項、正本選択、実装境界、検証条件を拘束する運用契約として扱う。
上位の`AGENTS.md`、システム指示、開発者指示、現在のユーザー明示要求と競合する場合は、Codexの優先順位規則に従う。
より深い階層に別の`AGENTS.md`が存在する場合、そのスコープではより局所のファイルを優先する。

## Source And Runtime Boundary

- このrepoは、CodexとGrokの間を接続する独立Rust provider bridgeの現役source-of-truthである。Codex plugin、Skill、汎用LLM router、agent harnessとして扱わない。
- `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`scripts/`をexecutable sourceとする。`docs/spec-v0.1.md`を製品意図、scope、実装順序、acceptance contractのauthoritative sourceとする。
- `Grok.md` は Grok モデルへ渡す overlay 本文の SSOT である。picker はカタログ生成時にこのファイルだけを Grok 行の `base_instructions` へ注入する。生成カタログは写しであり、本文を `picker.rs` に複製しない。
- `README.md`と`README.ja.md`は公開利用者向け説明であり、protocolまたは製品scopeのauthorityではない。
- `target/`はCargo build cache、`dist/`はmaterialized runtime artifactであり、authoritative sourceまたはGit管理対象として編集しない。
- 明示的な導入、activation、Codex config変更、LaunchAgent作成が実施されるまで、このrepoの存在だけでactive runtimeまたはconsumerが存在するとみなさない。

## Responsibility Ownership

- Codexはagent loop、permission、tool call、shell、filesystem、MCP、Skills、Browser、Computer Use、task/session stateを所有する。
- bridgeは、loopback provider endpoint、Codex側protocolのparse、Grok側protocolへの変換、streaming変換、local caller authentication、credentialのread-only参照、Grok upstream clientだけを所有する。
- Grok upstreamはmodel inferenceとupstream authentication contractを所有する。bridgeはGrokのagent harnessまたはtool executorを再実装しない。
- materialization scriptはrelease binaryのbuildと`dist/aarch64-apple-darwin/`への配置だけを所有する。通常callerの起動、インストール、常駐化を所有しない。
- coordinatorまたはinstallerを追加する場合、その責務をroute選択、設定適用、起動停止へ限定し、protocol変換やtool実行を重複所有させない。

## Handoff Contracts

- Codexからbridgeへのhandoffは、現在のCodex authoritative sourceと実runtimeで確認したprotocolだけを実装する。過去の会話、README、推測したOpenAI互換性をprotocol authorityにしない。
- bridgeからGrokへのhandoffは、現在のxAI authoritative sourceまたは観測可能な公式client contractで確認したendpoint、headers、streaming semanticsだけを実装する。private endpoint探索、fingerprint偽装、未確認fallbackを追加しない。
- credentialは選択されたauthoritative fileをin-placeかつread-onlyで参照する。repo、Codex config、log、SQLite、cache、environment dumpへ複製しない。
- normal runtimeは`./scripts/materialize-macos.sh`が配置したconcrete executableを直接実行する。`cargo run`、build-on-first-use、Cargo cache探索、interpreted fallbackをnormal runtimeに使用しない。

## Public Agent Build And Install Route

- このrepoを受け取ったagentは、最初に`git status --short`、`README.md`または`README.ja.md`、`rust-toolchain.toml`、`scripts/materialize-macos.sh`を確認する。既存差分がある場合は保持し、競合するfileを上書きしない。
- build要求ではrepo rootから`./scripts/materialize-macos.sh`を一度実行し、`./dist/aarch64-apple-darwin/grok-codex-bridge --version`で生成済みnative binaryの代表経路を確認する。normal operationへ`cargo run`または`target/`内binaryを渡さない。
- installまたはactivationはuserがそのactionを明示した場合だけ実行する。安全側のV1.0既定経路には`./scripts/grok-codex.sh`を使用し、scriptを別locationへcopyまたはsymlinkしない。
- lifecycleを個別操作する明示要求では、生成済みbinaryの`install`、`service install`、`doctor`、`auth status`、`service status` subcommandだけを使用する。Codex本体binary、Codex config、Grok auth、LaunchAgent plistを直接編集せず、`launchctl`を直接呼ばない。
- merged pickerはuserがV1.1を明示した場合だけ有効化する。有効化前にauthoritative Native Codex catalogと実効first-party Responses upstreamを確認し、`picker install --native-catalog <FILE> --native-upstream-base-url <URL>`へ渡す。どちらかを確認できなければ停止し、推測値で有効化しない。
- `--native-catalog`など絶対pathを要求するruntime引数は実行環境で解決する。README、handoff、commitへ個人固有の絶対path、credential、token、capability、session IDを記録しない。repo内file参照は相対pathで記述する。
- rollback要求では、統合pickerを使用している場合は最初に`picker uninstall`、次に`service uninstall`、最後に`uninstall`を使用する。manifest外のfileを削除または復元しない。
- build、install、activation、rollback、commit、push、releaseは別actionとして扱う。あるactionの明示要求から隣接actionの権限を推測しない。

## Security And Stop Conditions

- network listenerは`127.0.0.1`または`::1`だけにbindする。V1では`0.0.0.0`、LAN公開、remote accessを実装しない。
- caller authenticationが未実装または失敗する状態でprovider endpointを提供しない。
- token、credential、authorization header、capability URLをlog、panic、test fixture、snapshot、Git historyへ書き込まない。
- current protocol、credential source、consumer config、source/runtime境界のいずれかを確認できない場合、そのintegration実装を停止する。推測したadapter、fallback、defaultで補わない。

## Development And Verification

- source編集前に`git status --short`、`Cargo.toml`、対象call path、現在のmaterialized binary有無を確認し、既存差分を保持する。
- Rust source変更のminimum semantic acceptance bundleは、影響するfocused testとprimary path evidenceだけで構成する。現在のscaffold全体に対しては`cargo test --locked`を使用する。
- runnable binaryまたはmaterialization経路を変更した場合は、`./scripts/materialize-macos.sh`を実行し、配置されたbinaryを直接起動して代表commandを確認する。
- admitted bundleが成功した時点でverificationを終了する。追加review、broad matrix、重複buildを行わない。
- この`AGENTS.md`を作成または実質変更した場合は、final report、commit、runtime反映の前に`agents-md-clarifier`を一度適用する。

## GitHub And Release Boundary

- GitHub公開を将来候補としてsourceをportableに保つ。secret、credential、個人固有の絶対path、runtime state、generated binaryをGit管理へ入れない。
- commit、remote作成、push、GitHub repository作成、release、license変更、crate publicationは別actionであり、現在の明示要求または個別承認なしに実行しない。
- crates.io publicationは`Cargo.toml`の`publish = false`でfail closedにする。公開方針が決まるまで解除しない。
