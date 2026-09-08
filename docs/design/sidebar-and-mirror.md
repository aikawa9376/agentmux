# Sidebar and mirror design

## Visibility and navigation

`src/tui.rs::pane_is_visible` strictly filters plain panes in agent-only mode, even with zero agents. `App::ensure_visible_selection` rebuilds spaces from visible panes and clears an invisible selection. The `a` key explicitly enables ordinary panes; the TUI's own pane remains excluded.

`UiCommand::Focus` keeps the event loop alive and displays focus errors instead of exiting. A jump to another window leaves the sidebar running in its original window. The sidebar does not follow windows. `tests/ui_smoke.py` exercises Enter navigation, a surviving dashboard, and subsequent q exit using an attached tmux client in a private PTY.

## Preview ownership

`src/tmux.rs::Tmux::pane_view` uses a published transcript when `@agent_preview_path` is set or `@agent_status_owner` is `lazyagent`. Missing files, absent transcript paths, or stopped publishers yield an error instead of capturing the editor's current buffer. `tests/tmux_smoke.rs::editor_publish_and_owner_safe_withdraw_flow_into_snapshot` covers these cases.

LazyAgent's integration selects an ACP session by status priority and publishes that session's transcript. Multiple hosted ACP sessions still share one tmux pane identity. Independent thread selection and metadata lifecycle races beyond the tested conditions remain outside this change.

## LAN mirror

[Android documentation](../../android/README.md) owns the build steps, connection workflow, endpoints, authentication, rendering limits, and server resource limits. `src/remote.rs` exposes agent-only snapshots and the shared `Tmux::pane_view`. `web/mirror.js` renders terminal content as text with SGR attributes and uses generation checks to discard stale asynchronous results after selection or connection changes.

Tokens remain in memory; only the Android connection address persists. HTTP LAN traffic is unencrypted. No remote input endpoint exists. `tests/remote_smoke.py` verifies authorization, empty-agent filtering, rejection of ordinary pane capture, ACP content, and unavailable transcripts on isolated servers.

APK compilation and Android Lint do not verify physical-device LAN behavior. Android real-device validation remains necessary.
