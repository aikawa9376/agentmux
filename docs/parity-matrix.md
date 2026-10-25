# Herdr → agentmux parity matrix

比較基準: 当初Herdr v0.7.3 + master `c8850a2`。2026-09-08にv0.9.0時点のmaster `9e01168`を追加調査。2026-10-25に最新安定版v0.9.5 / `2ecf19a`と照合（[差分・根拠・推奨順序](research/herdr-v0.9.5.md)）。検出manifestもv0.9.5へ更新済み。

実装状況更新: 2026-10-25（現行ソースを確認し、通常pane UI、socket指定範囲、LAN機能、state/APIの不足を補正。通常pane/ACPのpreview履歴スクロールを追加。全行のruntime parityを再検証したものではない）。`implemented` はRust MVPで実装・test済み、`partial` は同じ利用目的の一部まで実装済みを表す。

この表は「Herdr の source を同じ構造で作り直す」表ではない。利用者から見える capability を、tmux native、agentmux 実装、または tmux 向けの適応のどれで満たすかを追跡する。

状態:

- `native`: tmux がすでに提供し、agentmux は薄く呼び出す
- `existing`: Bash 版に存在し、Rust milestone 1 で互換移植する
- `planned`: Rust 版で新規実装する
- `adapted`: Herdr と同じ内部方式にはせず、tmux 上で同等の結果を提供する
- `excluded`: tmuxの責務境界またはユーザー方針により対象外

現在の採用範囲は検出・共有監視・未読完了・通知・agent操作とpopup表示。Git/worktree管理とSSHマシン集約はユーザーの方針で対象外とする。[実装済みの設計と制約](design/observer-and-popup.md)を参照。Phaseは旧[実装計画](implementation-plan.md)の識別子であり、残るplanned項目をすべて実装する合意ではない。

## Runtime と hierarchy

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| R01 | background server namespace | tmux server/socket | native/partial | P1 | defaultと`--socket-name` (`-L`) は対応。`-S` path指定は未実装 |
| R02 | persistent PTY/process | tmux server 所有の pane | native | P1 | drawer/daemon 終了後も process が生きる |
| R03 | detach/reattach | tmux detach/attach | native | P1 | agentmux を介しても tmux 標準挙動を変えない |
| R04 | multi-client attach | tmux client | native/partial | P3 | `switch-client -c`とclient別既読あり。同一sessionでのwindow/pane focus独立性は追加検証が必要 |
| R05 | session/workspace/tab/pane hierarchy | server/session/window/pane | adapted/partial | P1 | 選択した1 socket内をID付き列挙。複数socket集約は未実装 |
| R06 | direct terminal/agent attach | `tmux attach/select-*` wrapper | adapted | P4 | agent name または pane ID から attach/focus |
| R07 | normal SSH remote use | SSH 上の tmux + agentmux | native | P1 | remote shell で同じ binary/UI が動く |
| R08 | custom SSH thin client | 既存SSH/tmuxを利用 | excluded | — | 独自SSH bridgeは作らない |
| R09 | snapshot restore | tmux topology + agentmux metadata snapshot | planned | P6 | server restart 後に session/window/layout/cwd/label を復元 |
| R10 | pane screen history restore | tmux history capture/restore | adapted | P6 | opt-in、secret 警告、上限付き保存 |
| R11 | native agent session resume | hook session ref + resume template | planned | P5 | invalid/duplicate/stale ref は shell fallback |
| R12 | server live handoff | tmux process は触らず daemon のみ restart | adapted | P8 | upgrade 中も tmux pane process は停止しない |
| R13 | native Windows beta | tmux 非対応 | excluded | — | WSL は対象、native Windows は非対応と明記 |
| R14 | saved SSH machines / combined agents | host別の既存tmux workflow | excluded | — | host横断の一覧集約・接続管理・CLI転送は作らない |
| R15 | LAN/mobile thin client | token認証HTTP + web/Android | implemented/adapted | P7 | QR、agent一覧、可視画面、binding付き送信・中断を実装。SSH集約とは別機能。実機検証は残る |

## Sidebar、navigator、入力

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| U01 | commandでdashboardを開く | 通常pane/window + optional popup | implemented | P1 | `M-a` / `prefix a` / `prefix A`、`ui --popup`。popupはEnterでfocusして終了、geometry維持を検証 |
| U02 | expanded two-section sidebar | session tree + all-agent list | partial | P3 | 上下 section、各 scroll、active highlight |
| U03 | compact/hidden collapse | TUI mode と保存設定 | planned | P3 | expanded/compact/hidden を toggle |
| U04 | width/min/max resize | tmux pane resize + TUI設定 | native/partial | P3 | 外側はtmuxでresize可能。agentmuxのmin/max・保存は未実装 |
| U05 | section split resize | workspace/agent divider | planned | P3 | drag 比率を保存し小さい高さでも破綻しない |
| U06 | mobile narrow layout | agent-first single column | partial | P3 | configurable threshold、40列程度でも操作可能 |
| U07 | real pane view | live capture + tmux/ACP history preview | adapted/partial | P1/P3 | ANSI保持、PageUp/Down、Home/End、履歴閲覧中は固定、末尾で追従復帰。全画面アプリ内部の履歴取得は未実装 |
| U08 | workspace/session row | state/name/count | partial | P3 | 現行session一覧を利用。Git branch/ahead/behindは対象外 |
| U09 | worktree tree | Git管理を行わない | excluded | — | tmux sessionをそのまま利用 |
| U10 | all-agent panel | 全 tmux session の agent | partial | P2/P3 | workspace順とpriority順を切替 |
| U11 | mouse focus/scroll | Crossterm mouse events | partial | P3 | click、カーソル下のpreview/一覧をwheel操作。scrollbar drag・outside click dismissは未実装 |
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
| A02 | supported agent catalog | 22 manifest + process matcher | partial | P2 | v0.9.5へ7種更新＋Letta追加。実agent最新版でのruntime検証は別途 |
| A03 | manual agent mark | tmux pane options | implemented | P1 | mark/unmark が現行 option と互換 |
| A04 | VM/sandbox hint | `@agent_kind` / env hint | partial | P2 | hidden processでも指定manifestを使える |
| A05 | screen manifest detection | TOML rule engine | implemented | P2 | 22 manifest、nested gates、regions、最新UI fixtures。observerはskip時に前回stateを保持 |
| A06 | OSC/title evidence | tmux pane_title / capture | partial/adapted | P2 | title対応、OSC progress入力は空。取得可能性とfallbackは追加検証 |
| A07 | local manifest override | XDG config | implemented | P2 | ID/schema/regex/region/engine検証。不正ならbundledへfallbackしexplainに警告。反映はdaemon再起動 |
| A08 | remote manifest update | versioned HTTPS catalog | planned | P8 | validation/checksum、disable、manual update |
| A09 | agent explain | identity + active manifest evidence | partial | P2 | observerのsource/version/全ruleの一致・region bytes・visible/skipを表示。nested gate個別の説明は未実装 |
| A10 | authority arbitration | lifecycle > screen > process | partial | P2 | state source が競合せずgenerationで切替 |
| A11 | semantic states | blocked/working/done/idle/unknown | partial | P2 | typed enum、未知値拒否、JSON安定化 |
| A12 | unseen done | completion seq + server/client seen | implemented/adapted | P2/P3 | work→idleを保持、readは既読にしない、focusは明示既読。startup誤通知・client間混同を検証。native会話IDは未取得 |
| A13 | rollup | pane→sessionを実装、windowは未実装 | partial | P2 | priority ruleをunit test。window集約を追加 |
| A14 | custom agent name | pane metadata/options | implemented | P1 | rename/clearと一意target解決 |
| A15 | custom display status | semanticとdisplayを分離 | planned | P2 | wait/rollupはcustom labelに影響されない |
| A16 | display metadata + TTL/seq | metadata sources | partial | P2/P4 | sanitize、TTL、seq、authority guard |
| A17 | owner PID/generation freshness | pane/process group/owner/transcript | partial | P1/P2 | owner終了・実行世代を区別。PID再利用と同process内の会話IDは厳密には追跡しない |
| A18 | state-change sequence | shared observer ledger | partial | P2 | state_seq/completion_seq/generationを保存。同priorityではstate_seq降順。wall clock timestampは未実装 |

## Agent と tmux object の操作

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| C01 | agent list/get | daemon index | partial | P2/P4 | human/JSON、曖昧targetはerror |
| C02 | agent read | `capture-pane` source variants | partial | P4 | visible/recent/unwrapped/detection/ANSI |
| C03 | agent send/run | observer経由の共通control path | implemented/adapted | P1/P4 | CLI/TUI/LANでbinding検証、terminal paste/ACP RPC、blocked拒否。process終了とのtransactionではない |
| C04 | agent focus | `switch-client/select-window/select-pane` | partial | P1 | caller client を正しく選択 |
| C05 | agent start | split/new-window/new-session | partial | P1/P4 | cwd/env/name/focus/split指定 |
| C06 | agent wait | observer snapshot + generation/seq | implemented/adapted | P4 | standalone waitとsend --wait、timeout、古いdone誤完了・世代変更・observer再起動を検証 |
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
| W01 | worktree list | `git worktree list --porcelain` | excluded | — | Git/worktree管理は対象外 |
| W02 | create/open | argv Git command + tmux session | excluded | — | Git/worktree管理は対象外 |
| W03 | grouping | session option + persisted origin | excluded | — | Git/worktree管理は対象外 |
| W04 | safe remove | `git worktree remove` | excluded | — | Git/worktree管理は対象外 |
| W05 | Git status/ahead/behind | async cache | excluded | — | Git/worktree管理は対象外 |
| P01 | state snapshot | private atomic JSON ledger | partial | P2 | generation/sequence/completion/server seenを保存。UI位置とtmux topologyは保存しない |
| P02 | autosave debounce | daemon writer | planned | P2 | burst中に過剰writeしない |
| P03 | agent session reference | integration-only secret metadata | planned | P5 | 通常listから不要なsecretを漏らさない |
| P04 | restore ordering | topology→client context→agent resume | planned | P6 | duplicate session refを起動しない |

## CLI、API、events

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| X01 | single binary CLI/TUI/daemon | subcommands/mode dispatch | implemented | P1/P2 | 通常CLIからsocket単位のobserverを自動起動 |
| X02 | machine JSON output | serde response envelope | partial | P4 | version/type/error codeを固定 |
| X03 | local socket API | private Unix socket/NDJSON v1 | partial | P2/P4 | version/result/error、bounded workers/requests。request ID echoは未実装 |
| X04 | initial state snapshot | snapshot cursor + event ring | implemented/adapted | P4 | epoch/seqを返し以後のeventを取得。欠落時は明示再同期。agent以外のtopology eventは未実装 |
| X05 | lifecycle events | agent state event ring | partial | P4 | agent.started/state/seen/removed。window/session/layout等のeventは未実装 |
| X06 | subscriptions | CLI NDJSON + cursor polling | partial/adapted | P4 | 256 event ringと欠落通知。IPC自体は1request/connection、push fanoutは未実装 |
| X07 | wait output | skill helperの capture polling、将来はoutput event | partial | P4 | literal/regex、future output、timeoutは実装済み。event race解消 |
| X08 | wait agent status | native CLI + helper polling | implemented/adapted | P4 | CLIは同じobserverのgeneration/seqを検証。helper単独は送信との相関を持たない |
| X09 | pane observe/control stream | control-mode bridge | planned | P7 | read-onlyとwriter authorityを分離 |
| X10 | API schema | schemars JSON Schema | planned | P4 | binaryからschema出力、CI diff |
| X11 | status diagnostics | daemon status + private log | partial | P2/P4 | version/PID/socket/epoch/seqをJSON出力。全面的doctorは未実装 |
| X12 | shell completion | clap_complete | planned | P8 | bash/zsh/fish/PowerShell/elvish |
| X13 | agent skill | tmux-native orchestration + agentmux identity/state/send | implemented | P4 | tmux内判定、self-pane保護、no-focus split、曖昧target拒否 |
| X14 | event loss / snapshot resync | epoch + sequence | implemented | P4 | events_lost / observer_restartedを明示。境界条件をunit/smokeで検証 |
| X15 | prompt followed by state wait | ordered send + activity/completion seq | implemented/adapted | P4 | 送信前のidle/doneで完了しない。活動なし5秒でstalled。個別turnの相関とは保証を分ける |

## Integrations、notifications、configuration

| ID | Herdr capability | tmux/agentmux での対応 | 現状 | Phase | 受入条件 |
| --- | --- | --- | --- | --- | --- |
| I01 | custom status protocol | `@agent_*` + socket report | partial | P1/P2 | LazyAgent Luaを変更せず動かす |
| I02 | install/uninstall/status | per-agent config editor | planned | P5 | idempotent、他設定を保持、backup |
| I03 | lifecycle integrations | agent別state authority | partial | P5 | Copilot独自hookとLazyAgent ACPあり。Pi/OMP等のinstallerは未実装。全hookを一律にstate authorityとしない |
| I04 | session-only integrations | Claude/Codex/Copilot等の会話参照 | planned | P5 | 本家v0.9.5のCodex等は画面state＋session-only hook。既存Copilot独自state hookとの役割を分ける |
| I05 | outdated integration check | embedded integration version | planned | P5/P8 | settings/CLIに更新候補を表示 |
| I06 | LazyAgent ACP lifecycle | `publish` / owner-safe `withdraw` bridge | implemented/native verified | P2 | buffer ACPのworking/blocked/idle、複数session集約、終了時解除 |
| I07 | self-reported resume command | owner/session ID/resume argv | planned | P5/P6 | state・表示metadataと分離。seq、終了時release、再起動後の会話再開を検証 |
| N01 | done/blocked sound | local player | planned | P7 | delay後も同stateの場合だけ再生 |
| N02 | in-drawer toast | TUI overlay | planned | P7 | click/keyでtargetへjump |
| N03 | terminal notification | OSC escape via caller client | planned | P7 | SSH経由でouter terminalに届く |
| N04 | system notification | optional Linux notify-send | partial | P7 | configで有効化、失敗非fatal、workerで分離。desktopでの実配信は未検証 |
| N05 | per-agent sound/custom mp3 | config | planned | P7 | done/request/global override |
| N06 | active target suppression | attached non-control client focus | implemented/adapted | P7 | tmux背景通知をactive paneで抑止。専用PTY smokeで検証 |
| N07 | tmux background message | tmux display-message | implemented | P7 | Done/BlockedをUI終了中も通知。tmux=trueが既定、active pane抑止 |
| F01 | TOML config/defaults | XDG config | implemented | P1 | default-config、validation、unknown警告 |
| F02 | hot reload | daemon config swap | planned | P2/P8 | 起動時限定項目を明示 |
| F03 | custom commands | pane/background/plugin action | planned | P8 | context env、argv安全性 |
| F04 | logs | private observer log | partial | P2 | background errorを記録。rotationは未実装 |
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
| G07 | plugin update | source/ref付きmanaged更新 | planned | P8 | 本家0.9.4で追加。稼働中pluginを壊さず更新しlocal linkを保持。互換hostの要否は未決定 |
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
