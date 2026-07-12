# Herdr → agentmux parity matrix

比較基準: Herdr v0.7.3 + master `c8850a2`、2026-07-10

実装状況更新: 2026-07-11。`implemented` はRust MVPで実装・test済み、`partial` は同じ利用目的の一部まで実装済みを表す。

この表は「Herdr の source を同じ構造で作り直す」表ではない。利用者から見える capability を、tmux native、agentmux 実装、または tmux 向けの適応のどれで満たすかを追跡する。

状態:

- `native`: tmux がすでに提供し、agentmux は薄く呼び出す
- `existing`: Bash 版に存在し、Rust milestone 1 で互換移植する
- `planned`: Rust 版で新規実装する
- `adapted`: Herdr と同じ内部方式にはせず、tmux 上で同等の結果を提供する
- `excluded`: tmux 前提と矛盾し、明示的に対象外

Phase は [実装計画](implementation-plan.md) の P0〜P9 を指す。

## Runtime と hierarchy

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| R01 | background server namespace | tmux server/socket | native | P1 | default と任意 `-L/-S` socket を選べる |
| R02 | persistent PTY/process | tmux server 所有の pane | native | P1 | drawer/daemon 終了後も process が生きる |
| R03 | detach/reattach | tmux detach/attach | native | P1 | agentmux を介しても tmux 標準挙動を変えない |
| R04 | multi-client attach | tmux client | native/adapted | P3 | focus は drawer を呼んだ client のみ変える |
| R05 | session/workspace/tab/pane hierarchy | server/session/window/pane | adapted | P1 | 全 socket/session/window/pane を stable ID で列挙 |
| R06 | direct terminal/agent attach | `tmux attach/select-*` wrapper | adapted | P4 | agent name または pane ID から attach/focus |
| R07 | normal SSH remote use | SSH 上の tmux + agentmux | native | P1 | remote shell で同じ binary/UI が動く |
| R08 | `--remote` thin client | SSH bridge と remote agentmux | planned | P7 | version check、keepalive、remote socket 選択 |
| R09 | snapshot restore | tmux topology + agentmux metadata snapshot | planned | P6 | server restart 後に session/window/layout/cwd/label を復元 |
| R10 | pane screen history restore | tmux history capture/restore | adapted | P6 | opt-in、secret 警告、上限付き保存 |
| R11 | native agent session resume | hook session ref + resume template | planned | P5 | invalid/duplicate/stale ref は shell fallback |
| R12 | server live handoff | tmux process は触らず daemon のみ restart | adapted | P8 | upgrade 中も tmux pane process は停止しない |
| R13 | native Windows beta | tmux 非対応 | excluded | — | WSL は対象、native Windows は非対応と明記 |

## Sidebar、navigator、入力

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| U01 | command で dashboard を開く | tmux popup | implemented | P1 | `M-a` / `prefix a` から起動 |
| U02 | expanded two-section sidebar | session tree + all-agent list | partial | P3 | 上下 section、各 scroll、active highlight |
| U03 | compact/hidden collapse | TUI mode と保存設定 | planned | P3 | expanded/compact/hidden を toggle |
| U04 | width/min/max resize | left popup + internal divider | planned | P3 | drag/keyboard resize、再起動後も保持 |
| U05 | section split resize | workspace/agent divider | planned | P3 | drag 比率を保存し小さい高さでも破綻しない |
| U06 | mobile narrow layout | agent-first single column | partial | P3 | configurable threshold、40列程度でも操作可能 |
| U07 | real pane view | selected pane live capture preview | adapted/partial | P1/P3 | ANSI preview、burst は20Hz以下、focusで実 paneへ |
| U08 | workspace/session row | state/name/branch/ahead/behind | partial | P3/P6 | Git情報を非同期取得しUIを止めない |
| U09 | worktree tree | parent/child session tree | planned | P6 | expand/collapse、focus、group close |
| U10 | all-agent panel | 全 tmux session の agent | partial | P2/P3 | workspace順とpriority順を切替 |
| U11 | mouse focus/scroll | Crossterm mouse events | partial | P3 | click、wheel、scrollbar、outside click |
| U12 | drag reorder | persisted session order + tmux window move | planned | P3 | drag中表示と確定後の正しい順序 |
| U13 | right-click menus | Ratatui context menu | planned | P3 | session/window/pane/worktree action を表示 |
| U14 | searchable navigator | session/window/pane/agent tree | planned | P3 | fuzzy search、state filter、mouse/keyboard |
| U15 | prefix/direct keybindings | tmux binding + TUI keymap | partial | P1/P3 | conflict検出、複数binding、help表示 |
| U16 | copy mode/search | tmux copy-modeを利用 | native/adapted | P3 | drawer preview copy と実 pane copy を区別 |
| U17 | settings UI | TOML editor surface | planned | P8 | safe setting は hot reload |
| U18 | onboarding/release notes | TUI modal | planned | P8 | first run と upgrade 後に一度だけ表示 |
| U19 | themes/light-dark/custom colors | Ratatui palette | planned | P3/P8 | builtin、terminal palette、custom color |
| U20 | CJK width/IME/truecolor | TUI側を検証、paneはtmux委譲 | adapted | P3/P9 | 日本語label/filterが崩れない |

## Agent detection と state

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| A01 | foreground agent detection | tmux format + TTY process tree | implemented | P1/P2 | wrapper/runtime越しも検出 |
| A02 | supported agent catalog | 20 agent の process matcher | partial | P2 | 公式一覧を fixture で網羅 |
| A03 | manual agent mark | tmux pane options | implemented | P1 | mark/unmark が現行 option と互換 |
| A04 | VM/sandbox hint | `@agent_kind` / env hint | partial | P2 | hidden processでも指定manifestを使える |
| A05 | screen manifest detection | TOML rule engine | planned | P2 | region/gate/matcher、safe fallback |
| A06 | OSC/title evidence | tmux format/capture 可能範囲 | planned/adapted | P2 | available evidence を explain に明示 |
| A07 | local manifest override | XDG config | planned | P2 | invalid file は警告しbundledへfallback |
| A08 | remote manifest update | versioned HTTPS catalog | planned | P8 | validation/checksum、disable、manual update |
| A09 | `agent explain` | evidence report | partial | P2 | `explain [--json]`でidentity/state sourceとmatched manifest ruleを表示。全rule evidenceは今後追加 |
| A10 | authority arbitration | lifecycle > screen > process | partial | P2 | state source が競合せずgenerationで切替 |
| A11 | semantic states | blocked/working/done/idle/unknown | partial | P2 | typed enum、未知値拒否、JSON安定化 |
| A12 | unseen done | transition + per-client seen | planned | P2/P3 | background完了はdone、閲覧後idle |
| A13 | rollup | pane→window→session | partial | P2 | priority rule をunit test |
| A14 | custom agent name | pane metadata/options | implemented | P1 | rename/clearと一意target解決 |
| A15 | custom display status | semanticとdisplayを分離 | planned | P2 | wait/rollupはcustom labelに影響されない |
| A16 | display metadata + TTL/seq | metadata sources | partial | P2/P4 | sanitize、TTL、seq、authority guard |
| A17 | owner PID/generation freshness | tmux options + daemon state | partial | P1/P2 | owner終了後にstale reportを使わない |
| A18 | state-change timestamps | daemon monotonic sequence | planned | P2 | priority sortと通知delayに利用 |

## Agent と tmux object の操作

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| C01 | agent list/get | daemon index | partial | P2/P4 | human/JSON、曖昧targetはerror |
| C02 | agent read | `capture-pane` source variants | partial | P4 | visible/recent/unwrapped/detection/ANSI |
| C03 | agent send/run | `send-keys -l` + Enter | implemented | P1/P4 | textをtmux commandとして解釈しない |
| C04 | agent focus | `switch-client/select-window/select-pane` | partial | P1 | caller client を正しく選択 |
| C05 | agent start | split/new-window/new-session | partial | P1/P4 | cwd/env/name/focus/split指定 |
| C06 | agent wait | daemon event wait | planned | P4 | timeout、disconnect、state raceなし |
| C07 | agent attach/takeover | tmux client/pane focus | adapted | P4 | ownershipの意味を文書化 |
| C08 | session CRUD | tmux session commands | partial | P4 | list/create/focus/rename/close JSON |
| C09 | tab/window CRUD | tmux window commands | partial | P4 | list/create/focus/rename/close JSON |
| C10 | pane get/process/cwd | tmux formats + process resolver | partial | P4 | cwdとforeground_cwdを分離 |
| C11 | pane split + ratio | `split-window -h/-v -l/-p` | partial | P4 | direction/ratio/cwd/env/focus |
| C12 | neighbor/edges/focus | layout geometry + `select-pane` | planned | P4 | 4方向、edge判定 |
| C13 | resize/swap/zoom | tmux resize/swap/resize-pane -Z | partial | P4 | no-op理由を返す |
| C14 | live pane move | `break-pane/join-pane/move-pane` | planned | P4 | processとpane IDを追跡し続ける |
| C15 | layout export/apply | tmux layout + declarative tree | planned | P6 | 構造/cwd/commandを再現、live PTYとは区別 |
| C16 | labels/borders/tab state | tmux names/options/formats | partial | P3/P4 | manual label優先、agent label fallback |
| C17 | kill/respawn | tmux kill/respawn-pane | implemented | P1 | destructive確認、saved command/cwd |

## Worktree と persistence

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| W01 | worktree list | `git worktree list --porcelain` | planned | P6 | bare repoもparse |
| W02 | create/open | argv Git command + tmux session | planned | P6 | existing branch/new branch/base/path |
| W03 | grouping | session option + persisted origin | planned | P6 | restart後もparent/child維持 |
| W04 | safe remove | `git worktree remove` | planned | P6 | dirtyは拒否、force再確認、branch非削除 |
| W05 | Git status/ahead/behind | async cache | planned | P6 | TUIをblockせず期限切れ更新 |
| P01 | UI/state snapshot | atomic JSON/SQLite | planned | P2/P6 | crash時に破損せずschema migration可能 |
| P02 | autosave debounce | daemon writer | planned | P2 | burst中に過剰writeしない |
| P03 | agent session reference | integration-only secret metadata | planned | P5 | 通常listから不要なsecretを漏らさない |
| P04 | restore ordering | topology→client context→agent resume | planned | P6 | duplicate session refを起動しない |

## CLI、API、events

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| X01 | single binary CLI/TUI/daemon | subcommands/mode dispatch | partial | P1/P2 | shell依存なしで主要操作が動く |
| X02 | machine JSON output | serde response envelope | partial | P4 | version/type/error codeを固定 |
| X03 | local socket API | Unix socket/NDJSON | planned | P2/P4 | user-only permission、request ID |
| X04 | initial session snapshot | topology + state response | planned | P4 | snapshot後subscriptionでraceを回避 |
| X05 | lifecycle events | daemon event hub | planned | P4 | session/window/pane/agent/worktree/layout |
| X06 | long-lived subscriptions | bounded fanout | planned | P4 | slow clientが他clientを止めない |
| X07 | wait output | skill helperの capture polling、将来はoutput event | partial | P4 | literal/regex、future output、timeoutは実装済み。event race解消 |
| X08 | wait agent status | skill helperのstate polling、将来はstate event | partial | P4 | pane ID/一意nameのwaitは実装済み。subscribe前後のrace解消 |
| X09 | pane observe/control stream | control-mode bridge | planned | P7 | read-onlyとwriter authorityを分離 |
| X10 | API schema | schemars JSON Schema | planned | P4 | binaryからschema出力、CI diff |
| X11 | status diagnostics | daemon/tmux/socket/protocol | planned | P2/P4 | human/JSON の両方 |
| X12 | shell completion | clap_complete | planned | P8 | bash/zsh/fish/PowerShell/elvish |
| X13 | agent skill | tmux-native orchestration + agentmux identity/state/send | implemented | P4 | tmux内判定、self-pane保護、no-focus split、曖昧target拒否 |

## Integrations、notifications、configuration

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| I01 | custom status protocol | `@agent_*` + socket report | partial | P1/P2 | LazyAgent Luaを変更せず動かす |
| I02 | install/uninstall/status | per-agent config editor | planned | P5 | idempotent、他設定を保持、backup |
| I03 | lifecycle integrations | Pi/OMP/Kimi/OpenCode/Kilo/Hermes/Mastra | planned | P5 | fixture eventで状態遷移検証 |
| I04 | session-only integrations | Claude/Codex/Copilot/Devin/Droid/Qoder/Cursor | planned | P5 | state authorityを奪わずsession refだけ保存 |
| I05 | outdated integration check | embedded integration version | planned | P5/P8 | settings/CLIに更新候補を表示 |
| I06 | LazyAgent ACP lifecycle | `publish` / owner-safe `withdraw` bridge | implemented/native verified | P2 | buffer ACPのworking/blocked/idle、複数session集約、終了時解除 |
| N01 | done/blocked sound | local player | planned | P7 | delay後も同stateの場合だけ再生 |
| N02 | in-drawer toast | TUI overlay | planned | P7 | click/keyでtargetへjump |
| N03 | terminal notification | OSC escape via caller client | planned | P7 | SSH経由でouter terminalに届く |
| N04 | system notification | notify-send/terminal-notifier等 | planned | P7 | unavailable時は非fatal |
| N05 | per-agent sound/custom mp3 | config | planned | P7 | done/request/global override |
| N06 | active target suppression | per-client focus | planned | P7 | active paneでは重複通知しない |
| F01 | TOML config/defaults | XDG config | implemented | P1 | default-config、validation、unknown警告 |
| F02 | hot reload | daemon config swap | planned | P2/P8 | 起動時限定項目を明示 |
| F03 | custom commands | pane/background/plugin action | planned | P8 | context env、argv安全性 |
| F04 | logs/rotation | tracing_appender | planned | P2 | daemon/client分離、容量上限 |
| F05 | update channel/check | self-update | planned | P8 | package manager installを破壊しない |

## Plugins と distribution

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| G01 | plugin manifest v1 | TOML manifest/argv | planned | P8 | version/platform/command validation |
| G02 | local link/enable/action/log | registry + process runner | planned | P8 | restart後もregistry保持 |
| G03 | GitHub install/uninstall | managed checkout | planned | P8 | source/ref/commit previewとconfirm |
| G04 | event hooks | event hub subscriber | planned | P8 | bounded queue、loop/rate guard |
| G05 | managed pane placement | popup/split/window/zoom | planned | P8 | ownershipをmove後も追跡 |
| G06 | link handler | captured URL→action | planned | P8 | regex validation、明示modifier |
| D01 | Linux/macOS binary | release artifacts | planned | P8 | checksum、install script、Homebrew候補 |
| D02 | single Rust binary | staticに近い配布 | partial | P1/P8 | fzf/curl/scriptを必須にしない |
| D03 | quality/parity audit | automated matrix | planned | P9 | 全行にtest/evidenceまたはdocumented exclusion |

## 完了の判定

「Herdr parity 完了」は単に command 名が存在することではない。各行が次のどれかになった時点とする。

1. 自動 test と利用者向け文書を伴う `implemented`
2. tmux native の挙動を integration test で確認した `native verified`
3. 差分と理由を文書化し、同じ利用目的を満たした `adapted verified`
4. tmux 前提と両立しないため明示合意した `excluded`

P1 完了時点で既存 Bash 版を置換可能、P4 完了時点で日常の agent orchestration が可能、P9 完了時点で上表全体の監査が完了する。
