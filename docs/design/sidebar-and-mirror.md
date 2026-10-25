# Sidebar and mirror design

Shared semantic state, current send/wait behavior, and optional tmux popup presentation are specified in [Observer and popup design](observer-and-popup.md). The normal pane/window UI retains the navigation behavior below.

## Visibility and navigation

`src/tui.rs::pane_is_visible` strictly filters plain panes in agent-only mode, even with zero agents. `App::ensure_visible_selection` rebuilds spaces from visible panes and clears an invisible selection. The `a` key explicitly enables ordinary panes; the TUI's own pane remains excluded.

`UiCommand::Focus` keeps the event loop alive and displays focus errors instead of exiting. A jump to another window leaves the sidebar running in its original window. The sidebar does not follow windows. `tests/ui_smoke.py` exercises Enter navigation, a surviving dashboard, and subsequent q exit using an attached tmux client in a private PTY.

## Preview ownership

`src/tmux.rs::Tmux::pane_view` uses a published transcript when `@agent_preview_path` is set or `@agent_status_owner` is `lazyagent`. Missing files, absent transcript paths, or stopped publishers yield an error instead of capturing the editor's current buffer. `tests/tmux_smoke.rs::editor_publish_and_owner_safe_withdraw_flow_into_snapshot` covers these cases.

LazyAgent's integration selects an ACP session by status priority and publishes that session's transcript. Multiple hosted ACP sessions still share one tmux pane identity. Independent thread selection and metadata lifecycle races beyond the tested conditions remain outside this change.

## Preview scrolling

The TUI cycles focus between spaces, agents, and the visible preview with Tab/Shift-Tab. PageUp/PageDown scroll the preview from any panel. With preview focus, j/k or arrows scroll one row, Ctrl-u/Ctrl-d scroll half a page, Home/g goes to the oldest retained row, and End/G resumes live output. Wheel events over the preview scroll three rows; wheel events over list rows move that list's selection. Prompt and confirmation dialogs consume input without changing the underlying selection.

Ctrl-u in a list opens a registration-removal confirmation rather than immediately unmarking the selected agent. Only y/Y confirms this removal; Enter, Esc, and n/N cancel it. Preview Ctrl-u always scrolls and never opens that confirmation. This distinction matters especially for published ACP agents: `Tmux::unmark` clears identity and native status/owner fields, so agent-only filtering removes the pane until the publisher republishes. It does not delete the transcript or stop its agent. The footer reflects the focused panel's Ctrl-u meaning.

`Tmux::pane_view` remains a visible-screen read for normal panes and a bounded 256 KiB transcript tail for ACP. The LAN mirror retains this interface. On the first upward scroll, `Tmux::pane_history` captures all currently retained tmux rows with `capture-pane -ep -S -`, or reads the complete published ACP transcript. Both use the same publisher-liveness/path validation. Transcript reads stop at the file size observed when opened instead of chasing concurrent appends. History is loaded only on demand, into the TUI process; the observer does not retain screen history.

History mode freezes the captured text and its top row while output continues. Periodic preview reads do not replace it. Reaching the captured bottom or choosing End/G returns to current live output, including output received while reading. Changing pane or observer instance identity discards frozen history, including a changed ACP transcript path. Resizing preserves the snapshot and clamps the visible offset. The title distinguishes `history` with a row range from `live · auto-follow`; the live terminal cursor is hidden during history viewing. Rendering slices rows with a `usize` index before passing them to Ratatui, so histories beyond 65,535 rows do not wrap a u16 scroll offset.

These operations are passive: they do not enter copy mode, send input, or move the source application's viewport. Available normal-pane history is limited by tmux's retention and screen behavior. Alternate-screen applications may keep their transcript internally without putting it into tmux scrollback; scrolling that internal application history is not implemented. ACP history is limited to the published file; deleted/truncated output cannot be recovered. Loading an unusually large history can take time and memory because it is read and parsed as one snapshot.

`tests/preview_smoke.py` exercises the actual TUI in a private attached tmux/PTY: ordinary and ACP history, Ctrl-u/Ctrl-d without unregistering, list registration confirmation/cancellation, wheel hit testing, PageUp/PageDown, Home/End, stable text under concurrent output, ANSI/CJK, resize, agent switching, and transcript replacement. It also checks PageUp/Home/End inside an actual popup. `tests/tmux_smoke.rs` checks history retrieval without changing the source viewport and ACP history beyond the live tail, with publisher errors shared by both read paths.

## LAN mirror

[Android documentation](../../android/README.md) owns the build steps, connection workflow, endpoints, authentication, rendering limits, and server resource limits. `src/remote.rs` exposes agent-only snapshots and the shared `Tmux::pane_view`. `web/mirror.js` renders terminal content as text with SGR attributes and uses generation checks to discard stale asynchronous results after selection or connection changes.

Tokens remain in memory; only the Android connection address persists. HTTP LAN traffic is unencrypted. `POST /api/action` authorizes operator send/interrupt actions with the same bearer token. `tests/remote_smoke.py` verifies authorization, empty-agent filtering, rejection of ordinary pane capture, ACP content, and unavailable transcripts on isolated servers.

APK compilation and Android Lint do not verify physical-device LAN behavior. Android real-device validation remains necessary.

## QR pairing

`src/remote.rs::pairing_addresses` uses actual listener ports and one representative interface address of the bind's IP family; wildcard and loopback addresses are never encoded for phone pairing. Physical LAN interfaces are preferred over virtual interfaces, then the default-route interface and deterministic name/address ordering. `--advertise-address` selects one IP explicitly. Generated tokens use 128 random bits as 32 hex characters, shrinking the QR while preserving medium error correction and the quiet zone. `show_pairing` emits high-contrast terminal QR codes, and optionally creates a new mode-0600 SVG. QR payloads are ordinary HTTP URLs with `#token=...`, matching the existing browser fragment handling; pairing credentials are not served through a public endpoint.

Android `Pairing` validates both manually entered connections and scanned URLs. `ScanContract` provides camera scanning with runtime permission handling; `GetContent` grants access only to the selected image. `QrImage` samples gallery images to at most 2048 pixels per dimension and decodes on a single background worker, accepting exactly one distinct valid pairing payload. No Google Play Services, online QR service, or image upload is involved. Activity teardown discards pending image results. JVM tests cover URL validation and QR pixel decoding; camera optics and gallery provider behavior require device validation.

Development uses the SDK under `/tmp/agentmux-sdk`; the user permits this host-build workflow. Gradle dependencies use the existing user cache.

## Operator controls

`src/control.rs` checks current agent identity and a binding derived from pane process, owner PID, transcript, published socket, kind/name, foreground command and process detection before each action. Requests carry the binding selected in the UI. A process-wide mutex serializes terminal pastes and ACP actions. This is a freshness guard, not a transaction over agent lifecycle: an agent can still exit during a tmux operation.

CLI, TUI, and LAN now route this control path through the shared observer. Prompt sends to blocked agents are rejected; interrupt remains available. The instance fingerprint participates in the binding, and observer receipts support activity-correlated waits.

Terminal sends use a private named tmux buffer, bracketed paste when supported, and Enter; interrupt sends Ctrl+C. Plain panes, exited agents, editors without an ACP bridge, and stale bindings are unavailable. Known shells are unavailable unless process-based agent detection still finds an agent.

`Tmux::publish_agent` captures only the publishing Neovim job's `NVIM` environment as `@agent_control_socket`; arbitrary editor socket discovery is not used. The embedded `src/control_acp.lua` verifies the actual editor PID and finds exactly one ACP session matching the published transcript. It calls LazyAgent's `paste_and_submit` or `send_keys(..., 'C-c')`, without focusing or typing in editor buffers. Publishing state again enables the bridge for existing sessions. Neovim RPC timeout reports uncertain delivery and never retries automatically.

HTTP accepts bounded Content-Length JSON bodies, rejects duplicate lengths and transfer encoding, requires bearer authentication for both reads and writes, and exposes only send/interrupt actions. Unknown actions and stale targets are rejected. Frontend drafts are keyed by pane and binding, remain in memory, and survive selection changes or send errors. Successful send clears only the submitted draft. Selection is blocked during an in-flight action; disconnected or unavailable targets disable controls. `tests/remote_smoke.py` exercises real tmux input and an isolated Neovim RPC stub; `tests/mirror_ui.cjs` covers list collapse, drafts, request targeting and errors.

The agent list is a left-edge modal drawer on both phone and tablet widths, opened by a persistent side tab. Native dialog semantics trap focus; agent selection, the close button, backdrop click, and Escape dismiss it and restore focus to the side tab. The preview retains its full-width layout beneath the drawer.
