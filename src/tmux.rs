use crate::{
    detect::{
        detect_agent_command, last_line, normalize_status, one_line, screen_status_with_title,
        strip_ansi,
    },
    model::{AgentState, PaneRecord, Snapshot},
};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

const SEP: char = '\x1f';
const PANE_FORMAT: &str = concat!(
    "#{session_id}\x1f#{session_name}\x1f#{window_id}\x1f#{window_index}\x1f",
    "#{window_name}\x1f#{pane_id}\x1f#{pane_index}\x1f#{pane_current_command}\x1f",
    "#{pane_current_path}\x1f#{pane_title}\x1f#{pane_tty}\x1f#{pane_dead}\x1f",
    "#{window_active}\x1f#{pane_active}\x1f#{@agent_name}\x1f#{@agent_kind}\x1f",
    "#{@agent_command}\x1f#{@agent_created_at}\x1f#{@agent_status}\x1f",
    "#{@agent_status_at}\x1f#{@agent_status_ttl}\x1f#{@agent_status_message}\x1f",
    "#{@agent_status_pid}\x1f#{@agent_status_owner}"
);

#[derive(Debug, Clone, Default)]
pub struct Tmux {
    socket_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaneView {
    pub ansi: String,
    pub width: u16,
    pub height: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct PublishedAgent<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub state: &'a str,
    pub message: Option<&'a str>,
    pub owner: &'a str,
    pub owner_pid: i32,
    pub preview_path: Option<&'a str>,
}

impl Tmux {
    pub fn new(socket_name: Option<String>) -> Self {
        Self { socket_name }
    }

    pub fn socket_name(&self) -> Option<&str> {
        self.socket_name.as_deref()
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let process_map = process_map();
        let output = self.run(["list-panes", "-a", "-F", PANE_FORMAT])?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut panes = Vec::new();

        for line in output.lines().filter(|line| !line.is_empty()) {
            let fields: Vec<_> = line.split(SEP).collect();
            if fields.len() != 24 {
                continue;
            }
            let tty = fields[10].to_owned();
            let processes = process_map.get(&tty).map(Vec::as_slice).unwrap_or_default();
            let status_pid = fields[22].parse::<i32>().ok();
            let owned_metadata_stale = !fields[23].is_empty()
                && status_pid.is_some_and(|pid| pid > 0 && !process_alive(pid));
            let explicit_name = (!owned_metadata_stale)
                .then(|| nonempty(fields[14]))
                .flatten();
            let explicit_kind = (!owned_metadata_stale)
                .then(|| nonempty(fields[15]))
                .flatten();
            let process_agent = detect_agent_process(fields[7], processes);
            let (kind, identity_source) = if let Some(kind) = explicit_kind {
                (Some(kind), Some("pane-option:@agent_kind".to_owned()))
            } else if let Some(kind) = process_agent.clone() {
                (Some(kind), Some("process:tty".to_owned()))
            } else {
                (None, None)
            };
            let is_editor = is_editor_command(fields[7]);
            let mut screen = String::new();

            // Screen text is state evidence only. Using it as identity evidence makes an
            // editor displaying words such as "codex" look like a running agent.
            if kind.is_some() {
                screen = strip_ansi(&self.capture_screen(fields[5]).unwrap_or_default());
            }

            let dead = fields[11] == "1";
            let explicit_status = nonempty(fields[18]);
            let status_at = fields[19].parse::<u64>().unwrap_or_default();
            let status_ttl = fields[20].parse::<u64>().unwrap_or_default();
            let explicit_fresh = explicit_status.is_some()
                && !owned_metadata_stale
                && explicit_status_is_fresh(now, status_at, status_ttl, status_pid);

            let (state, source) = if dead {
                (AgentState::Dead, "process:dead".to_owned())
            } else if explicit_fresh {
                let raw = explicit_status.as_deref().unwrap_or_default();
                normalize_status(raw)
                    .map(|state| (state, format!("native:{raw}")))
                    .unwrap_or((AgentState::Unknown, "native:invalid".to_owned()))
            } else if let Some(agent_kind) = kind.as_deref() {
                if is_editor {
                    (AgentState::Unknown, "editor:no native status".to_owned())
                } else if is_shell_command(fields[7]) && process_agent.is_none() {
                    (AgentState::Idle, "process:agent exited".to_owned())
                } else {
                    let detected = screen_status_with_title(agent_kind, &screen, fields[9]);
                    (detected.state, detected.source)
                }
            } else if is_shell_command(fields[7]) {
                (AgentState::Idle, "process:shell".to_owned())
            } else {
                (AgentState::Plain, "process:pane".to_owned())
            };
            let screen_checked = kind.is_some();

            let pane = PaneRecord {
                session_id: fields[0].to_owned(),
                session_name: fields[1].to_owned(),
                window_id: fields[2].to_owned(),
                window_index: fields[3].parse().unwrap_or_default(),
                window_name: fields[4].to_owned(),
                pane_id: fields[5].to_owned(),
                pane_index: fields[6].parse().unwrap_or_default(),
                command: fields[7].to_owned(),
                cwd: fields[8].to_owned(),
                title: fields[9].to_owned(),
                tty,
                dead,
                window_active: fields[12] == "1",
                pane_active: fields[13] == "1",
                agent_kind: kind,
                agent_name: explicit_name,
                agent_command: nonempty(fields[16]),
                identity_source,
                process_agent,
                screen_checked,
                state,
                state_source: source,
                state_message: nonempty(fields[21]),
                last_line: if screen.is_empty() {
                    one_line(fields[9], 180)
                } else {
                    last_line(&screen)
                },
            };
            panes.push(pane);
        }

        Snapshot::sort_panes(&mut panes);
        let spaces = Snapshot::build_spaces(&panes);
        Ok(Snapshot { spaces, panes })
    }

    pub fn resolve_pane(&self, target: &str) -> Result<PaneRecord> {
        let snapshot = self.snapshot()?;
        let mut matches: Vec<_> = snapshot
            .panes
            .into_iter()
            .filter(|pane| {
                pane.pane_id == target
                    || pane.location() == target
                    || pane.agent_name.as_deref() == Some(target)
                    || pane.agent_kind.as_deref() == Some(target)
            })
            .collect();
        match matches.len() {
            0 => bail!("no pane or agent matches {target:?}"),
            1 => Ok(matches.remove(0)),
            _ => bail!("target {target:?} is ambiguous ({} matches)", matches.len()),
        }
    }

    pub fn capture_screen(&self, pane: &str) -> Result<String> {
        self.run(["capture-pane", "-pJ", "-t", pane])
    }

    pub fn preview(&self, pane: &str, lines: usize) -> Result<String> {
        let header = self.run([
            "display-message",
            "-p",
            "-t",
            pane,
            "pane #{pane_id}  #{session_name}:#{window_index}.#{pane_index}  #{pane_current_command}  #{pane_current_path}\nagent #{@agent_name}  kind #{@agent_kind}  native #{@agent_status}  message #{@agent_status_message}",
        ])?;
        let start = format!("-{}", lines.max(1));
        let body = self.run(["capture-pane", "-epJ", "-t", pane, "-S", &start])?;
        Ok(format!("{header}\n\n{body}"))
    }

    pub fn pane_view(&self, pane: &str) -> Result<PaneView> {
        let metadata = self.run([
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_width}\x1f#{pane_height}\x1f#{cursor_x}\x1f#{cursor_y}\x1f#{@agent_preview_path}\x1f#{@agent_status_pid}\x1f#{@agent_status_owner}",
        ])?;
        let fields: Vec<_> = metadata.trim().split(SEP).collect();
        if fields.len() != 7 {
            bail!("tmux returned invalid pane view metadata");
        }
        let width = fields[0].parse().context("invalid pane width")?;
        let height = fields[1].parse().context("invalid pane height")?;
        let owner_pid = fields[5].parse::<i32>().ok();
        // An owned transcript must never fall back to the editor's current buffer.
        // A missing/rotating file or a stopped publisher is an unavailable preview.
        if !fields[4].is_empty() || fields[6] == "lazyagent" {
            if fields[6].is_empty() || !owner_pid.is_some_and(|pid| pid > 0 && process_alive(pid)) {
                bail!("Published preview unavailable: publisher is no longer active");
            }
            if fields[4].is_empty() {
                bail!("Published preview unavailable: waiting for transcript");
            }
            let text = read_preview_tail(fields[4])?;
            let content_height = text.lines().count().clamp(1, u16::MAX as usize) as u16;
            return Ok(PaneView {
                ansi: colorize_acp_transcript(&text),
                width,
                height: content_height,
                cursor_x: 0,
                cursor_y: content_height.saturating_sub(1),
            });
        }
        Ok(PaneView {
            ansi: self.run(["capture-pane", "-ep", "-t", pane])?,
            width,
            height,
            cursor_x: fields[2].parse().context("invalid cursor x")?,
            cursor_y: fields[3].parse().context("invalid cursor y")?,
        })
    }

    pub fn focus_pane(&self, pane: &str, client: Option<&str>) -> Result<()> {
        let identity = self.run([
            "display-message",
            "-p",
            "-t",
            pane,
            "#{session_id}\x1f#{window_id}",
        ])?;
        let mut fields = identity.trim().split(SEP);
        let session_id = fields.next().context("missing session id")?;
        let window_id = fields.next().context("missing window id")?;

        let mut args = vec!["switch-client"];
        if let Some(client) = client {
            args.extend(["-c", client]);
        }
        args.extend(["-t", session_id]);
        self.run(args)?;
        self.run(["select-window", "-t", window_id])?;
        self.run(["select-pane", "-t", pane])?;
        Ok(())
    }

    pub fn spawn_agent(
        &self,
        kind: &str,
        cwd: &str,
        origin: Option<&str>,
        command: Option<&str>,
        name: Option<&str>,
        client: Option<&str>,
    ) -> Result<String> {
        let default_command = match kind {
            "cursor" => "cursor-agent",
            other => other,
        };
        let command = command.unwrap_or(default_command);
        let name = name.unwrap_or(kind);
        let mut args = vec!["split-window", "-h", "-c", cwd, "-P", "-F", "#{pane_id}"];
        if let Some(origin) = origin {
            args.extend(["-t", origin]);
        }
        args.push(command);
        let pane = self.run(args)?.trim().to_owned();
        if pane.is_empty() {
            bail!("tmux did not return a pane id");
        }
        self.set_option(&pane, "@agent_kind", kind)?;
        self.set_option(&pane, "@agent_name", name)?;
        self.set_option(&pane, "@agent_command", command)?;
        self.set_option(
            &pane,
            "@agent_created_at",
            &SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string(),
        )?;
        self.focus_pane(&pane, client)?;
        Ok(pane)
    }

    pub fn send_text(&self, pane: &str, text: &str, enter: bool) -> Result<()> {
        self.run(["send-keys", "-t", pane, "-l", text])?;
        if enter {
            self.run(["send-keys", "-t", pane, "Enter"])?;
        }
        Ok(())
    }

    pub fn respawn(&self, pane: &str) -> Result<()> {
        let command = self.show_option(pane, "@agent_command").unwrap_or_default();
        let cwd = self.run(["display-message", "-p", "-t", pane, "#{pane_current_path}"])?;
        if command.trim().is_empty() {
            self.run(["respawn-pane", "-k", "-t", pane])?;
        } else {
            self.run([
                "respawn-pane",
                "-k",
                "-c",
                cwd.trim(),
                "-t",
                pane,
                command.trim(),
            ])?;
        }
        Ok(())
    }

    pub fn kill_pane(&self, pane: &str) -> Result<()> {
        self.run(["kill-pane", "-t", pane])?;
        Ok(())
    }

    pub fn mark(
        &self,
        pane: &str,
        kind: &str,
        name: Option<&str>,
        command: Option<&str>,
    ) -> Result<()> {
        self.set_option(pane, "@agent_kind", kind)?;
        self.set_option(pane, "@agent_name", name.unwrap_or(kind))?;
        if let Some(command) = command.filter(|value| !value.is_empty()) {
            self.set_option(pane, "@agent_command", command)?;
        }
        Ok(())
    }

    pub fn unmark(&self, pane: &str) -> Result<()> {
        for option in [
            "@agent_kind",
            "@agent_name",
            "@agent_command",
            "@agent_created_at",
        ] {
            self.unset_option(pane, option)?;
        }
        self.clear_status(pane)
    }

    pub fn set_status(
        &self,
        pane: &str,
        state: &str,
        message: Option<&str>,
        ttl: u64,
    ) -> Result<()> {
        normalize_status(state).ok_or_else(|| anyhow!("unknown agent status {state:?}"))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.set_option(pane, "@agent_status", state)?;
        self.set_option(pane, "@agent_status_at", &now.to_string())?;
        self.set_option(pane, "@agent_status_ttl", &ttl.to_string())?;
        self.set_option(pane, "@agent_status_message", message.unwrap_or_default())?;
        self.set_option(pane, "@agent_status_pid", "")?;
        Ok(())
    }

    pub fn clear_status(&self, pane: &str) -> Result<()> {
        for option in [
            "@agent_status",
            "@agent_status_at",
            "@agent_status_ttl",
            "@agent_status_message",
            "@agent_status_pid",
            "@agent_status_owner",
        ] {
            self.unset_option(pane, option)?;
        }
        Ok(())
    }

    pub fn publish_agent(&self, pane: &str, agent: PublishedAgent<'_>) -> Result<()> {
        let normalized = normalize_status(agent.state)
            .ok_or_else(|| anyhow!("unknown agent status {:?}", agent.state))?;
        if agent.kind.trim().is_empty()
            || agent.name.trim().is_empty()
            || agent.owner.trim().is_empty()
        {
            bail!("kind, name, and owner must not be empty");
        }
        if agent.owner_pid <= 0 {
            bail!("owner-pid must be a positive process id");
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        let owner_pid = agent.owner_pid.to_string();
        let values = [
            ("@agent_kind", agent.kind),
            ("@agent_name", agent.name),
            ("@agent_status", normalized.label()),
            ("@agent_status_at", now.as_str()),
            ("@agent_status_ttl", "0"),
            ("@agent_status_message", agent.message.unwrap_or_default()),
            ("@agent_status_pid", owner_pid.as_str()),
            ("@agent_status_owner", agent.owner),
            (
                "@agent_preview_path",
                agent.preview_path.unwrap_or_default(),
            ),
        ];
        for (option, value) in values {
            self.set_option(pane, option, value)?;
        }
        Ok(())
    }

    pub fn withdraw_agent(&self, pane: &str, owner: &str) -> Result<bool> {
        let output = self.output(["show-option", "-pv", "-t", pane, "@agent_status_owner"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("invalid option") || stderr.contains("unknown option") {
                return Ok(false);
            }
            bail!("tmux show-option failed: {}", stderr.trim());
        }
        if String::from_utf8_lossy(&output.stdout).trim() != owner {
            return Ok(false);
        }
        for option in [
            "@agent_kind",
            "@agent_name",
            "@agent_command",
            "@agent_created_at",
            "@agent_status",
            "@agent_status_at",
            "@agent_status_ttl",
            "@agent_status_message",
            "@agent_status_pid",
            "@agent_status_owner",
            "@agent_preview_path",
        ] {
            self.unset_option(pane, option)?;
        }
        Ok(true)
    }

    pub fn session_for_pane(&self, pane: &str) -> Result<String> {
        Ok(self
            .run(["display-message", "-p", "-t", pane, "#{session_name}"])?
            .trim()
            .to_owned())
    }

    pub fn pane_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .run(["list-panes", "-a", "-F", "#{pane_id}"])?
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub fn set_option(&self, pane: &str, option: &str, value: &str) -> Result<()> {
        self.run(["set-option", "-pt", pane, option, value])?;
        Ok(())
    }

    pub fn unset_option(&self, pane: &str, option: &str) -> Result<()> {
        let output = self.output(["set-option", "-upt", pane, option])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("invalid option") && !stderr.contains("unknown option") {
                bail!("tmux set-option failed: {}", stderr.trim());
            }
        }
        Ok(())
    }

    pub fn show_option(&self, pane: &str, option: &str) -> Result<String> {
        self.run(["show-option", "-pv", "-t", pane, option])
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        if let Some(socket_name) = &self.socket_name {
            command.args(["-L", socket_name]);
        }
        command
    }

    fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(
                "tmux command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn output<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command()
            .args(args)
            .output()
            .context("failed to execute tmux")
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn read_preview_tail(path: &str) -> Result<String> {
    const MAX_BYTES: u64 = 256 * 1024;
    let mut file = File::open(path).context("failed to open published preview")?;
    let length = file.metadata()?.len();
    let start = length.saturating_sub(MAX_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes)?;
    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn colorize_acp_transcript(text: &str) -> String {
    if text.contains('\x1b') {
        return text.to_owned();
    }

    const RESET: &str = "\x1b[0m";
    const USER: &str = "\x1b[1;38;2;137;180;250m";
    const ASSISTANT: &str = "\x1b[1;38;2;166;227;161m";
    const SYSTEM: &str = "\x1b[1;38;2;186;194;222m";
    const ERROR: &str = "\x1b[1;38;2;243;139;168m";
    const TOOL: &str = "\x1b[1;38;2;249;226;175m";
    const CODE: &str = "\x1b[38;2;148;226;213m";
    const HEADING: &str = "\x1b[1;38;2;203;166;247m";

    let mut output = String::with_capacity(text.len() + text.lines().count() * 12);
    let mut in_code = false;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let newline = if line.ends_with('\n') { "\n" } else { "" };
        let lower = body.to_lowercase();
        let color = if body.trim_start().starts_with("```") {
            in_code = !in_code;
            Some(HEADING)
        } else if in_code {
            Some(CODE)
        } else if body.starts_with('─') || body.starts_with('╭') {
            if lower.contains("error") || body.contains('󰅚') {
                Some(ERROR)
            } else if lower.contains("user") || body.contains('󰍩') {
                Some(USER)
            } else if lower.contains("system") || body.contains('󰋽') {
                Some(SYSTEM)
            } else if lower.contains("tool") {
                Some(TOOL)
            } else {
                Some(ASSISTANT)
            }
        } else if body.starts_with('#') {
            Some(HEADING)
        } else {
            None
        };
        if let Some(color) = color {
            output.push_str(color);
            output.push_str(body);
            output.push_str(RESET);
            output.push_str(newline);
        } else {
            output.push_str(line);
        }
    }
    output
}

fn is_shell_command(command: &str) -> bool {
    matches!(command, "zsh" | "bash" | "fish" | "sh" | "nu" | "pwsh")
}

fn is_editor_command(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    command.contains("nvim") || command == "vim" || command.ends_with("vim")
}

fn explicit_status_is_fresh(now: u64, changed_at: u64, ttl: u64, owner_pid: Option<i32>) -> bool {
    if let Some(pid) = owner_pid.filter(|pid| *pid > 0) {
        if !process_alive(pid) {
            return false;
        }
    }
    ttl == 0 || changed_at == 0 || now.saturating_sub(changed_at) <= ttl
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: kill(pid, 0) performs existence/permission checking and sends no signal.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    command: String,
    args: String,
}

fn detect_agent_process(current_command: &str, processes: &[ProcessInfo]) -> Option<String> {
    detect_agent_executable(current_command).or_else(|| {
        processes.iter().find_map(|process| {
            let argv0 = process.args.split_whitespace().next().unwrap_or_default();
            detect_agent_executable(&process.command)
                .or_else(|| detect_agent_executable(argv0))
                .or_else(|| {
                    (is_runtime_wrapper(&process.command) || is_runtime_wrapper(argv0))
                        .then(|| detect_wrapped_agent(&process.args))
                        .flatten()
                })
        })
    })
}

fn detect_agent_executable(command: &str) -> Option<String> {
    let executable = command.rsplit('/').next().unwrap_or(command);
    detect_agent_command(executable)
}

fn is_runtime_wrapper(command: &str) -> bool {
    let executable = command.rsplit('/').next().unwrap_or(command);
    matches!(
        executable,
        "node"
            | "nodejs"
            | "bun"
            | "deno"
            | "python"
            | "python3"
            | "uv"
            | "npx"
            | "npm"
            | "pnpm"
            | "yarn"
    ) || executable.starts_with("node-")
        || executable.starts_with("node_")
}

fn detect_wrapped_agent(args: &str) -> Option<String> {
    let mut words = args.split_whitespace();
    words.next()?;
    let mut candidate = words.find(|word| !word.starts_with('-'))?;
    if matches!(candidate, "exec" | "run" | "dlx" | "x") {
        candidate = words.find(|word| !word.starts_with('-'))?;
    }
    detect_agent_executable(candidate).or_else(|| detect_agent_command(candidate))
}

fn process_map() -> HashMap<String, Vec<ProcessInfo>> {
    let Ok(output) = Command::new("ps")
        .args(["-eo", "tty=,comm=,args="])
        .output()
    else {
        return HashMap::new();
    };
    let mut map = HashMap::<String, Vec<ProcessInfo>>::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let (Some(tty), Some(command)) = (fields.next(), fields.next()) else {
            continue;
        };
        if tty == "?" {
            continue;
        }
        let args = fields.collect::<Vec<_>>().join(" ");
        map.entry(format!("/dev/{tty}"))
            .or_default()
            .push(ProcessInfo {
                command: command.to_owned(),
                args,
            });
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_expires_native_status() {
        assert!(explicit_status_is_fresh(100, 90, 15, None));
        assert!(!explicit_status_is_fresh(110, 90, 15, None));
        assert!(explicit_status_is_fresh(10_000, 1, 0, None));
    }

    #[test]
    fn editor_arguments_are_not_agent_evidence() {
        let processes = vec![ProcessInfo {
            command: "nvim".into(),
            args: "nvim notes-about-codex.md".into(),
        }];
        assert_eq!(detect_agent_process("nvim", &processes), None);
    }

    #[test]
    fn runtime_wrapped_agents_are_detected() {
        let processes = vec![ProcessInfo {
            command: "node".into(),
            args: "node /opt/codex/bin/codex".into(),
        }];
        assert_eq!(
            detect_agent_process("node", &processes),
            Some("codex".into())
        );
    }

    #[test]
    fn runtime_arguments_are_not_scanned_as_processes() {
        let processes = vec![ProcessInfo {
            command: "node".into(),
            args: "node /opt/language-server/server.js notes-about-codex.md".into(),
        }];
        assert_eq!(detect_agent_process("nvim", &processes), None);
    }

    #[test]
    fn copilot_native_process_is_detected_from_argv_zero() {
        let processes = vec![ProcessInfo {
            command: "MainThread".into(),
            args: "/usr/lib/node_modules/@github/copilot/node_modules/@github/copilot-linux-x64/copilot".into(),
        }];
        assert_eq!(
            detect_agent_process("node", &processes),
            Some("copilot".into())
        );
    }

    #[test]
    fn renamed_node_thread_detects_copilot_loader() {
        let processes = vec![ProcessInfo {
            command: "node-MainThread".into(),
            args: "node /usr/bin/copilot".into(),
        }];
        assert_eq!(
            detect_agent_process("node", &processes),
            Some("copilot".into())
        );
    }

    #[test]
    fn package_paths_detect_agents_with_generic_entrypoints() {
        let fixtures = [
            (
                "node /usr/lib/node_modules/@github/copilot/index.js",
                "copilot",
            ),
            (
                "node /usr/lib/node_modules/@google/gemini-cli/dist/index.js",
                "gemini",
            ),
            (
                "node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                "claude",
            ),
        ];
        for (args, expected) in fixtures {
            let processes = vec![ProcessInfo {
                command: "node".into(),
                args: args.into(),
            }];
            assert_eq!(
                detect_agent_process("node", &processes).as_deref(),
                Some(expected),
                "failed to detect {args}"
            );
        }
    }

    #[test]
    fn acp_transcript_colors_roles_without_adding_a_background() {
        let colored = colorize_acp_transcript(
            "─ 󰍩 User\nhello\n─ 󰭹 Codex\n```rust\nfn main() {}\n```\n─ 󰅚 Error\n",
        );
        assert!(colored.contains("\x1b[1;38;2;137;180;250m─ 󰍩 User"));
        assert!(colored.contains("\x1b[1;38;2;166;227;161m─ 󰭹 Codex"));
        assert!(colored.contains("\x1b[38;2;148;226;213mfn main() {}"));
        assert!(colored.contains("\x1b[1;38;2;243;139;168m─ 󰅚 Error"));
        assert!(!colored.contains("48;2;"));
    }
}
