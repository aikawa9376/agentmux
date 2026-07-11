use agentmux::{model::AgentState, tmux::Tmux, watcher};
use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime},
};

struct TestServer {
    socket: String,
}

impl TestServer {
    fn start() -> Option<Self> {
        Self::start_with("sleep 30")
    }

    fn start_with(pane_command: &str) -> Option<Self> {
        if Command::new("tmux").arg("-V").output().is_err() {
            return None;
        }
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket = format!("agentmux-test-{}-{nonce}", std::process::id());
        let status = Command::new("tmux")
            .args([
                "-L",
                &socket,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "alpha",
                "-n",
                "main",
                pane_command,
            ])
            .status()
            .ok()?;
        status.success().then_some(Self { socket })
    }
}

#[test]
fn control_mode_watcher_receives_pane_output() {
    let Some(server) = TestServer::start_with("cat") else {
        eprintln!("tmux unavailable; skipping integration smoke test");
        return;
    };
    let tmux = Tmux::new(Some(server.socket.clone()));
    let pane_id = tmux.snapshot().unwrap().panes[0].pane_id.clone();
    let receiver = watcher::start(&tmux, &pane_id, vec![pane_id.clone()]).unwrap();
    thread::sleep(Duration::from_millis(150));
    tmux.send_text(&pane_id, "agentmux watcher smoke", true)
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut observed = false;
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(watcher::WatcherEvent::Output(output_pane)) if output_pane == pane_id => {
                observed = true;
                break;
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    assert!(observed, "control-mode watcher did not receive pane output");
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .status();
    }
}

#[test]
fn pane_options_flow_into_snapshot_and_rollup() {
    let Some(server) = TestServer::start() else {
        eprintln!("tmux unavailable; skipping integration smoke test");
        return;
    };
    let tmux = Tmux::new(Some(server.socket.clone()));
    let initial = tmux.snapshot().unwrap();
    let pane_id = initial.panes[0].pane_id.clone();

    tmux.mark(&pane_id, "codex", Some("reviewer"), None)
        .unwrap();
    tmux.set_status(&pane_id, "blocked", Some("approval required"), 30)
        .unwrap();

    let snapshot = tmux.snapshot().unwrap();
    assert_eq!(snapshot.panes[0].agent_name.as_deref(), Some("reviewer"));
    assert_eq!(snapshot.panes[0].state, AgentState::Blocked);
    assert_eq!(snapshot.panes[0].state_source, "native:blocked");
    assert_eq!(snapshot.spaces[0].state, AgentState::Blocked);
    assert_eq!(snapshot.spaces[0].agent_count, 1);

    tmux.clear_status(&pane_id).unwrap();
    tmux.unmark(&pane_id).unwrap();
    assert!(!tmux.snapshot().unwrap().panes[0].is_agent());
}

#[test]
fn target_resolution_rejects_ambiguous_agent_kinds() {
    let Some(server) = TestServer::start() else {
        eprintln!("tmux unavailable; skipping integration smoke test");
        return;
    };
    let tmux = Tmux::new(Some(server.socket.clone()));
    let first = tmux.snapshot().unwrap().panes[0].pane_id.clone();
    let output = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "split-window",
            "-t",
            &first,
            "-P",
            "-F",
            "#{pane_id}",
            "sleep 30",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let second = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    tmux.mark(&first, "codex", Some("reviewer"), None).unwrap();
    tmux.mark(&second, "codex", Some("implementer"), None)
        .unwrap();

    assert!(
        tmux.resolve_pane("codex")
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    assert_eq!(tmux.resolve_pane("reviewer").unwrap().pane_id, first);
}

#[test]
fn nvim_displaying_agent_names_is_not_an_agent() {
    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("nvim unavailable; skipping regression test");
        return;
    }
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!("agentmux-editor-{nonce}.txt"));
    fs::write(
        &fixture,
        "codex and claude are words in this ordinary editor buffer\n",
    )
    .unwrap();
    let command = format!("nvim -u NONE -n {}", fixture.display());
    let Some(server) = TestServer::start_with(&command) else {
        let _ = fs::remove_file(&fixture);
        eprintln!("tmux unavailable; skipping regression test");
        return;
    };
    thread::sleep(Duration::from_millis(250));

    let pane = Tmux::new(Some(server.socket.clone()))
        .snapshot()
        .unwrap()
        .panes
        .remove(0);
    assert_eq!(pane.command, "nvim");
    assert!(!pane.is_agent());
    assert_eq!(pane.identity_source, None);
    assert!(!pane.screen_checked);

    drop(server);
    let _ = fs::remove_file(fixture);
}

#[test]
fn tmux_config_replaces_stale_popup_bindings_with_panes() {
    let Some(server) = TestServer::start() else {
        eprintln!("tmux unavailable; skipping integration smoke test");
        return;
    };
    let old = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "bind-key",
            "-n",
            "M-a",
            "display-popup",
            "legacy-agentmux",
        ])
        .status()
        .unwrap();
    assert!(old.success());

    let config = format!("{}/contrib/agentmux.tmux", env!("CARGO_MANIFEST_DIR"));
    let sourced = Command::new("tmux")
        .args(["-L", &server.socket, "source-file", &config])
        .status()
        .unwrap();
    assert!(sourced.success());

    let keys = Command::new("tmux")
        .args(["-L", &server.socket, "list-keys"])
        .output()
        .unwrap();
    let keys = String::from_utf8(keys.stdout).unwrap();
    let agentmux_keys: Vec<_> = keys
        .lines()
        .filter(|line| line.contains("agentmux"))
        .collect();
    assert!(
        agentmux_keys
            .iter()
            .any(|line| line.contains("M-a") && line.contains("split-window"))
    );
    assert!(
        agentmux_keys
            .iter()
            .any(|line| line.contains(" a ") && line.contains("new-window"))
    );
    assert!(
        agentmux_keys
            .iter()
            .all(|line| !line.contains("display-popup"))
    );
    assert!(
        agentmux_keys
            .iter()
            .all(|line| !line.contains("legacy-agentmux"))
    );
}
