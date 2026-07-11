use crate::tmux::Tmux;
use anyhow::Result;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{BufRead, BufReader, Write},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherEvent {
    Output(String),
    Topology,
    Error(String),
}

pub fn start(tmux: &Tmux, origin: &str, pane_ids: Vec<String>) -> Result<Receiver<WatcherEvent>> {
    let session = tmux.session_for_pane(origin)?;
    let socket_name = tmux.socket_name().map(str::to_owned);
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("agentmux-tmux-control".into())
        .spawn(move || watch(socket_name, session, pane_ids, sender))?;
    Ok(receiver)
}

fn watch(
    socket_name: Option<String>,
    session: String,
    pane_ids: Vec<String>,
    sender: Sender<WatcherEvent>,
) {
    if let Err(error) = watch_inner(socket_name, session, pane_ids, &sender) {
        let _ = sender.send(WatcherEvent::Error(error.to_string()));
    }
}

fn watch_inner(
    socket_name: Option<String>,
    session: String,
    pane_ids: Vec<String>,
    sender: &Sender<WatcherEvent>,
) -> Result<()> {
    let tmux = Tmux::new(socket_name.clone());
    let pair = native_pty_system().openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut command = CommandBuilder::new("tmux");
    if let Some(socket_name) = socket_name {
        command.args(["-L", &socket_name]);
    }
    command.args([
        "-C",
        "attach-session",
        "-f",
        "ignore-size,no-detach-on-destroy",
        "-t",
        &session,
    ]);
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    for pane in pane_ids {
        writeln!(writer, "refresh-client -A \"{pane}:on\"")?;
    }
    writer.flush()?;

    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&bytes);
        if let Some(pane) = output_pane(&line) {
            if sender.send(WatcherEvent::Output(pane.to_owned())).is_err() {
                break;
            }
        } else if is_topology_event(&line) {
            if let Ok(panes) = tmux.pane_ids() {
                for pane in panes {
                    writeln!(writer, "refresh-client -A \"{pane}:on\"")?;
                }
                writer.flush()?;
            }
            if sender.send(WatcherEvent::Topology).is_err() {
                break;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn output_pane(line: &str) -> Option<&str> {
    let mut fields = line.split_whitespace();
    match fields.next()? {
        "%output" | "%extended-output" => fields.next(),
        _ => None,
    }
}

fn is_topology_event(line: &str) -> bool {
    [
        "%sessions-changed",
        "%session-changed ",
        "%session-renamed ",
        "%session-window-changed ",
        "%window-add ",
        "%window-close ",
        "%window-renamed ",
        "%window-pane-changed ",
        "%layout-change ",
        "%pane-mode-changed ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_output_notifications() {
        assert_eq!(output_pane("%output %12 hello"), Some("%12"));
        assert_eq!(output_pane("%extended-output %9 123 : hello"), Some("%9"));
        assert_eq!(output_pane("%window-add @2"), None);
    }

    #[test]
    fn recognizes_topology_notifications() {
        assert!(is_topology_event("%layout-change @1 abc"));
        assert!(is_topology_event("%sessions-changed"));
        assert!(!is_topology_event("%output %1 text"));
    }
}
