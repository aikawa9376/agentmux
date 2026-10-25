# Shared observer, agent input, and popup UI

## Scope and ownership

The selected scope is detection refresh, shared monitoring, completion/seen state, notifications, agent input/waits, and an optional floating UI. Git/worktree management and SSH-machine aggregation are excluded. Existing Git and SSH/tmux workflows remain the integration points.

Tmux owns terminals, PTYs, processes, layout, and attach/detach. `src/observer.rs` owns semantic state for one live tmux server. `src/tmux.rs::Tmux::raw_snapshot` is the discovery boundary; configured CLI, TUI, and LAN consumers read cached snapshots through the observer. Selected previews still use live `pane_view` capture or the publisher's ACP transcript.

## Discovery and generation

Bundled detection rules are pinned to Herdr v0.9.5, with 22 manifests including Letta. `NOTICE` records provenance and the local Copilot folder-trust and Codex legacy timer-separator extensions. OSC progress remains unavailable as input; screen/title rules provide fallback.

`Tmux::raw_snapshot` uses tmux metadata and process discovery for identity before interpreting screen text as state. Where terminal foreground groups are available, background process groups are excluded. The instance fingerprint contains the tmux pane PID and foreground group; an owned editor integration instead uses its owner PID, owner label, and transcript path. Starting helper subprocesses in the same foreground group does not create a new generation.

The observer assigns a monotonic generation when an instance fingerprint or kind changes. This separates an agent from a later occupant of the same pane. It is a best-effort process identity: PID/group reuse and native conversation changes inside an unchanged process are not exact session identity. Stable LazyAgent UUID messaging remains a separate interface.

Local rules replace a known bundled kind from `$XDG_CONFIG_HOME/agentmux/agent-detection/<kind>.toml`, falling back to `~/.config`. The loader validates kind ID, schema, engine version, regions, and regexes before adopting an override. Invalid overrides fall back to bundled rules and surface a warning in `explain`. The cache is process-local; restart the observer to reload rules. `explain` evaluates against the running observer's cache and includes source/version, matching rules, region sizes, and visible/skip flags without returning screen excerpts.

## Service lifecycle and storage

Normal CLI commands call `observer::ensure_running`. `agentmux daemon start|status|stop` exposes management; `daemon run` is the foreground implementation. Startup detaches from the launching shell, serializes concurrent starters with `flock`, and binds one Unix socket. A live tmux socket path plus server PID scopes the endpoint, preventing pane-ID collisions across tmux servers. Neither stopping nor restarting the observer stops tmux processes.

Artifacts are under the current user's private `agentmux-<uid>` directory under `/tmp`. The directory must have owner-only permissions and the expected owner; symlinks are rejected. Socket, lock, log, and ledger use private permissions. The lock owner may replace a stale socket; an unresponsive server is not killed automatically.

The ledger saves state, fingerprints, generation/sequence counters, completion, and server seen state through file sync and atomic rename. It stores no screen contents. A corrupt ledger is preserved and reported instead of silently overwritten. Client seen positions and event history are in memory. Temporary-directory cleanup or an entire tmux-server replacement starts a new scope; this ledger is not tmux layout restore or native agent conversation resume.

One control-mode watcher supplies invalidation hints. Shared discovery is limited by `refresh.status_ms` during work/output and reconciles idle state at `max(status_ms, 2000)` milliseconds. Explicit target resolution and pane-option mutations force a refresh so newly created panes can be operated immediately. Independent UI instances do not create their own control-mode watchers when connected to the service. Bounded hint queues may coalesce notifications; periodic scans recover current state.

Three consecutive background scan failures stop the observer. Subsequent CLI startup can create it again; already-running UI/LAN consumers report observer connection errors rather than silently choosing a different state source.

## Completion and seen state

The state machine retains observed work across snapshots. Initial idle or a stale initial native done report does not generate completion. An agent first observed working can complete its first task. Native state is authoritative; screen-only working-to-idle needs at least 300 ms of confirmation to avoid a disappearing spinner producing an immediate completion. `skip_state_update` holds the last state.

An observed return to idle after work assigns `completion_seq` and displays Done while that completion is unseen. Fresh work, process replacement, dead/plain states, and blocked/unknown states are not themselves successful completion. This tracks agent lifecycle, not shell exit status or background job completion. The agent list sorts equal-priority entries by descending state sequence.

Passive list/preview reads never mark completion seen. Explicit focus updates the server/CLI seen state and the caller's client seen state. Actual tmux foreground viewing updates that client's position independently. Popup clients register a PID-owned overlay so the pane hidden beneath the popup is not automatically considered viewed. Explicit focus from the popup closes it and marks the selected agent seen. Other terminal overlays are not detected.

TUI and CLI Done badges can therefore differ. Client positions are not persisted across observer restart; server seen/completion are. Native conversation switches are only separated when the instance fingerprint changes; session-only identity integration is not implemented.

## Notifications

`notifications.tmux = true` delivers Done/Blocked through tmux client messages by default. A transition queues one notification, with active-pane suppression per attached non-control client. There is no startup Done notification. A bounded worker isolates notification work from observation. `notifications.system = true` optionally invokes Linux `notify-send`; absence/failure is nonfatal. Audio and Android push delivery are not implemented.

Notification queues are best effort. Durable state and event sequences are the source of truth; notification delivery is not an acknowledgment or authorization.

## Agent input and waits

CLI send, TUI prompt submission, and LAN actions use `control` through the observer. The shared action path serializes submissions, validates the current identity binding, and rejects unavailable/exited/plain targets and blocked prompt submission. Terminal input uses a named tmux buffer, bracketed paste where supported, and Enter. ACP input goes to the exact published editor/transcript through the existing RPC bridge. Raw tmux keys remain the way to deliberately answer a blocked dialog.

The binding includes the instance fingerprint. Validation is not a transaction over process exit: the occupant can still change between checking and writing. No failed/uncertain submission is retried automatically.

`send --wait` returns a receipt containing observer epoch, generation, and pre-submission sequence. Waiting requires activity after that sequence (or an agent already working), then an accepted state. A completion sequence also proves a short working-to-idle transition that happened between polling reads. Pre-existing idle/Done cannot complete a new submission. If no activity is observed within five seconds, waiting fails with `agent_prompt_stalled`; a shorter caller deadline produces timeout. The deadline includes submission work after CLI argument parsing. Reads after observer restart or generation replacement interrupt a wait.

This does not identify individual turns: if an agent is already working, that active turn's completion can satisfy the wait. `unknown` does not prove completion. Standalone `wait` matches current state immediately. Default accepted states are idle/Done/blocked, repeated `--until` selects exact alternatives, and all timeout arguments are milliseconds.

## IPC and event synchronization

Version 1 uses one NDJSON request/response per Unix connection. Responses have `version`, `result`, and optional `error`. Request size, worker count, accepted queue, write/read time, and the event ring are bounded. A slow/malformed reader does not own the observer state lock.

| Method | Input | Result |
| --- | --- | --- |
| `status` | — | protocol version, PID, epoch, sequence, socket path |
| `snapshot` | optional `client` | existing spaces/panes schema plus generation, state/completion seq and observer cursor |
| `refresh` | — | null; serialize an immediate discovery pass |
| `seen` | `pane`, optional `client` | null; explicit server/client acknowledgment |
| `action` | existing control `request` with pane/binding/action/text | submission receipt |
| `events` | `epoch`, `since` | events after the supplied sequence |
| `explain` | `pane` | active manifest evidence or null |
| `overlay` | `client`, positive owner `pid`, `active` | register/withdraw popup visibility |
| `stop` | — | null; stop the observer after responding |

Example request: `{"method":"snapshot"}`. Event kinds are `agent.started`, `agent.state`, `agent.seen`, and `agent.removed`. State events describe the underlying lifecycle; idle plus an unseen completion becomes Done in snapshot views.

Take a snapshot, retain `observer.epoch` and `observer.seq`, then request events after that sequence. Mutations between these requests are retained in the ring. The ring holds 256 events: older cursors return `events_lost`; epoch changes return `observer_restarted`. Both require a new snapshot, never silent continuation. This is a state-event interface, not a lossless PTY-byte or arbitrary tmux-topology stream.

`agentmux events` prints an initial snapshot then polls this cursor interface as NDJSON. Resume explicitly with `agentmux events --epoch <epoch> --since <seq>`; `--once` performs one read. The per-connection protocol is intentionally independent of CLI stream lifetime.

## Popup presentation

`agentmux ui --popup [--width 80%] [--height 80%]` launches `tmux display-popup -E` with the current executable. Dimensions accept positive cells or percentages up to 100%. Socket selection, cwd, origin, client, and relevant config environment are forwarded; shell arguments are quoted. A hidden `--popup-child` flag prevents recursive launch.

The child has a separate PTY. Its inherited `TMUX_PANE` belongs to the underlying origin, so it must not exclude that origin as if it were the UI pane. Enter focuses and closes; q/Esc closes. Normal pane/window UI still survives focus. The origin layout and pane count do not change.

`caller_client` ignores observer control clients and infers a single human client or a unique client viewing the origin. Ambiguous callers supply `--client`; contributed bindings always provide client and origin. `prefix A` adds popup access while existing `M-a` and `prefix a` retain their presentation.

## Verification boundaries

`observer` unit tests cover completion, passive reads, independent seen state, restart ledger, replacement, skipped updates, transient idle, and explicit event loss/restart. Detection fixtures cover the updated UI rules and invalid override validation. Isolated tmux tests cover metadata, foreground/background detection, and control-mode behavior.

`tests/observer_smoke.py` covers concurrent startup, private IPC, persisted completion, epoch reset, blocked rejection, activity-correlated waits, and occupant replacement. `tests/popup_smoke.py` uses a real attached client in a private PTY for popup rendering, origin visibility, Enter/q, geometry, notifications and suppression. Existing UI and LAN/ACP smoke tests cover their previous contracts. Real agent versions, macOS behavior, system notification delivery, and Android devices require separate validation.
