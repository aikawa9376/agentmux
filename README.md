# agentmux

tmux 上で動く複数の coding agent を、Herdr のサイドバーに近い操作感で一覧・監視・操作する Rust 製ツールです。tmux の session / window / pane と永続 PTY はそのまま利用します。

現在は最初の usable MVP です。全 tmux session の agent、状態、場所を横断表示し、選択 pane の live preview と主要操作を提供します。UIはHerdrと同じCatppuccin Mocha基調の状態色・spinner・フラットな2段sidebarを採用しています。

## Implemented

- `spaces` と `agents` の2段サイドバー
- blocked → done → working → idle → unknown の優先表示
- session/window/pane を横断した一覧と adaptive preview
- agentごとのrepo名・branch・作業名の2行表示、選択中のroot/cwd/tmux位置の詳細表示
- ANSI色・文字装飾・改行境界・cursor位置を保つvisible-screen preview
- 通常paneのtmux履歴とACP公開ログのスクロール、履歴閲覧中の表示位置固定と末尾での自動追従復帰
- blocked=赤、working=黄spinner、done=teal、idle=緑のHerdr式状態アイコン
- foreground command、TTY process、argv0、Node/Python package path、pane options による agent 識別
- 識別済みagentの画面下端による状態推定（画面本文はagent識別に不使用）
- `@agent_*` pane option と既存 LazyAgent integration の互換
- focus、spawn、send、respawn、kill、mark/unmark
- private PTY 上の tmux control-mode によるイベント更新
- mouse click/wheel、keyboard navigation、狭いpopup向け単一カラム表示
- human-readable output と `list --json`
- tmux socketごとの共有observer、未読完了、client別既読、背景通知
- 共通のagent送信経路、`send --wait`、`wait`、sequence付きstate events
- `ui --popup` でtmuxのフロート表示（通常pane/window表示も利用可能）
- TOML config。fzf、curl、util-linux `script` は不要

## Build and install

Rust 1.85 以上と tmux が必要です。

```bash
cargo test --all-targets
cargo install --path .
```

tmux 設定には [contrib/agentmux.tmux](contrib/agentmux.tmux) を source します。

```tmux
source-file ~/workspace/agentmux/contrib/agentmux.tmux
```

- `M-a`: 右側に通常paneとしてsidebarを開く
- `prefix a`: 通常windowとして広いdashboardを開く
- `prefix A`: フロートpopupでdashboardを開く

引数からも開けます。幅・高さは割合またはcell数です。popupは`Enter`で選択先へ移動して閉じ、`q` / `Esc`でも閉じます。paneの分割配置は変わりません。

```bash
agentmux ui --popup
agentmux ui --popup --width 90% --height 85%
```

clientが複数あり呼び出し元を特定できない場合は`--client`とorigin paneを指定します。tmux bindingは両方を自動で渡します。

設定を反映したら `tmux source-file ~/.config/tmux/tmux.conf` を実行してください。

tmuxの`source-file`は差分適用です。設定から削除した古いbindingは自動では消えないため、[contrib/agentmux.tmux](contrib/agentmux.tmux) は旧agentmux bindingを明示的に`unbind`してから再定義します。

## Sidebar keys

| Key | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | spaces / agents / preview panel の切替（preview非表示時は2つ） |
| `j`, `k`, arrows | 一覧では選択移動、previewでは1行スクロール |
| `PageUp` / `PageDown` | previewを1ページスクロール（どのpanelからも操作可能） |
| `Ctrl-u` / `Ctrl-d` | preview選択中、半ページ上 / 下へスクロール |
| `Home` / `g`, `End` / `G` | preview選択中、履歴先頭 / 最新出力に戻って自動追従 |
| マウスホイール | preview上では3行スクロール、一覧上では選択移動 |
| `Enter` | 選択先をfocus。通常pane/windowでは継続、popupでは閉じる |
| `a` | agent-only / all panes の切替 |
| `r`, `Ctrl-g` | 即時 refresh |
| `Alt-c` / `Alt-l` / `Alt-o` | Codex / Claude / OpenCode を split 起動 |
| `Alt-n` | custom command を split 起動 |
| `Ctrl-s` | prompt を送信 |
| `Ctrl-r` | pane を確認付き respawn |
| `Ctrl-k` | pane を確認付き kill |
| `Ctrl-m` | agent metadata を設定 |
| `Ctrl-u`（一覧選択中） | 確認後にagent登録を解除。`y`で確定、`Enter` / `Esc` / `n`で取消 |
| `q`, `Esc` | sidebar を閉じる |

agent-only表示はagentが0件でも通常paneへ切り替わりません。`a`で明示的にall panesへ切り替えられます。別windowへ移動した場合、agentmuxは元のwindowに残ります。

agent一覧は1行目に状態アイコン・repo名・branch、2行目にagent名と取得できた作業名を表示します。作業名は有効な公開status messageを優先し、なければterminal titleを使います。汎用的な状態名やNeovimなどのeditor titleは省きます。長い名前は省略し、選択中のrepoルート・cwd・tmux位置は画面下部に表示します。

Git情報はpaneのcwdから読み取り専用で取得し、約5秒ごとに更新します。Git管理外や取得できない場合はディレクトリ名だけを表示します。Neovim内のACP agentでもroot/cwdはpaneの位置であり、会話固有の作業ディレクトリを保証するものではありません。

## Android / LAN mirror

同梱の[Androidアプリ](android/README.md)からagent一覧と画面を閲覧できます。PC側で `agentmux serve --bind 0.0.0.0:9876` を起動し、Androidで表示されたQRをカメラまたは保存画像から読み取ると接続できます（IPとトークンの手入力も可能）。一覧の開閉、命令送信、中断、最新出力への追従ができ、ブラウザからも利用できます。

## CLI compatibility

既存 Bash 版と同じ主要コマンドを維持しています。

```bash
agentmux list
agentmux list --json
agentmux preview reviewer
agentmux explain %25
agentmux explain %25 --json
agentmux focus reviewer
agentmux spawn codex ~/project
agentmux send reviewer --text "run the failing tests"
agentmux send reviewer --text "run the failing tests" --wait --timeout 120000
agentmux wait reviewer --until done --until blocked --timeout 120000
agentmux events
agentmux mark %25 --kind codex --name reviewer
agentmux status %25 blocked "approval required"
agentmux status %25 working "prompt sent" 15
agentmux clear-status %25
agentmux unmark %25
agentmux respawn %25
```

Editor integration向けには、識別情報と状態を一度に更新する`publish`と、ownerが一致する場合だけ解除する`withdraw`があります。LazyAgent ACPはこのinterfaceを自動利用します。

```sh
agentmux publish "$TMUX_PANE" --kind copilot --name "Copilot (ACP)" \
  --state working --message "Thinking..." --owner lazyagent --owner-pid "$$" \
  --preview-path /path/to/live-transcript.log
agentmux withdraw "$TMUX_PANE" --owner lazyagent
```

Target には `%pane_id`、`session:window.pane`、一意な agent name/kind を指定できます。曖昧な名前はエラーになります。

`send`は現在のagent identityを再検証し、terminalではpaste、LazyAgent ACPでは公開元へのRPCを使います。blocked中は送信を拒否するので、実paneで内容を確認して応答してください。`send --wait`は送信後の活動を確認してからidle/done/blockedを待ち、古いidleやdoneでは完了しません。待機結果はJSONです。timeoutの単位はミリ秒。timeoutや応答喪失後は、再送前にpaneを確認してください。

### Shared state and notifications

CLI/UI/LANは同じobserverを自動起動して利用します。observerはUIを閉じても監視を続け、tmuxのprocessやPTYは所有しません。

```bash
agentmux daemon status
agentmux daemon stop
agentmux daemon start
```

workingからidleになった完了を`done`として保持します。単なるstartupや会話paneの初回idleは完了に数えません。`list`やpreviewの読み取りは既読にせず、明示的な`focus`はCLI側の既読を更新します。TUIのclient別既読は別に管理するため表示が異なる場合があります。完了seq・generation・CLI側の既読はobserver再起動をまたいで保存します。

既定ではbackgroundのdone/blockedをtmuxのmessageで通知し、表示中のpaneでは抑止します。Linuxのsystem通知は`notifications.system = true`で有効にできます。監視・通知設定とlocal manifest変更はobserverのstop/start、UI設定はUIの開き直しで反映します。

`events`は初回snapshotに続きNDJSONのstate eventを出力します。各eventにsequenceがあり、保持上限を超えた場合は`events_lost`、observerの再起動時は`observer_restarted`となります。再開方法とIPC protocolはObsidianの[設計文書](obsidian://open?vault=obsidian&file=notes%2Fprojects%2Fagentmux%2Fdocs%2Fdesign%2Fobserver-and-popup)を参照してください。

誤検出を調べる場合は `agentmux explain <target>` を使います。agentの識別根拠と状態判定の根拠を別々に表示します。nvimなどeditor内のintegrationは `@agent_kind` と `@agent_status` をpublishすることで検出され、通常のbuffer本文にagent名が書かれているだけでは検出されません。

## Agent orchestration skill

[`skills/agentmux`](skills/agentmux/SKILL.md) に Codex/LazyAgent 向けの orchestration skill があります。pane分割・window/session・layout・process実行は tmux に任せ、agentmux は agent の検出、状態、曖昧性のないtarget解決、prompt送信を担当します。agentmux本体に split wrapper は追加していません。

```sh
mkdir -p "${CODEX_HOME:-$HOME/.codex}/skills"
ln -s "$PWD/skills/agentmux" "${CODEX_HOME:-$HOME/.codex}/skills/agentmux"
```

skillは現在paneを `$TMUX_PANE` で保護し、`tmux split-window -d` でfocusを奪わず sibling agentやtest serverを起動します。`scripts/wait.py` で将来のpane出力またはagentmuxのsemantic stateをtimeout付きで待機できます。

### GitHub Copilot CLI state

Copilot CLIはprocessだけではworking/doneを安定して公開しないため、公式hookを使います。[contrib/copilot-hooks.json](contrib/copilot-hooks.json)を`~/.copilot/hooks/agentmux.json`として配置するか、同じ`hooks`オブジェクトを`~/.copilot/settings.json`へmergeし、Copilotを再起動してください。

- `userPromptSubmitted`: working
- `permissionRequest`: blocked
- `postToolUse`: working
- `agentStop`: done
- trust dialog / prompt: screen fallbackでblocked / idle

別の tmux socket name を使う場合:

```bash
agentmux --socket-name work list --json
```

## Configuration

既定パスは `~/.config/agentmux/config.toml` です。完全なdefaultを表示できます。

```bash
agentmux --default-config
```

```toml
[ui]
workspace_ratio = 0.38
preview_min_width = 120
show_plain_panes = false

[refresh]
preview_ms = 50
status_ms = 250

[notifications]
tmux = true
system = false
```

UI全体の幅が`preview_min_width`未満ならpreviewを描画せず、spaces/agents sidebarだけを全幅表示します。通常のlive previewはtmuxの現在のvisible cell gridを`capture-pane -e`で取得し、foreground/background color、太字、italic、underline、元の改行境界、cursor位置を復元します。再wrapは行いません。agentmux自身は背景色を指定せず、tmuxのpaneまたはpopupの背景を継承します。

上へスクロールすると、通常paneはtmuxに残っている履歴、ACPは公開ログ全体をその時点で読み込みます。`history`表示中はその内容を固定し、新しい出力でも読んでいる場所を動かしません。末尾まで下へスクロールするか、previewで`End` / `G`を押すと最新出力への追従を再開します。agentや公開ログの切替時もliveへ戻ります。履歴の読み取りは対象paneの表示位置や入力を変更しません。tmuxが保持しない全画面アプリ内部の履歴や、既に削除されたログは取得できません。

非表示のpreviewは取得しません。状態は出力イベントで更新し、無出力時も `max(status_ms, 2000)` ミリ秒間隔で再確認するため、pane optionの変更・TTL期限切れ・watcher停止からも追従します。

これはcell snapshotの同期表示であり、tmux paneそのものの複製ではありません。sixel/kitty画像、OSC hyperlink、IME、application固有のcursor形状、非常に短い出力後の空白領域などは完全には再現できません。

## Current boundary

Herdr v0.9.5を基準とする22個のagent別manifestで画面状態を判定します。Copilotのfolder trustとCodex旧timer区切りへの互換拡張を維持しています。local overrideは`~/.config/agentmux/agent-detection/<kind>.toml`（`XDG_CONFIG_HOME`対応）。不正なoverrideはbundledへfallbackし、`explain`がsource/version・各ruleの一致・regionと警告を表示します。OSC progressの入力は未取得です。

対象は検出・共有監視・未読完了・通知・agent操作です。Git/worktree管理と複数SSHマシン集約は対象外とし、既存のGit・SSH/tmux workflowを利用します。tmux自身が提供するPTY、session永続化、layout、copy-modeもtmuxに任せます。LAN向けのthin clientは `android/` と `web/` に実装しています。

設計・調査・計画はObsidianで管理しています。[資料一覧](obsidian://open?vault=obsidian&file=notes%2Fprojects%2Fagentmux%2Fdocs%2Findex)（vault内の `notes/projects/agentmux/docs/index.md`）を参照してください。

ライセンスは `AGPL-3.0-or-later` です。状態判定エンジンはHerdr v0.7.3由来で、manifestの更新元・個別ライセンスを含めた固定した派生元は [NOTICE](NOTICE) に記録しています。ロゴ、音源などのブランドassetは含めていません。
