//! Small, target-bound operator actions shared by the LAN client.
use crate::{model::PaneRecord, tmux::Tmux};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    hash::{Hash, Hasher},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct Controls {
    pub binding: String,
    pub available: bool,
    pub mode: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub pane: String,
    pub binding: String,
    pub action: Action,
    #[serde(default)]
    pub text: String,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Send,
    Interrupt,
}

fn metadata(tmux: &Tmux, pane: &str) -> Result<Vec<String>> {
    Ok(tmux
        .control_metadata(pane)?
        .trim_end_matches('\n')
        .split('\x1f')
        .map(str::to_owned)
        .collect())
}

fn describe(pane: &PaneRecord, fields: &[String]) -> Controls {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fields.hash(&mut hasher);
    pane.agent_kind.hash(&mut hasher);
    pane.agent_name.hash(&mut hasher);
    pane.command.hash(&mut hasher);
    pane.process_agent.hash(&mut hasher);
    let owner = fields.get(1).map(String::as_str).unwrap_or("");
    let mode = if owner == "lazyagent" {
        "acp"
    } else {
        "terminal"
    };
    let command = pane.command.to_ascii_lowercase();
    let editor = command.contains("nvim")
        || command.ends_with("vim")
        || matches!(command.as_str(), "vi" | "view" | "emacs" | "emacsclient");
    let available = pane.is_agent()
        && !pane.dead
        && fields.len() == 5
        && if mode == "acp" {
            !fields[3].is_empty() && !fields[4].is_empty()
        } else {
            owner.is_empty()
                && !editor
                && (pane.process_agent.is_some()
                    || !matches!(pane.command.as_str(), "sh" | "bash" | "zsh" | "fish" | "nu"))
        };
    Controls {
        binding: format!("{:016x}", hasher.finish()),
        available,
        mode,
    }
}

pub fn capabilities(tmux: &Tmux, pane: &PaneRecord) -> Result<Controls> {
    Ok(describe(pane, &metadata(tmux, &pane.pane_id)?))
}

pub fn execute(tmux: &Tmux, request: Request) -> Result<()> {
    static ACTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ACTION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if request.action == Action::Send
        && (request.text.trim().is_empty()
            || request.text.len() > 12000
            || request
                .text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'))
    {
        bail!("invalid prompt");
    }
    let snapshot = tmux.snapshot()?;
    let Some(pane) = snapshot.pane(&request.pane) else {
        bail!("agent unavailable");
    };
    let fields = metadata(tmux, &request.pane)?;
    let controls = describe(pane, &fields);
    if !controls.available || controls.binding != request.binding {
        bail!("agent changed or controls unavailable");
    }
    if controls.mode == "acp" {
        let pid: u32 = fields[2].parse()?;
        let args = serde_json::json!({"pid": pid, "preview": fields[3], "action": request.action, "text": request.text});
        let expr = format!(
            "luaeval('{}', json_decode('{}'))",
            include_str!("control_acp.lua").replace('\'', "''"),
            args.to_string().replace('\'', "''")
        );
        let mut child = Command::new("nvim")
            .args(["--server", &fields[4], "--remote-expr", &expr])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("ACP response timed out; delivery may have occurred");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let result = child.wait_with_output()?;
        if !result.status.success() || String::from_utf8_lossy(&result.stdout).trim() != "accepted"
        {
            bail!("ACP rejected the action");
        }
    } else {
        match request.action {
            Action::Send => tmux.paste_prompt(&request.pane, &request.text)?,
            Action::Interrupt => tmux.interrupt(&request.pane)?,
        }
    }
    Ok(())
}
