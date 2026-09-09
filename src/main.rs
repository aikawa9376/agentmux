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
    /// Serve a token-protected, read-only LAN mirror.
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
    /// Send one prompt line and Enter to a pane.
    Send {
        target: String,
        #[arg(long)]
        text: Option<String>,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.default_config {
        print!("{}", LoadedConfig::default_toml());
        return Ok(());
    }
    let tmux = Tmux::new(cli.socket_name);
    match cli.command.unwrap_or(Commands::Ui {
        origin: None,
        client: None,
    }) {
        Commands::Serve {
            bind,
            token_file,
            advertise_address,
            qr_svg,
        } => {
            agentmux::remote::serve(
                tmux,
                bind,
                token_file.as_deref(),
                advertise_address,
                qr_svg.as_deref(),
            )?;
        }
        Commands::Ui { origin, client } => {
            tui::run(tmux, origin, client, LoadedConfig::load())?;
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
            let explanation = DetectionExplanation::from(&pane);
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
        Commands::Send { target, text } => {
            let pane = tmux.resolve_pane(&target)?;
            let text = match text {
                Some(text) => text,
                None => prompt("prompt> ")?,
            };
            if text.is_empty() {
                bail!("prompt must not be empty");
            }
            tmux.send_text(&pane.pane_id, &text, true)?;
            tmux.set_status(&pane.pane_id, "working", Some("prompt sent"), 15)?;
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
        }
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
