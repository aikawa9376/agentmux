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

## QR pairing

`src/remote.rs::pairing_addresses` uses actual listener ports and one representative interface address of the bind's IP family; wildcard and loopback addresses are never encoded for phone pairing. Physical LAN interfaces are preferred over virtual interfaces, then the default-route interface and deterministic name/address ordering. `--advertise-address` selects one IP explicitly. Generated tokens use 128 random bits as 32 hex characters, shrinking the QR while preserving medium error correction and the quiet zone. `show_pairing` emits high-contrast terminal QR codes, and optionally creates a new mode-0600 SVG. QR payloads are ordinary HTTP URLs with `#token=...`, matching the existing browser fragment handling; pairing credentials are not served through a public endpoint.

Android `Pairing` validates both manually entered connections and scanned URLs. `ScanContract` provides camera scanning with runtime permission handling; `GetContent` grants access only to the selected image. `QrImage` samples gallery images to at most 2048 pixels per dimension and decodes on a single background worker, accepting exactly one distinct valid pairing payload. No Google Play Services, online QR service, or image upload is involved. Activity teardown discards pending image results. JVM tests cover URL validation and QR pixel decoding; camera optics and gallery provider behavior require device validation.

Development uses the SDK under `/tmp/agentmux-sdk`; the user permits this host-build workflow. Gradle dependencies use the existing user cache.
