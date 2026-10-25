use agentmux::{
    config::LoadedConfig,
    tmux::{PublishedAgent, Tmux},
    tui,
};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::{env, io::Write};

#[derive(Debug, Parser)]
#[command(
    name = "agentmux",
    version,
    about = "A Herdr-inspired agent sidebar for tmux"
)]
struct Cli {
    /// Select a tmux socket by name (tmux -L).
    #[arg(long, global = true)]
    socket_name: Option<String>,

    /// Print the default configuration and exit.
    #[arg(long)]
    default_config: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage the shared, tmux-server-scoped state observer.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
    /// Wait for the current agent to reach a state (JSON result).
    Wait {
        target: String,
        #[arg(long, value_parser = parse_state)]
        until: Vec<agentmux::model::AgentState>,
        #[arg(long, default_value_t = 30000, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },
    /// Stream state events as NDJSON; starts with a snapshot by default.
    Events {
        #[arg(long, requires = "epoch")]
        since: Option<u64>,
        #[arg(long, requires = "since")]
        epoch: Option<String>,
        #[arg(long)]
        once: bool,
    },
    /// Serve a token-protected LAN mirror with send and interrupt controls.
    Serve {
        #[arg(long, default_value = "127.0.0.1:9876")]
        bind: std::net::SocketAddr,
        #[arg(long)]
        token_file: Option<std::path::PathBuf>,
        /// LAN IP to encode in the pairing QR (auto-detected for wildcard binds).
        #[arg(long)]
        advertise_address: Option<std::net::IpAddr>,
        /// Save a pairing QR as a new private SVG file.
        #[arg(long)]
        qr_svg: Option<std::path::PathBuf>,
    },
    /// Open the interactive sidebar.
    #[command(alias = "menu")]
    Ui {
        origin: Option<String>,
        #[arg(long)]
        client: Option<String>,
        /// Open in a floating tmux popup; Enter focuses the target and closes it.
        #[arg(long)]
        popup: bool,
        #[arg(long, default_value = "80%", value_parser = agentmux::popup::dimension)]
        width: String,
        #[arg(long, default_value = "80%", value_parser = agentmux::popup::dimension)]
        height: String,
        #[arg(long, hide = true)]
        popup_child: bool,
    },
    /// List tmux panes and inferred agent state.
    List {
        origin: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print pane metadata and recent output.
    Preview {
        target: String,
        #[arg(long, default_value_t = 120)]
        lines: usize,
    },
    /// Explain why a pane is or is not classified as an agent.
    Explain {
        target: String,
        #[arg(long)]
        json: bool,
    },
    /// Focus a pane or uniquely named agent.
    Focus {
        target: String,
        #[arg(long)]
        client: Option<String>,
    },
    /// Split a pane and start an agent.
    Spawn {
        kind: String,
        cwd: Option<String>,
        #[arg(long)]
        origin: Option<String>,
        #[arg(long)]
        command: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        client: Option<String>,
    },
    /// Submit a prompt to a current agent, optionally waiting for it.
    Send {
        target: String,
        #[arg(long)]
        text: Option<String>,
        /// Wait for activity and then a settled state after sending.
        #[arg(long)]
        wait: bool,
        #[arg(long, requires = "wait", value_parser = parse_state)]
        until: Vec<agentmux::model::AgentState>,
        #[arg(long, default_value_t = 30000, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
        #[arg(long)]
        json: bool,
    },
    /// Mark an existing pane as an agent.
    Mark {
        target: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    /// Clear agent metadata from a pane.
    Unmark { target: String },
    /// Respawn a pane using @agent_command when available.
    Respawn { target: String },
    /// Publish native agent state through tmux pane options.
    Status {
        target: String,
        state: String,
        message: Option<String>,
        #[arg(default_value_t = 0)]
        ttl: u64,
    },
    /// Clear a published native status.
    ClearStatus { target: String },
    /// Publish an editor-hosted agent and its native state.
    Publish {
        target: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        state: String,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        owner_pid: i32,
        #[arg(long)]
        preview_path: Option<String>,
    },
    /// Remove editor-hosted metadata when it belongs to the given owner.
    Withdraw {
        target: String,
        #[arg(long)]
        owner: String,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCommands {
    Start,
    Status,
    Stop,
    #[command(hide = true)]
    Run,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.default_config {
        print!("{}", LoadedConfig::default_toml());
        return Ok(());
    }
    let mut tmux = Tmux::new(cli.socket_name);
    let command = cli.command.unwrap_or(Commands::Ui {
        origin: None,
        client: None,
        popup: false,
        width: "80%".into(),
        height: "80%".into(),
        popup_child: false,
    });
    if let Commands::Daemon { command } = command {
        use agentmux::observer::{self, Request};
        match command {
            DaemonCommands::Run => observer::run(tmux)?,
            DaemonCommands::Start => {
                let path = observer::ensure_running(&mut tmux)?;
                println!("{}", path.display());
            }
            DaemonCommands::Status => {
                let path = observer::endpoint(&tmux)?;
                let status: serde_json::Value = observer::call(&path, &Request::Status)?;
                println!("{status}");
            }
            DaemonCommands::Stop => {
                let path = observer::endpoint(&tmux)?;
                observer::call::<()>(&path, &Request::Stop)?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while path.exists() {
                    anyhow::ensure!(
                        std::time::Instant::now() < deadline,
                        "observer is still stopping"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        return Ok(());
    }
    agentmux::observer::ensure_running(&mut tmux)?;
    let should_refresh = matches!(
        &command,
        Commands::Mark { .. }
            | Commands::Unmark { .. }
            | Commands::Respawn { .. }
            | Commands::Status { .. }
            | Commands::ClearStatus { .. }
            | Commands::Publish { .. }
            | Commands::Withdraw { .. }
            | Commands::Spawn { .. }
    );
    match command {
        Commands::Daemon { .. } => unreachable!(),
        Commands::Serve {
            bind,
            token_file,
            advertise_address,
            qr_svg,
        } => {
            agentmux::remote::serve(
                tmux.clone(),
                bind,
                token_file.as_deref(),
                advertise_address,
                qr_svg.as_deref(),
            )?;
        }
        Commands::Ui {
            origin,
            client,
            popup,
            width,
            height,
            popup_child,
        } => {
            if popup {
                agentmux::popup::open(&tmux, origin, client, &width, &height)?;
            } else {
                let client = client.or(tmux.caller_client(origin.as_deref())?);
                let _overlay = if popup_child {
                    client
                        .as_deref()
                        .map(|client| {
                            agentmux::observer::OverlayGuard::register(
                                tmux.observer_socket().unwrap(),
                                client,
                            )
                        })
                        .transpose()?
                } else {
                    None
                };
                tmux.set_viewer(client.clone());
                tui::run(tmux, origin, client, LoadedConfig::load(), popup_child)?;
                return Ok(());
            }
        }
        Commands::List { origin: _, json } => {
            let snapshot = tmux.snapshot()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                for pane in snapshot.panes {
                    println!(
                        "[{}]\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        pane.state.label(),
                        pane.display_name(),
                        pane.pane_id,
                        pane.location(),
                        pane.command,
                        pane.cwd,
                        pane.state_source,
                        pane.last_line
                    );
                }
            }
        }
        Commands::Preview { target, lines } => {
            let pane = tmux.resolve_pane(&target)?;
            print!("{}", tmux.preview(&pane.pane_id, lines)?);
        }
        Commands::Explain { target, json } => {
            let pane = tmux.resolve_pane(&target)?;
            let mut explanation = DetectionExplanation::from(&pane);
            explanation.manifest = agentmux::observer::call(
                tmux.observer_socket().unwrap(),
                &agentmux::observer::Request::Explain {
                    pane: pane.pane_id.clone(),
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&explanation)?);
            } else {
                println!("pane:            {} ({})", pane.pane_id, pane.location());
                println!("command:         {}", pane.command);
                println!(
                    "agent:           {}",
                    pane.agent_kind.as_deref().unwrap_or("not detected")
                );
                println!(
                    "identity source: {}",
                    pane.identity_source.as_deref().unwrap_or("none")
                );
                println!(
                    "process agent:   {}",
                    pane.process_agent.as_deref().unwrap_or("none")
                );
                println!("state:           {}", pane.state.label());
                println!("state source:    {}", pane.state_source);
                println!("screen checked:  {}", pane.screen_checked);
                println!("note:            {}", explanation.note);
                if let Some(manifest) = &explanation.manifest {
                    println!(
                        "manifest:        {}",
                        serde_json::to_string_pretty(manifest)?
                    );
                }
            }
        }
        Commands::Focus { target, client } => {
            let pane = tmux.resolve_pane(&target)?;
            tmux.focus_pane(&pane.pane_id, client.as_deref())?;
        }
        Commands::Spawn {
            kind,
            cwd,
            origin,
            mut command,
            mut name,
            client,
        } => {
            let origin = origin.or_else(|| env::var("TMUX_PANE").ok());
            let cwd = cwd
                .or_else(|| {
                    origin
                        .as_deref()
                        .and_then(|pane| tmux.resolve_pane(pane).ok())
                        .map(|pane| pane.cwd)
                })
                .unwrap_or(env::current_dir()?.to_string_lossy().into_owned());
            let mut effective_kind = kind.clone();
            if kind == "custom" {
                if command.is_none() {
                    command = Some(prompt("custom command> ")?);
                }
                if name.is_none() {
                    let fallback = command.as_deref().unwrap_or("custom");
                    name = Some(prompt_default("name", fallback)?);
                }
                effective_kind = name.clone().unwrap_or_else(|| "custom".into());
            }
            let pane = tmux.spawn_agent(
                &effective_kind,
                &cwd,
                origin.as_deref(),
                command.as_deref(),
                name.as_deref(),
                client.as_deref(),
            )?;
            println!("{pane}");
        }
        Commands::Send {
            target,
            text,
            wait,
            until,
            timeout,
            json,
        } => {
            let submitted_at = std::time::Instant::now();
            let pane = tmux.resolve_pane(&target)?;
            let text = match text {
                Some(text) => text,
                None => prompt("prompt> ")?,
            };
            if text.is_empty() {
                bail!("prompt must not be empty");
            }
            let info = agentmux::control::capabilities(&tmux, &pane)?;
            let receipt = agentmux::observer::action(
                tmux.observer_socket().unwrap(),
                agentmux::control::Request {
                    pane: pane.pane_id.clone(),
                    binding: info.binding,
                    action: agentmux::control::Action::Send,
                    text,
                },
            )?;
            if wait {
                let states = settled_states(until);
                let result = agentmux::observer::wait(
                    &tmux,
                    &pane.pane_id,
                    &states,
                    std::time::Duration::from_millis(timeout)
                        .saturating_sub(submitted_at.elapsed()),
                    Some(&receipt),
                )?;
                println!("{}", serde_json::to_string(&result)?);
            } else if json {
                println!("{}", serde_json::to_string(&receipt)?);
            }
        }
        Commands::Wait {
            target,
            until,
            timeout,
        } => {
            let result = agentmux::observer::wait(
                &tmux,
                &target,
                &settled_states(until),
                std::time::Duration::from_millis(timeout),
                None,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Commands::Events { since, epoch, once } => {
            let snapshot = tmux.snapshot()?;
            let info = snapshot.observer.as_ref().unwrap();
            let mut seq = since.unwrap_or(info.seq);
            let epoch = epoch.unwrap_or_else(|| info.epoch.clone());
            if since.is_none() {
                println!(
                    "{}",
                    serde_json::json!({"type":"snapshot", "snapshot":snapshot})
                );
            }
            loop {
                let events: Vec<agentmux::observer::Event> = agentmux::observer::call(
                    tmux.observer_socket().unwrap(),
                    &agentmux::observer::Request::Events {
                        epoch: epoch.clone(),
                        since: seq,
                    },
                )?;
                for event in events {
                    seq = event.seq;
                    println!("{}", serde_json::to_string(&event)?);
                }
                std::io::stdout().flush()?;
                if once {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        Commands::Mark {
            target,
            kind,
            name,
            command,
        } => {
            let pane = tmux.resolve_pane(&target)?;
            let kind = match kind {
                Some(kind) => kind,
                None => prompt("kind> ")?,
            };
            if kind.is_empty() {
                bail!("kind must not be empty");
            }
            let name = match name {
                Some(name) => name,
                None => prompt_default("name", &kind)?,
            };
            tmux.mark(&pane.pane_id, &kind, Some(&name), command.as_deref())?;
        }
        Commands::Unmark { target } => {
            let pane = tmux.resolve_pane(&target)?;
            tmux.unmark(&pane.pane_id)?;
        }
        Commands::Respawn { target } => {
            let pane = tmux.resolve_pane(&target)?;
            tmux.respawn(&pane.pane_id)?;
        }
        Commands::Status {
            target,
            state,
            message,
            ttl,
        } => {
            let pane = tmux.resolve_pane(&target)?;
            tmux.set_status(&pane.pane_id, &state, message.as_deref(), ttl)?;
        }
        Commands::ClearStatus { target } => {
            let pane = tmux.resolve_pane(&target)?;
            tmux.clear_status(&pane.pane_id)?;
        }
        Commands::Publish {
            target,
            kind,
            name,
            state,
            message,
            owner,
            owner_pid,
            preview_path,
        } => tmux.publish_agent(
            &target,
            PublishedAgent {
                kind: &kind,
                name: &name,
                state: &state,
                message: message.as_deref(),
                owner: &owner,
                owner_pid,
                preview_path: preview_path.as_deref(),
            },
        )?,
        Commands::Withdraw { target, owner } => {
            tmux.withdraw_agent(&target, &owner)?;
        }
    }
    if should_refresh {
        if let Some(path) = tmux.observer_socket() {
            agentmux::observer::refresh(path)?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct DetectionExplanation<'a> {
    pane_id: &'a str,
    location: String,
    command: &'a str,
    agent_kind: Option<&'a str>,
    agent_name: Option<&'a str>,
    identity_source: Option<&'a str>,
    process_agent: Option<&'a str>,
    state: &'a str,
    state_source: &'a str,
    screen_checked: bool,
    note: &'static str,
    manifest: Option<serde_json::Value>,
}

impl<'a> From<&'a agentmux::model::PaneRecord> for DetectionExplanation<'a> {
    fn from(pane: &'a agentmux::model::PaneRecord) -> Self {
        let note = if pane.agent_kind.is_none() {
            "screen text is never used as agent identity evidence"
        } else if pane.command.contains("vim") && pane.state_source == "editor:no native status" {
            "editor integrations must publish @agent_status for reliable state"
        } else {
            "screen text may refine state only after agent identity is established"
        };
        Self {
            pane_id: &pane.pane_id,
            location: pane.location(),
            command: &pane.command,
            agent_kind: pane.agent_kind.as_deref(),
            agent_name: pane.agent_name.as_deref(),
            identity_source: pane.identity_source.as_deref(),
            process_agent: pane.process_agent.as_deref(),
            state: pane.state.label(),
            state_source: &pane.state_source,
            screen_checked: pane.screen_checked,
            note,
            manifest: None,
        }
    }
}

fn parse_state(value: &str) -> std::result::Result<agentmux::model::AgentState, String> {
    agentmux::detect::normalize_status(value)
        .ok_or_else(|| "use idle, done, blocked, working, unknown, or dead".into())
}

fn settled_states(states: Vec<agentmux::model::AgentState>) -> Vec<agentmux::model::AgentState> {
    if states.is_empty() {
        vec![
            agentmux::model::AgentState::Idle,
            agentmux::model::AgentState::Done,
            agentmux::model::AgentState::Blocked,
        ]
    } else {
        states
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut value = String::new();
    std::io::stdin()
        .read_line(&mut value)
        .context("failed to read input")?;
    Ok(value.trim().to_owned())
}

fn prompt_default(label: &str, default: &str) -> Result<String> {
    let value = prompt(&format!("{label} [{default}]> "))?;
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value
    })
}
