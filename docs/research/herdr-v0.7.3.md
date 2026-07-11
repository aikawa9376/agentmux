# Herdr v0.7.3 機能調査

最終更新: 2026-07-10

## 1. 結論

Herdr は「agent の一覧サイドバー」だけではなく、独自の PTY、terminal emulator、background server、thin client を持つ完全な terminal multiplexer である。中心的な価値は次の組み合わせにある。

1. 実プロセスが動く永続 pane
2. session / workspace / tab / pane の階層
3. coding agent の自動検出と `blocked / working / done / idle / unknown` の状態管理
4. workspace と tab への状態 rollup
5. mouse-first の sidebar と検索 navigator
6. agent 自身も利用できる CLI、local socket API、wait、event subscription
7. agent hook/plugin、native session resume、Git worktree、通知、remote attach、plugin host

agentmux では terminal multiplexer を再実装しない。Herdr の利用者向け挙動を移植し、PTY、layout、attach/detach、scrollback、multi-client、SSH 上の永続性は tmux に委譲する。この境界なら Herdr の主要価値を tmux ユーザーへ持ち込める。

## 2. 調査範囲と根拠

- 安定版: [Herdr v0.7.3](https://github.com/ogulcancelik/herdr/releases/tag/v0.7.3)
- 調査した master: [`c8850a2e3e284033142db0f8117fa5998c68bd59`](https://github.com/ogulcancelik/herdr/commit/c8850a2e3e284033142db0f8117fa5998c68bd59)
- 公式: [概要](https://herdr.dev/)、[concepts](https://herdr.dev/docs/concepts/)、[agents](https://herdr.dev/docs/agents/)、[configuration](https://herdr.dev/docs/configuration/)、[keyboard](https://herdr.dev/docs/keyboard/)
- 運用: [session state](https://herdr.dev/docs/session-state/)、[remote](https://herdr.dev/docs/persistence-remote/)、[integrations](https://herdr.dev/docs/integrations/)
- 自動化: [CLI](https://herdr.dev/docs/cli-reference/)、[socket API](https://herdr.dev/docs/socket-api/)、[plugins](https://herdr.dev/docs/plugins/)
- tmux 側の実現根拠: [tmux control mode](https://github.com/tmux/tmux/wiki/Control-Mode)、[tmux formats](https://github.com/tmux/tmux/wiki/Formats)
- ローカル基準実装: `~/.config/tmux/scripts/agentmux` と同 README、dotfiles commit `1e4d5540`

Web ページの更新は v0.7.3 tag より進んでいる箇所があるため、安定版の機能に加え、master `c8850a2` で確認できた unreleased の copy-mode search も「追随候補」として区別した。

## 3. 概念の tmux 対応

| Herdr | tmux / agentmux | 方針 |
| --- | --- | --- |
| named session namespace | tmux server/socket (`-L` / `-S`) | default socket を基本とし、任意 socket を選択可能にする |
| workspace | tmux session | project/repository 単位。状態を session へ rollup |
| tab | tmux window | label、並び替え、active/zoom 状態を利用 |
| pane | tmux pane | `%pane_id` を canonical ID とする |
| terminal runtime / PTY | tmux server | agentmux は所有しない |
| Herdr client | tmux client + agentmux TUI | 呼び出した client のみ focus を切り替える |
| sidebar | 左寄せ tmux popup 内の Ratatui TUI | 通常 layout を壊さない command-invoked drawer |
| local socket API | agentmux daemon の Unix socket | tmux options との互換層も維持 |

この対応は Herdr の階層を失わない。特に `Herdr workspace = tmux session`、`Herdr tab = tmux window` であり、workspace を window に縮めないことが重要である。

## 4. UI と sidebar

### 4.1 expanded sidebar

Sidebar は上下2セクションである。

- Spaces/workspaces
  - workspace 番号、名前、集約状態の色と marker
  - Git branch、upstream の ahead/behind
  - Git worktree の親子 tree、expand/collapse
  - active/selected highlight、scrollbar、drag reorder
- Agents
  - 全 workspace の agent を表示
  - `workspace · tab`、custom agent name、agent kind
  - semantic state、custom status label、spinner/attention icon
  - `spaces` 順または `priority` 順。priority は blocked、done、working、idle、unknown。同順位は直近の状態変更順
  - click で agent pane へ移動

幅は手動 resize でき、min/max/default width を設定できる。上下セクションの divider も drag でき、比率と幅は保存される。compact collapse と完全 hidden collapse があり、single tab 時の tab bar 非表示も設定できる。

### 4.2 responsive/mobile

狭い terminal では single-column の switcher に変わる。agent-first summary、worktree tree、workspace/tab/pane の切替を提供し、SSH mobile client からも操作できる。閾値は設定可能である。

### 4.3 mouse と keyboard

- pane、tab、workspace、agent の click focus
- split border、sidebar width、sidebar section、scrollbar の drag
- workspace/tab の drag reorder
- terminal text の drag selection/copy と autoscroll
- workspace/tab/pane の right-click menu
- URL の modified click、plugin link handler
- tmux-style prefix。既定は `ctrl+b`
- navigate、resize、copy、terminal、prefix の各 mode
- `prefix+?` の live keybinding help、settings、onboarding、release notes
- `prefix+g` の searchable navigator。workspace/tab/pane tree、agent state filter、mouse/keyboard selection

主な pane 操作は focus、cycle、last pane、split right/down、swap、resize、zoom、move、rename、close。tab/workspace は create、focus、rename、reorder、close を備える。

### 4.4 既定 keymap

Prefix は `ctrl+b`。主要な default は次のとおりである。

| 領域 | 操作 | Key |
| --- | --- | --- |
| global | help / settings / detach | `prefix+?` / `prefix+s` / `prefix+q` |
| global | config reload / notification target | `prefix+shift+r` / `prefix+o` |
| navigation | workspace picker / searchable navigator | `prefix+w` / `prefix+g` |
| workspace | new / rename / close | `prefix+shift+n` / `prefix+shift+w` / `prefix+shift+d` |
| worktree | new | `prefix+shift+g` |
| tab | new / previous / next / 1..9 | `prefix+c` / `prefix+p` / `prefix+n` / `prefix+1..9` |
| tab | rename / close | `prefix+shift+t` / `prefix+shift+x` |
| pane | focus | `prefix+h/j/k/l` |
| pane | swap | `prefix+shift+h/j/k/l` |
| pane | cycle next/previous | `prefix+tab` / `prefix+shift+tab` |
| pane | split right/down | `prefix+v` / `prefix+minus` |
| pane | rename / close / zoom / resize | `prefix+shift+p` / `prefix+x` / `prefix+z` / `prefix+r` |
| pane | copy mode / edit scrollback | `prefix+[` / `prefix+e` |
| sidebar | toggle | `prefix+b` |

Action ごとに複数 binding、direct modified chord、workspace/tab/agent の indexed 1..9 jump、pane/background/plugin の custom command を設定できる。未割当 action も help に表示できる。

Copy mode は `h/j/k/l`、`w/b/e`、`{`/`}`、page motion、`v`/Space の選択開始、`y`/Enter の copy、`q`/Esc の終了を持つ。master `c8850a2` では `/`・`?` の smart-case literal search、`n/N`、match highlight が unreleased で追加されている。

### 4.5 Context menu

| 対象 | Action |
| --- | --- |
| workspace | Rename、Close |
| Git workspace | Rename、Close、New worktree、Open worktree |
| worktree parent | Close group、New/Open worktree、Expand/Collapse |
| worktree child | Rename、Close、Delete worktree checkout |
| tab | New tab、Rename、Close |
| pane | Rename/Clear name、Swap with focused、Split right/down、Zoom、Close |

### 4.6 agentmux での UI 差分

Herdr の中央領域は本物の pane renderer だが、agentmux の drawer 内では選択 pane の `capture-pane` preview を表示する。Enter/focus/attach で tmux の本物の pane に戻る。これは terminal renderer を二重実装しないための意図的な適応である。

既定 UI は次を想定する。

```tmux
display-popup -E -x 0 -y 0 -w 42% -h 100% "agentmux ui --client '#{client_name}' --origin '#{pane_id}'"
```

狭い画面は一覧のみ、十分な幅では左に tree、右に live preview を置く。expanded/compact/hidden の状態、sidebar 幅、上下比率、filter は保存する。

## 5. Agent の状態モデル

### 5.1 semantic state

| 状態 | 意味 |
| --- | --- |
| `blocked` | 入力、承認、質問への回答、判断が必要 |
| `working` | turn、tool、subagent 等を実行中 |
| `done` | 作業が完了したが、まだ利用者が対象 pane を確認していない |
| `idle` | 入力待ちまたは完了済みで確認済み |
| `unknown` | agent は識別できるが状態を確信できない |

既存 Bash 版の `dead` と通常の `pane` は agentmux の表示分類として残すが、agent semantic state と混同しない。`done` は integration から直接届く状態だけではなく、background の `working/blocked -> idle` と `seen` 情報から導出する。

### 5.2 authority

1 pane に複数の真実を競合させず、次の authority を使う。

1. active な full-lifecycle integration report
2. foreground process + screen manifest + OSC evidence
3. known agent の安全な fallback
4. manual mark/process-only fallback

full-lifecycle report が active な間は screen fallback を併用しない。session ID しか報告しない hook は state authority にならず、screen manifest を使い続ける。

### 5.3 process と screen detection

- pane foreground process、process group/tree、TTY 上の wrapper/child process を調べる
- tmux pane の live bottom buffer を `capture-pane -pJ` で取得する
- scrollback 中の client viewport ではなく live bottom を判定する
- TOML manifest の region/matcher/gate を評価する
- permission/question UI の強い evidence がある場合だけ `blocked` にする
- rule がない known agent は安全側の `idle/unknown` にする
- `HERDR_AGENT` 相当の明示 hint と manual kind を用意する
- manifest は bundled、cached remote、local override の順序と version/validation を持つ
- `agentmux agent explain` で authority、manifest source/version、matched rule、evidence、fallback reason を出す

状態は pane から window/session へ rollup する。attention priority は `blocked > done > working > idle > unknown` とし、表示用 custom label は wait/notification/rollup を変えない。

### 5.4 対応 agent

| authority / capability | Agent |
| --- | --- |
| lifecycle state + native resume | Pi、OMP、Kimi Code CLI、OpenCode、Kilo Code CLI、Hermes Agent、MastraCode |
| screen state + hook session identity/resume | Claude Code、Codex、GitHub Copilot CLI、Devin CLI、Droid、Qoder CLI、Cursor Agent CLI |
| screen detection | Amp、Grok CLI、Antigravity CLI、Kiro CLI |
| detected but upstream testing が薄い | Gemini CLI、Cline |

未知の agent は通常 pane として動かせ、manual mark または status API で agent 化できる。

## 6. Agent 操作

- list/get と target resolution
- visible/recent/recent-unwrapped/detection の read
- text/key/atomic command send
- focus、rename/clear、start、attach/takeover
- semantic status wait と timeout
- state explain
- custom lifecycle report、session identity report
- display-only metadata report
- pane agent authority の release/clear

Target は `%pane_id`、一意な agent name、agent kind、`session:window.pane` を受け付ける。曖昧な label は error にし、勝手に最初の pane を選ばない。

## 7. tmux / terminal 管理機能

Herdr が独自実装している次の機能は tmux の command と server に委譲する。

- session/window/pane create/list/get/focus/rename/close
- split ratio、directional neighbor/focus/swap/resize、zoom
- running pane を別 window/session へ move
- layout snapshot/export/apply
- scrollback、copy mode、search、ANSI capture
- attach/detach、multi-client、remote SSH attach
- pane process info、cwd、foreground cwd
- pane input、bracketed paste、mouse routing、terminal key encoding
- pane title、border label、status/tab bar

agentmux が補うのは agent-aware label/state、cross-session navigator、drawer、automation API、metadata、notification である。tmux の通常 keybinding、copy-mode、terminal emulator と競合しないことを要件にする。

## 8. Git worktree

- session row から branch を指定して worktree を作成
- local branch があれば checkout、なければ base/HEAD から作成
- `<root>/<repo>/<branch-slug>` の既定配置と明示 path
- 既存 checkout の一覧/open。すでに開いていれば focus
- parent session と child worktree session の tree grouping
- group collapse/expand/close
- checkout 削除は明示操作。まず `git worktree remove`、dirty の強制削除は再確認
- branch 自体は削除しない

tmux session option に group/origin metadata を持たせ、daemon の永続 state にも保存する。

## 9. 永続化、復元、remote

Herdr は4種類を区別する。

1. live persistence: server が生きたまま client を detach/reattach
2. snapshot restore: workspace/tab/pane/cwd/layout/focus を再構築
3. optional screen-history replay
4. native agent session resume と experimental live server handoff

agentmux では 1 は tmux native である。2 は tmux topology と agent metadata の snapshot、3 は tmux history、4 の agent resume は hook の session reference と command template で実装する。tmux server 自体の live handoffは実装せず、tmux が所有する live processを維持したまま agentmux daemon/binary だけ再起動する。

Remote は段階的に次を扱う。

- SSH して remote tmux/agentmux を使う基本経路
- `agentmux --remote host` thin-client 経路
- SSH config/keepalive、remote binary version check/bootstrap
- local keybinding snapshot または server keybinding
- image clipboard bridge は後期 parity 項目
- native Windows は tmux 依存のため対象外。WSL は対象にできる

## 10. CLI と local API

Herdr の主要 command surface は以下である。

- runtime/status/update/channel/completion/api schema
- server start/stop/reload/config/manifest status/update
- session、workspace、worktree、tab、pane
- agent、terminal attach/observe/control
- wait output/agent-status
- notification
- integration install/uninstall/status
- plugin install/link/list/enable/disable/action/log/pane/config-dir

Socket API は newline-delimited JSON request/response と long-lived subscription を持つ。workspace/tab/pane/worktree/agent lifecycle、agent status、output match、layout、scroll の event がある。initial snapshot 後に subscription することで race を避ける。

agentmux も CLI を薄い socket client とし、machine output は versioned JSON、human output は明示的に分ける。Herdr と同じ method 名を参考にするが、tmux 固有 target と protocol version を持つ独立 API にする。

## 11. 通知

- `blocked` と unseen `done` を対象
- state が delay 経過後も同じ場合だけ発火
- active pane/window では抑制し、background または unfocused client で通知
- drawer 内 toast、outer terminal notification、OS notification、off
- done/request sound、custom mp3、agent 別 on/off
- notification target へ jump
- clipboard copy feedback は background agent notification と分離

既存 tmux client が複数あるため、notification の active 判定と delivery は client 単位で行う。

## 12. 設定と運用

- TOML config、default config 出力、validation、unknown key warning、hot reload
- configurable keybindings と複数 binding、custom command/plugin action
- drawer width/min/max/collapse、mobile threshold、mouse、sort、confirmation
- themes、terminal palette、light/dark auto switch、custom colors
- shell/cwd policy は tmux default-command/default-path と衝突しない範囲で wrapper に適用
- logs の rotation、runtime/client/daemon status、protocol compatibility
- update channel/version check/manifest check、shell completion
- first-run onboarding、settings UI、integration freshness badge、release notes

Herdr の Kitty graphics、host cursor/IME、OSC color、terminal image renderer 等は tmux/outer terminal の責務である。agentmux TUI 自身の CJK width、IME 入力、mouse、truecolor は検証する。

## 13. Plugin host

Plugin v1 は `agentmux-plugin.toml` と任意言語の argv command からなる process-out-of-process model とする。

- metadata、minimum agentmux version、platforms、build commands
- manifest actions、event hooks、managed terminal panes、link handlers
- local link/unlink と GitHub shorthand install/uninstall
- enable/disable、action invoke、bounded logs
- plugin config/state directory と invocation context/env
- placement: popup overlay、split、window、zoomed pane
- user confirmation と source/ref/commit preview
- sandbox ではないことを明示

Plugin command は socket API/CLI の全機能を使える。runtime action registration と native widget injection は v1 の対象外にする。

## 14. 既存 Bash 版 agentmux

この workspace は空だが、dotfiles にすでに動作中の基準実装がある。

| 項目 | 現状 |
| --- | --- |
| UI | fzf popup、agent-first list、右側 live preview |
| command | `menu/list/preview/focus/spawn/send/mark/unmark/respawn/status/clear-status` |
| actions | focus、Codex/Claude/OpenCode/custom spawn、send、respawn、kill、mark/unmark |
| detection | foreground command、TTY process args、pane options、visible TUI |
| state | working/blocked/idle/done/dead/unknown/pane、source 表示 |
| protocol | `@agent_*` tmux pane options、TTL、owner PID freshness |
| integration | LazyAgent ACP の thinking/waiting/idle report |
| refresh | tmux control mode。preview 20Hz 上限、list 4Hz 上限、topology は即時、idle polling なし |

Rust 版の最初の milestone はこの機能を完全に置換し、既存 tmux keybinding と LazyAgent integration を変更せず動かすことである。その後 Herdr parity を積み上げる。

## 15. ライセンス上の注意

Herdr の repository は [AGPL-3.0-or-later と commercial の dual license](https://github.com/ogulcancelik/herdr#license) である。機能・挙動の調査結果から独自実装することと、Herdr の Rust source、manifest、hook script をコピー/改変することは分ける必要がある。

- 非 AGPL で公開する可能性があるなら、Herdr source をコピーせず public behavior/spec を基準に独自実装する
- Herdr code を再利用するなら、AGPL の network/source disclosure 要件を含め license 方針を先に確定する
- agent ごとの hook は各 agent の公開 hook API から新規作成する
- UI 名称、logo、文章、音源等の asset はコピーしない

実装開始前に agentmux 自体の license を決める。

## 16. 調査から導いた必須品質

- tmux が静かなとき idle polling しない
- first paint 150ms 以下を目標とし、通常操作 p95 50ms 以下
- pane output burst は preview 最大20Hz、重い detection/snapshot は最大4Hz
- 1 refresh につき tmux topology snapshot は1回。screen capture は候補 pane のみ
- shell string interpolation を避け、argv で tmux/git/agent command を起動
- destructive close/remove/force は target を再解決し確認する
- status report は source、sequence、generation、TTL、owner PID を検証する
- ambiguous target は error
- test は専用 `tmux -L agentmux-test-*` server で行い、利用者の server を触らない

## 17. Source code の照合箇所

公式文書だけでは曖昧な挙動は、調査 commit の次の実装と照合した。

- [Sidebar renderer](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/ui/sidebar.rs)
- [Mobile UI](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/ui/mobile.rs)
- [Navigator](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/ui/navigator.rs)
- [Context menu model](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/app/state.rs)
- [Agent process detection](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/detect/mod.rs)
- [Manifest engine](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/detect/manifest.rs)
- [Pane seen/state](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/pane/state.rs)
- [Workspace rollup](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/workspace/aggregate.rs)
- [API schema](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/api/schema.rs)
- [Persistence snapshot](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/persist/snapshot.rs)
- [Plugin manifest/runner](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/plugin_command.rs)
- [Configuration model/defaults](https://github.com/ogulcancelik/herdr/blob/c8850a2e3e284033142db0f8117fa5998c68bd59/src/config/model.rs)
