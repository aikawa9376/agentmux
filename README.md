# agentmux

tmux 上で動く複数の coding agent を、Herdr のサイドバーに近い操作感で一覧・監視・操作する Rust 製ツールです。tmux の session / window / pane と永続 PTY はそのまま利用します。

現在は最初の usable MVP です。全 tmux session の agent、状態、場所を横断表示し、選択 pane の live preview と主要操作を提供します。UIはHerdrと同じCatppuccin Mocha基調の状態色・spinner・フラットな2段sidebarを採用しています。

## Implemented

- `spaces` と `agents` の2段サイドバー
- blocked → done → working → idle → unknown の優先表示
- session/window/pane を横断した一覧と adaptive preview
- ANSI 16/256/truecolorと文字装飾を保った、末尾自動追従live preview
- blocked=赤、working=黄spinner、done=teal、idle=緑のHerdr式状態アイコン
- foreground command、TTY process、argv0、Node/Python package path、pane options による agent 識別
- 識別済みagentの画面下端による状態推定（画面本文はagent識別に不使用）
- `@agent_*` pane option と既存 LazyAgent integration の互換
- focus、spawn、send、respawn、kill、mark/unmark
- private PTY 上の tmux control-mode によるイベント更新
- mouse click/wheel、keyboard navigation、狭いpopup向け単一カラム表示
- human-readable output と `list --json`
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

設定を反映したら `tmux source-file ~/.config/tmux/tmux.conf` を実行してください。

tmuxの`source-file`は差分適用です。設定から削除した古いbindingは自動では消えないため、[contrib/agentmux.tmux](contrib/agentmux.tmux) は旧agentmux bindingを明示的に`unbind`してから再定義します。

## Sidebar keys

| Key | Action |
| --- | --- |
| `Tab` | spaces / agents panel の切替 |
| `j`, `k`, arrows | 選択移動 |
| `Enter` | 選択した session/pane を focus |
| `a` | agent-only / all panes の切替 |
| `r`, `Ctrl-g` | 即時 refresh |
| `Alt-c` / `Alt-l` / `Alt-o` | Codex / Claude / OpenCode を split 起動 |
| `Alt-n` | custom command を split 起動 |
| `Ctrl-s` | prompt を送信 |
| `Ctrl-r` | pane を確認付き respawn |
| `Ctrl-k` | pane を確認付き kill |
| `Ctrl-m` / `Ctrl-u` | agent metadata を設定 / 解除 |
| `q`, `Esc` | sidebar を閉じる |

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
agentmux mark %25 --kind codex --name reviewer
agentmux status %25 blocked "approval required"
agentmux status %25 working "prompt sent" 15
agentmux clear-status %25
agentmux unmark %25
agentmux respawn %25
```

Target には `%pane_id`、`session:window.pane`、一意な agent name/kind を指定できます。曖昧な名前はエラーになります。

誤検出を調べる場合は `agentmux explain <target>` を使います。agentの識別根拠と状態判定の根拠を別々に表示します。nvimなどeditor内のintegrationは `@agent_kind` と `@agent_status` をpublishすることで検出され、通常のbuffer本文にagent名が書かれているだけでは検出されません。

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
```

UI全体の幅が`preview_min_width`未満ならpreviewを描画せず、spaces/agents sidebarだけを全幅表示します。色付きpreviewはtmuxの`capture-pane -e`からforeground/background color、太字、italic、underlineなどを復元します。agentmux自身は背景色を指定せず、terminalの背景を継承します。

## Current boundary

このバージョンは「tmuxで使えるHerdr風サイドバー」の基礎です。永続daemon、`done = idle + unseen`、agent別manifest、通知、worktree、plugin、remote thin clientは次のphaseです。tmux自身が提供するPTY、session永続化、layout、copy-modeは再実装しません。

設計・調査文書:

- [Herdr 機能調査](docs/research/herdr-v0.7.3.md)
- [機能 parity matrix](docs/parity-matrix.md)
- [実装計画](docs/implementation-plan.md)

ライセンスは未決定です。HerdrのAGPL source/assetsは流用せず、公開仕様とtmux APIから独自実装しています。
