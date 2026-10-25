use crate::tmux::Tmux;
use anyhow::{Context, Result, ensure};

/// Open a separate popup PTY; the child never opens another popup recursively.
pub fn open(
    tmux: &Tmux,
    origin: Option<String>,
    client: Option<String>,
    width: &str,
    height: &str,
) -> Result<()> {
    let origin = origin.or_else(|| std::env::var("TMUX_PANE").ok()).context(
        "popup requires tmux; pass an origin pane and --client when launching externally",
    )?;
    let context = tmux.run([
        "display-message",
        "-p",
        "-t",
        &origin,
        "#{pane_current_path}",
    ])?;
    let cwd = context.trim_end_matches('\n');
    let client = client
        .or(tmux.caller_client(Some(&origin))?)
        .unwrap_or_default();
    ensure!(
        !client.is_empty(),
        "popup requires an attached tmux client; pass --client"
    );
    let executable = std::env::current_exe()?;
    let mut command = vec![executable.to_string_lossy().into_owned()];
    if let Some(socket) = tmux.socket_name() {
        command.extend(["--socket-name".into(), socket.into()]);
    }
    command.extend([
        "ui".into(),
        origin,
        "--client".into(),
        client.clone(),
        "--popup-child".into(),
    ]);
    let command = command
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let mut args = vec![
        "display-popup".to_owned(),
        "-E".into(),
        "-w".into(),
        width.into(),
        "-h".into(),
        height.into(),
        "-t".into(),
        client,
        "-d".into(),
        cwd.into(),
    ];
    for name in ["AGENTMUX_CONFIG_PATH", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR"] {
        if let Ok(value) = std::env::var(name) {
            args.extend(["-e".into(), format!("{name}={value}")]);
        }
    }
    args.push(format!("exec {command}"));
    tmux.run(args)?;
    Ok(())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn dimension(value: &str) -> Result<String, String> {
    let number = value.strip_suffix('%').unwrap_or(value);
    let valid = number
        .parse::<u16>()
        .ok()
        .is_some_and(|n| n > 0 && (!value.ends_with('%') || n <= 100));
    if valid {
        Ok(value.to_owned())
    } else {
        Err("use a positive cell count or 1%..100%".into())
    }
}
