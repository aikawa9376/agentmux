# Herdr追従調査（2026-10-25）

調査後の採用範囲: 推奨順の1〜3とpopup表示を採用し、Git/worktreeとSSHマシン集約は対象外とした。実装後の仕様・制約は[共有監視・送信待機・popup設計](../design/observer-and-popup.md)に集約する。以下の比較は調査時の`ff46711`を基準とする。

## 結論

tmuxを基盤にしたまま追従できる。直近は **検出ルール更新 → 状態を共有する監視基盤 → 未読完了・通知・安全な送信／待機 → worktree／検索UI → 複数マシン** の順を推奨する。

今回の調査で比較表と文書を更新した。製品コード、設定、hook、稼働中のtmux環境は変更していない。以下の優先順位や構造は実装提案であり、採用済み仕様ではない。

最新安定版は **v0.9.5（2026-10-14）**。このプロジェクトは検出manifestをv0.9.0時点まで更新済みだが、未読完了・通知・worktreeなどには以前からの不足が残る。一方、9月9日にLAN/Androidミラー、QR pairing、送信・中断が実装されており、旧比較表の「remote未実装」だけでは現状を表せない。

## 1. 比較基準と調査方法

| 対象 | 固定した基準 | 用途 |
| --- | --- | --- |
| agentmux | `ff46711`（調査開始時HEAD） | 実装と文書の照合 |
| 前回追従 | `9e01168b140ce8e3821131345dc82bc2bf9994eb` | v0.9.0時点のmanifest更新元。`NOTICE`参照 |
| Herdr最新安定版 | `v0.9.5` / `2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01` | 今回の追従候補の基準 |
| Herdr master | `5608de0608918aba2f9d501e7793b984a0ca9cfa`（最終commit 2026-10-23 UTC） | 安定版外の候補の確認 |

公式release API、CHANGELOG、固定commitの検出manifest・検出処理・公式ドキュメントと、agentmuxのソースを照合した。manifestはTOMLとして読み、rule単位でも比較した。Herdrを起動しての操作比較、実agentによる最新UI再現、Android実機検証は今回の対象外。

Herdrはbundled manifestとは別にremote rule更新も適用するため、稼働中のv0.9.5が同じruleとは限らない。今回のmanifest差分は固定commitのbundled版が対象で、配信中catalogとの完全一致は未確認。

注意: masterとv0.9.5は単純な直列履歴ではない。GitHub compareは`diverged`（ahead 34 / behind 10）で、同じhotfixの別commitも含む。差分commit数をそのまま「v0.9.5以後の新機能数」として扱わない。masterのCHANGELOGのUnreleased欄は空だった。

根拠: [最新release](https://github.com/herdrdev/herdr/releases/tag/v0.9.5)、[release API](https://api.github.com/repos/herdrdev/herdr/releases/latest)、[安定版CHANGELOG](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/CHANGELOG.md)、[masterとの差分](https://github.com/herdrdev/herdr/compare/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01...5608de0608918aba2f9d501e7793b984a0ca9cfa)。

## 2. 前回から本家で増えた／改善されたもの

| 版 | 主な変化 | agentmuxへの意味 |
| --- | --- | --- |
| 0.9.1 / 09-16 | 保存SSHマシンへの`--machine` CLI転送、Letta検出・会話復元、Unicode入力編集、リモート通信・再接続の改善 | 複数hostへ操作を送るならmachine単位のtargetと接続管理が必要。Lettaは小さく切り出せる |
| 0.9.2 / 09-29 | agent自身のresume argv報告、machine status/reconnect、復元起動の間隔制御、event欠落の明示、初回task完了や検出・性能の修正 | stateとsession identityを分ける。event subscriberには欠落後の再同期が必要 |
| 0.9.3 / 09-29 | EscapeベースのOption・Shift+Enter等の入力修正 | 実paneへの入力はtmuxに任せる。agentmux独自入力経路だけを検証対象にする |
| 0.9.4 / 10-13 | plugin update、保存・shutdown・process枯渇対策、Ctrl+Z後のagent identity保持、検出・CPU改善 | pluginより先に監視・identityの信頼性を整える価値が高い |
| 0.9.5 / 10-14 | Codex turn hookの撤回と画面判定への回帰、named sessionのClaude質問検出、IME/prefix等のhotfix | Codexに古いhook前提を持ち込まない。Claude manifestは更新対象 |

これは各版の全変更の転載ではなく、このプロジェクトへの影響を選んだもの。複数マシンの一覧集約とclient独立表示はすでに**0.9.0**で導入されており、0.9.1以降は転送・再接続・性能が強化された。

根拠: [固定版CHANGELOG](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/CHANGELOG.md)の0.9.0〜0.9.5。

## 3. agentmuxの現在地

| 分野 | ソースで確認した現状 | 主な不足 |
| --- | --- | --- |
| tmux基盤 | 1つの選択socket内の全session/window/paneを列挙。PTY・layout・プロセス永続性はtmux所有 | `--socket-name`（`-L`）のみ。複数socket集約や`-S`指定はない |
| Sidebar | 通常paneの2段sidebarと通常windowのdashboard、priority表示、可視画面preview、mouse/keyboard操作 | fuzzy navigator、Git情報、worktree tree、表示設定保存、theme切替 |
| 検出 | 21 manifest、process/TTY/package path、pane options、画面とtitleによる判定 | 最新rule、Letta、local override、詳細な判定evidence |
| 状態 | snapshotごとの判定、typed state、session rollup、native statusのTTLとowner PID検証 | 継続的なtransition記録、window rollup、未読完了、clientごとの既読管理 |
| CLI/自動化 | list/JSON、explain、focus、spawn、send、mark、publish/withdraw、skillのpolling wait | versioned local socket API、semantic events、送信と一体のwait、agent世代の検証 |
| Editor integration | LazyAgent ACPのstate/transcript公開。LAN操作は公開元NeovimへのRPCを利用 | 同一pane内の複数ACP threadは独立agentとして列挙できない |
| LAN/Android | token認証、QR、一覧、SGR preview、target binding付き送信・中断 | SSH host一覧集約、接続profile、push通知、差分stream |
| Worktree/復元/plugin | tmuxと外部Git操作は利用可能 | agentmuxとしての管理機能は未実装 |

ソース: [`src/main.rs::Commands`](../../src/main.rs)、[`Tmux::snapshot` / `focus_pane` / `publish_agent`](../../src/tmux.rs)、[`Snapshot::build_spaces`](../../src/model.rs)、[`tui::run_loop`](../../src/tui.rs)、[`contrib/agentmux.tmux`](../../contrib/agentmux.tmux)、[`control::execute`](../../src/control.rs)、[`remote::serve` / `route`](../../src/remote.rs)、[`wait.py`](../../skills/agentmux/scripts/wait.py)。LANの境界は[Sidebar and mirror design](../design/sidebar-and-mirror.md)も参照。

重要な区別:

- `Done` enumとhookによる`done`報告は存在するが、**working→idleを未読完了として保持する機能はない**。画面manifest自体の出力はidle/working/blocked/unknown。
- `serve`は長時間動作するHTTPサーバーだが、全consumerが使うstatefulな監視daemonではない。TUI・CLI・HTTP requestはそれぞれsnapshotを取得する。
- control-mode watcherは出力の到着とtopology変更を知らせる。agent stateを保持・配信するevent APIとは異なる。
- LANの送信・中断はbinding検証とACP経路を持つが、CLI `send`は`Tmux::send_text`経由。全送信面が同じ保証を持つわけではない。
- `focus_pane --client`は`switch-client -c`を使うが、続くwindow/pane選択も含めた同一sessionのmulti-client独立性は今回未検証。

## 4. すぐに追従できる検出差分

本家v0.9.5には22個のbundled manifestがある。現行21個と比較すると、**13個は内容一致、7個は本家のrule更新、1個はagentmux独自差分、1個は新規**だった。

| manifest | 固定版との差 | 取り込み時の確認事項 |
| --- | --- | --- |
| Codex | trust/update画面、mention選択、弱いblocker、working fallbackの5 rule変更 | アニメーション無効・固定title・interrupt key変更・queued follow-up。入力中の引用文でblockedにしない |
| Claude | named session質問フォーム追加、workingのUnicode spinner修正 | 通常sessionとnamed sessionの双方で確認 |
| Cline | active turn、composer idle、inline許可・質問の4 rule追加 | launcher識別はprocess側も別途確認 |
| Kiro | 許可・質問・crew・working footer・promptを再構成 | OSC progressがない入力でもscreen fallbackが働くこと |
| Pi | working border追加とworking文字列修正 | idle画面や過去出力をworkingにしない |
| Antigravity | trust/許可/質問/取消footer、spinner修正 | promptへ戻った後のbackground taskだけでworkingにしない |
| Grok | spinner/title/tool待ち修正、background chipのworking rule削除 | agentが入力可能になった時点とbackground job完了を混同しない |
| Letta | 新規manifest | process matcher、canonical kind、package path、fixturesも必要。TOML追加だけでは検出されない |
| Copilot | 本家には`folder_trust_blocker`がない | これは既存のagentmux拡張。上書きで消さず維持する |

一致した13個: amp、cursor、devin、droid、gemini、hermes、kilo、kimi、maki、muse、opencode、qodercli、qwen。

比較元: [本家manifestディレクトリ](https://github.com/herdrdev/herdr/tree/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/src/detect/manifests)。比較先: [`src/detect/manifests`](../../src/detect/manifests)。

### 単純コピーでは埋まらない差

各manifestの`min_engine_version`は3以下で、使われるregionの種類も現行engineに存在する。これは静的な取り込み可能性であり、動作同等性の確認ではない。

現行engineは`visible_idle` / `visible_blocker` / `visible_working`を読み込むものの利用せず、`skip_state_update`をUnknownに変換する。本家は可視evidenceと前回stateを使い、根拠の弱いworking→idleを複数回確認する。ruleの同期だけで本家のstate安定性やdone通知が揃うわけではない。

`osc_progress`は現行入力では常に空。今回も取得可能性を実機確認していないため、tmuxからの取得を実装済みとも不可能とも断定しない。`pane_title`は使えるが、shell/editorによる上書き条件がある。既存のCRLF・Unicodeのbyte offset修正は維持し、engine全体の置換で戻さない。

根拠: [`src/detect/manifest.rs::Rule` / `detect` / `region`](../../src/detect/manifest.rs)、[本家engine](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/src/detect/manifest.rs)、[本家PendingIdleConfirmation](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/src/pane/agent_detection.rs)。

masterには[新しいHermes installer launcherの識別修正](https://github.com/herdrdev/herdr/commit/43b36b68caf5)もある。これは安定版manifestが同じでもprocess識別側に差がある例。必要なら固定commit単位で別途取り込む候補とする。

## 5. 主要な機能差とtmuxへの適応

### 未読完了・通知

本家の`done`は単なる終了コードではない。入力可能なidleになった完了を未読として扱い、閲覧でidle表示になる。CLI/API側のseenと各TUI clientのseenは区別される。readは既読化せず、明示focusはserver側の既読化を伴う。startup、restore、会話切替を完了に数えないための`completion_seq`もある。

agentmuxでは、実行世代と完了seqを共有監視側で持ち、表示clientの既読状態を別に扱う設計が適する。LANの定期preview取得を「人が見た」と誤認しないこと、sidebarを閉じても完了を検知できることが必要。通知はこの基盤の後に追加し、blocked/doneの重複抑止とactive pane抑止を行う。

根拠: [固定版agent automation / Choose the control surface](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/agent-automation.mdx)。

### 送信・wait・events

本家はlayout、raw pane、recognized agentを分けている。`agent prompt --wait`は送信前のidleを完了として返さず、活動を観測してからsettled stateを待つ。一方で個別turnの完了を厳密に相関するものではなく、すでにworkingならそのturnの終了が条件を満たす場合がある。既にblockedのagentへのpromptは入力を送らず拒否する。

agentmuxのpolling helperはstandalone状態待ちとして使えるが、送信直後の古いidle/doneを拾う可能性や、短い遷移の見逃しがある。共有observerでstate seqとagent generationを持ち、sendとwaitの起点を結び付けるのが次の段階。LANのbinding guardとCLIの挙動も揃える。

event配信では「購読開始→snapshot→seqで整合」の手順と、遅いreaderへの欠落通知・再同期が必要。本家0.9.2は`events_lost`を明示する。現行watcherのqueue満杯時の通知破棄は再snapshotのきっかけとしては成立するが、losslessなlifecycle APIにそのまま転用できない。出力待ちは本家でもsnapshot pollingであり、すべてを出力streamへ置き換える必要はない。

根拠: [固定版automation](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/agent-automation.mdx)、[CHANGELOG 0.9.0 / 0.9.2](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/CHANGELOG.md)、[`watcher::notify`](../../src/watcher.rs)、[`wait.py`](../../skills/agentmux/scripts/wait.py)。

### Integrationsと会話復元

hookはstate報告とsession identity報告を区別する。本家v0.9.5のCodex・Claude・Copilotは画面判定を使い、integrationは復元用identityを報告する。Pi/OMP/OpenCode等にはstateも報告するintegrationがある。agentmuxのCopilot hookとLazyAgent ACPは独自のstate報告経路なので、本家の設定で一括上書きしない。

0.9.2のself-reported resume argvは、agentmuxにも応用できる。state authority、会話ID、resume argv、表示metadataを分け、owner・seq・終了時releaseでstale reportを防ぐ。ただし既存paneへの再接続と、再起動後の新processによる会話再開は別の保証になる。

根拠: [固定版integrations](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/integrations.mdx)、[agent作者向けprotocol](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/add-herdr-support.mdx)、[`contrib/copilot-hooks.json`](../../contrib/copilot-hooks.json)。

### Worktree・navigator・復元

Git worktreeを作成／openし、tmux sessionと関連付け、branch・dirty・ahead/behindと親子関係を表示するのは「モダンな開発環境」として効果が大きい。pane/layoutはtmux nativeを使い、Git取得は非同期cacheにする。本家にも遅いGitでUIが止まる問題や、open時に親workspaceを誤って再利用する修正がある。

restoreは既存のtmux-resurrect等との役割を確認してから追加する。agentmuxはagent metadata・会話参照を補完できるが、tmux topologyを保存する仕組みをもう1つ常時動かすかは未決定。detach時のlive process維持、server/machine再起動後のlayout復元、会話再開をそれぞれ検証する。

根拠: [固定版session state](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/session-state.mdx)、[worktree API](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/socket-api.mdx)、[CHANGELOG 0.9.1〜0.9.4](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/CHANGELOG.md)。

### 複数マシン・plugin

LANミラーは1台のtmuxを薄いclientから使う機能。本家のsaved SSH machinesは複数hostのagentを1つの一覧に統合し、hostごとに接続・再接続・CLI転送する機能。agentmuxで実装するなら`machine + socket/server instance + pane ID + agent generation`をtargetの基礎にし、接続失敗時にLocalへ誤送信しない。単なる`%pane_id`はhost/socketをまたぐと衝突する。

pluginは本家に既存機能があり、0.9.4でGitHub由来pluginのupdateが追加された。tmux向けには、最初はcustom commandとevent hookで足りるかを確認する。Herdr pluginの環境変数・API・pane ownershipまで互換にするかは別の大きな判断で、今回parityの必須条件とは決めない。

根拠: [固定版connecting machines](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/connecting-machines.mdx)、[固定版plugins](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/docs/next/website/src/content/docs/plugins.mdx)。

## 6. 推奨する実装の切り方

規模は相対的な目安。日数見積もりではない。既存計画のP0〜P9とは別に、今回の追従順序を示す。

| 順 | 成果物 | 規模 | 依存・受入条件 |
| --- | --- | --- | --- |
| 1 | 7 manifest更新＋Letta追加、process検出の差分確認 | 小〜中 | 最新UI fixture、Copilot拡張とCRLF修正維持、OSCなしfallback。NOTICEも更新 |
| 2 | socket単位の共有observer、state/identity/generation、詳細explain、local override | 大 | TUI/CLI/LANが同じ状態を読む。owner終了・TTL・再起動・取りこぼし後の再同期、pane数別の負荷測定 |
| 3 | unseen done、通知、CLI/LAN共通のagent入力、sendと結び付いたwait | 中〜大 | 2に依存。古いidle誤完了なし、startup誤通知なし、既読をclient間で混同しない、stale target拒否 |
| 4 | worktree CRUD、Git cache、fuzzy navigator、integration installerと会話参照 | 中〜大 | UIを止めないGit、dirty削除拒否、設定mergeで他hook保持、restore担当の整理 |
| 5 | SSH machine集約・CLI転送、差分配信、custom command／plugin拡張 | 大 | host/socketを含むID、独立reconnect、Localへの誤fallbackなし。2・3のAPIを再利用 |

2の共有監視をdaemonとして実装する案は既存計画と整合する。ただし常駐方式、既読の保存範囲、API schemaは未確定。まず単一socketの状態保持とconsumerの共有を小さく設計し、PTY所有やterminal描画は引き続きtmuxに任せる。

tmux nativeのsplit/layout/copy-mode/SSH attachや、paneへ流れるKitty/IME/keyboard処理をHerdr式に再実装する優先度は低い。agentmux自身のTUI入力、preview、LAN/ACP操作に現れる差を対象にする。

## 7. 文書の修正と未検証事項

古い文書に残るpopup前提、`-S`対応済み相当の記述、LAN機能とSSH集約の混同を[parity matrix](../parity-matrix.md)で補正した。[実装計画](../implementation-plan.md)は当初の計画として残し、今回の比較基準と優先順へのリンクを付けた。

次の実装開始時に確認する点:

- 実agent最新版の画面fixtureと、animation/title設定差による検出結果。
- 複数tmux clientが同じsessionを見ている時のfocus・既読・通知の所有権。
- Ctrl+Z/fgや同じTTY内のbackground agentを区別するforeground process情報。現行はTTY内process走査であり、exact process世代の保持はない。
- control-modeのoutput情報をOSC取得や差分streamに使う場合のtmux版・escape処理・backpressure。
- tmux-resurrect等が担当するtopology復元とagentmuxの会話参照の連携。
- Android実機での接続、入力、通知。既存LAN mirrorの存在はSSH集約やpush通知の検証を意味しない。

調査中の確認: `cargo test --lib --offline`は41件成功。Herdr本体のruntime parityや新規manifestの実動作を保証するものではない。
