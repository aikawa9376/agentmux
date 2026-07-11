use crate::{
    config::LoadedConfig,
    detect::{detect_agent_command, strip_ansi},
    model::{AgentState, PaneRecord, Snapshot},
    tmux::Tmux,
    watcher::{self, WatcherEvent},
};
use ansi_to_tui::IntoText;
use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    style::force_color_output,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    env,
    io::{self, Stdout},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

// Catppuccin Mocha accents, while the background stays terminal-native.
const SURFACE_DIM: Color = Color::Rgb(30, 30, 46);
const OVERLAY0: Color = Color::Rgb(108, 112, 134);
const TEXT: Color = Color::Rgb(205, 214, 244);
const SUBTEXT0: Color = Color::Rgb(166, 173, 200);
const ACCENT: Color = Color::Rgb(137, 180, 250);
const GREEN: Color = Color::Rgb(166, 227, 161);
const YELLOW: Color = Color::Rgb(249, 226, 175);
const RED: Color = Color::Rgb(243, 139, 168);
const TEAL: Color = Color::Rgb(148, 226, 213);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPanel {
    Spaces,
    Agents,
}

#[derive(Debug, Clone)]
enum PromptMode {
    Send { pane: String },
    SpawnCommand,
    SpawnName { command: String },
    MarkKind { pane: String },
    MarkName { pane: String, kind: String },
}

#[derive(Debug, Clone)]
struct PromptState {
    mode: PromptMode,
    input: String,
}

#[derive(Debug, Clone)]
enum ConfirmAction {
    Kill(String),
    Respawn(String),
}

#[derive(Debug)]
enum UiCommand {
    Quit,
    Refresh,
    Focus(String),
    Spawn {
        kind: String,
        command: Option<String>,
        name: Option<String>,
    },
    Send {
        pane: String,
        text: String,
    },
    Mark {
        pane: String,
        kind: String,
        name: String,
    },
    Unmark(String),
    Kill(String),
    Respawn(String),
}

struct App {
    snapshot: Snapshot,
    focus: FocusPanel,
    space_index: usize,
    selected_pane: Option<String>,
    show_plain: bool,
    preview: Text<'static>,
    status: String,
    prompt: Option<PromptState>,
    confirm: Option<ConfirmAction>,
    space_rows: Vec<(Rect, usize)>,
    agent_rows: Vec<(Rect, String)>,
    workspace_ratio: f32,
    preview_min_width: u16,
    animation_frame: usize,
}

pub fn run(
    tmux: Tmux,
    origin: Option<String>,
    client: Option<String>,
    loaded: LoadedConfig,
) -> Result<()> {
    // agentmux is a color-semantic TUI: state and captured pane colors carry
    // information, so preserve them even when the parent shell exports NO_COLOR.
    force_color_output(true);
    let origin = origin
        .or_else(|| env::var("TMUX_PANE").ok())
        .context("agentmux ui must run inside tmux or receive --origin")?;
    let snapshot = tmux.snapshot()?;
    let mut app = App::new(snapshot, &tmux, &origin, &loaded)?;
    let pane_ids = app
        .snapshot
        .panes
        .iter()
        .map(|pane| pane.pane_id.clone())
        .collect();
    let watcher = match watcher::start(&tmux, &origin, pane_ids) {
        Ok(receiver) => Some(receiver),
        Err(error) => {
            app.status = format!("event watcher unavailable; Ctrl-G refresh: {error}");
            None
        }
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;

    let result = run_loop(
        &mut terminal,
        &tmux,
        &origin,
        client.as_deref(),
        watcher.as_ref(),
        &loaded,
        &mut app,
    );

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    tmux: &Tmux,
    origin: &str,
    client: Option<&str>,
    watcher: Option<&Receiver<WatcherEvent>>,
    loaded: &LoadedConfig,
    app: &mut App,
) -> Result<()> {
    let preview_interval = Duration::from_millis(loaded.config.refresh.preview_ms);
    let status_interval = Duration::from_millis(loaded.config.refresh.status_ms);
    let mut last_preview = Instant::now() - preview_interval;
    let mut last_status = Instant::now() - status_interval;
    let mut pending_status = false;
    let mut pending_preview = false;

    loop {
        terminal.draw(|frame| app.render(frame))?;

        if let Some(watcher) = watcher {
            loop {
                match watcher.try_recv() {
                    Ok(WatcherEvent::Output(pane)) => {
                        pending_status = true;
                        pending_preview |= app.selected_pane.as_deref() == Some(pane.as_str());
                    }
                    Ok(WatcherEvent::Topology) => pending_status = true,
                    Ok(WatcherEvent::Error(error)) => {
                        app.status = format!("event watcher stopped: {error}");
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
        }

        if pending_preview && last_preview.elapsed() >= preview_interval {
            app.reload_preview(tmux);
            last_preview = Instant::now();
            pending_preview = false;
        }
        if pending_status && last_status.elapsed() >= status_interval {
            app.reload(tmux);
            last_status = Instant::now();
            pending_status = false;
            app.reload_preview(tmux);
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let command = match event::read()? {
            Event::Key(key) if key.kind == event::KeyEventKind::Press => app.handle_key(key),
            Event::Mouse(mouse) => {
                let changed = app.handle_mouse(mouse);
                if changed {
                    app.reload_preview(tmux);
                }
                None
            }
            Event::Resize(_, _) => {
                app.reload_preview(tmux);
                None
            }
            _ => None,
        };

        let Some(command) = command else {
            continue;
        };
        match command {
            UiCommand::Quit => break,
            UiCommand::Refresh => {
                app.reload(tmux);
                app.reload_preview(tmux);
                last_status = Instant::now();
            }
            UiCommand::Focus(pane) => {
                tmux.focus_pane(&pane, client)?;
                break;
            }
            UiCommand::Spawn {
                kind,
                command,
                name,
            } => {
                let cwd = app.origin_cwd(origin);
                tmux.spawn_agent(
                    &kind,
                    &cwd,
                    Some(origin),
                    command.as_deref(),
                    name.as_deref(),
                    client,
                )?;
                break;
            }
            UiCommand::Send { pane, text } => {
                if let Err(error) = tmux.send_text(&pane, &text, true) {
                    app.status = error.to_string();
                } else {
                    let _ = tmux.set_status(&pane, "working", Some("prompt sent"), 15);
                    app.status = format!("sent prompt to {pane}");
                    app.reload(tmux);
                    app.reload_preview(tmux);
                }
            }
            UiCommand::Mark { pane, kind, name } => {
                match tmux.mark(&pane, &kind, Some(&name), None) {
                    Ok(()) => {
                        app.status = format!("marked {pane} as {name}");
                        app.reload(tmux);
                    }
                    Err(error) => app.status = error.to_string(),
                }
            }
            UiCommand::Unmark(pane) => match tmux.unmark(&pane) {
                Ok(()) => {
                    app.status = format!("cleared agent metadata from {pane}");
                    app.reload(tmux);
                }
                Err(error) => app.status = error.to_string(),
            },
            UiCommand::Kill(pane) => match tmux.kill_pane(&pane) {
                Ok(()) => {
                    app.status = format!("killed {pane}");
                    app.reload(tmux);
                    app.reload_preview(tmux);
                }
                Err(error) => app.status = error.to_string(),
            },
            UiCommand::Respawn(pane) => match tmux.respawn(&pane) {
                Ok(()) => {
                    app.status = format!("respawned {pane}");
                    app.reload(tmux);
                    app.reload_preview(tmux);
                }
                Err(error) => app.status = error.to_string(),
            },
        }
    }
    Ok(())
}

impl App {
    fn new(snapshot: Snapshot, tmux: &Tmux, origin: &str, loaded: &LoadedConfig) -> Result<Self> {
        let selected_pane = snapshot
            .agents()
            .next()
            .or_else(|| snapshot.panes.first())
            .map(|pane| pane.pane_id.clone());
        let mut app = Self {
            snapshot,
            focus: FocusPanel::Agents,
            space_index: 0,
            selected_pane,
            show_plain: loaded.config.ui.show_plain_panes,
            preview: Text::default(),
            status: loaded
                .warning
                .clone()
                .unwrap_or_else(|| format!("config: {}", loaded.path.display())),
            prompt: None,
            confirm: None,
            space_rows: Vec::new(),
            agent_rows: Vec::new(),
            workspace_ratio: loaded.config.ui.workspace_ratio,
            preview_min_width: loaded.config.ui.preview_min_width,
            animation_frame: 0,
        };
        if app.snapshot.pane(origin).is_some() {
            app.selected_pane = Some(origin.to_owned());
        }
        app.ensure_visible_selection();
        app.reload_preview(tmux);
        Ok(app)
    }

    fn visible_pane_indices(&self) -> Vec<usize> {
        let has_agents = self.snapshot.panes.iter().any(PaneRecord::is_agent);
        let mut indices: Vec<_> = self
            .snapshot
            .panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| self.show_plain || !has_agents || pane.is_agent())
            .map(|(index, _)| index)
            .collect();
        if !self.show_plain && has_agents {
            indices.sort_by(|left, right| {
                let left = &self.snapshot.panes[*left];
                let right = &self.snapshot.panes[*right];
                right
                    .state
                    .priority()
                    .cmp(&left.state.priority())
                    .then_with(|| left.session_name.cmp(&right.session_name))
                    .then_with(|| left.window_index.cmp(&right.window_index))
                    .then_with(|| left.pane_index.cmp(&right.pane_index))
            });
        }
        indices
    }

    fn selected_visible_index(&self) -> usize {
        let visible = self.visible_pane_indices();
        self.selected_pane
            .as_ref()
            .and_then(|selected| {
                visible.iter().position(|index| {
                    self.snapshot.panes[*index].pane_id.as_str() == selected.as_str()
                })
            })
            .unwrap_or_default()
    }

    fn selected(&self) -> Option<&PaneRecord> {
        self.selected_pane
            .as_deref()
            .and_then(|id| self.snapshot.pane(id))
    }

    fn selected_target(&self) -> Option<String> {
        match self.focus {
            FocusPanel::Agents => self.selected_pane.clone(),
            FocusPanel::Spaces => self
                .snapshot
                .spaces
                .get(self.space_index)
                .and_then(|space| {
                    space.active_pane_id.clone().or_else(|| {
                        self.snapshot
                            .panes
                            .iter()
                            .find(|pane| pane.session_id == space.session_id)
                            .map(|pane| pane.pane_id.clone())
                    })
                }),
        }
    }

    fn origin_cwd(&self, origin: &str) -> String {
        self.snapshot
            .pane(origin)
            .or_else(|| self.selected())
            .map(|pane| pane.cwd.clone())
            .unwrap_or_else(|| ".".into())
    }

    fn ensure_visible_selection(&mut self) {
        let visible = self.visible_pane_indices();
        let selected_is_visible = self.selected_pane.as_ref().is_some_and(|selected| {
            visible
                .iter()
                .any(|index| self.snapshot.panes[*index].pane_id == *selected)
        });
        if !selected_is_visible {
            self.selected_pane = visible
                .first()
                .map(|index| self.snapshot.panes[*index].pane_id.clone());
        }
        self.space_index = self
            .space_index
            .min(self.snapshot.spaces.len().saturating_sub(1));
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            FocusPanel::Spaces => {
                self.space_index =
                    offset_index(self.space_index, delta, self.snapshot.spaces.len());
            }
            FocusPanel::Agents => {
                let visible = self.visible_pane_indices();
                if visible.is_empty() {
                    return;
                }
                let next = offset_index(self.selected_visible_index(), delta, visible.len());
                self.selected_pane = Some(self.snapshot.panes[visible[next]].pane_id.clone());
            }
        }
    }

    fn reload(&mut self, tmux: &Tmux) {
        match tmux.snapshot() {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.ensure_visible_selection();
                self.status = format!(
                    "{} spaces · {} agents · {} panes",
                    self.snapshot.spaces.len(),
                    self.snapshot.agents().count(),
                    self.snapshot.panes.len()
                );
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn reload_preview(&mut self, tmux: &Tmux) {
        let Some(pane) = self.selected_pane.clone() else {
            self.preview = Text::default();
            return;
        };
        match tmux.preview(&pane, 120) {
            Ok(preview) => {
                self.preview = preview
                    .as_bytes()
                    .into_text()
                    .unwrap_or_else(|_| Text::raw(strip_ansi(&preview)));
            }
            Err(error) => self.preview = Text::raw(error.to_string()),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<UiCommand> {
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }
        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return match key.code {
                KeyCode::Char('c') => Some(spawn("codex")),
                KeyCode::Char('l') => Some(spawn("claude")),
                KeyCode::Char('o') => Some(spawn("opencode")),
                KeyCode::Char('n') => {
                    self.prompt = Some(PromptState {
                        mode: PromptMode::SpawnCommand,
                        input: String::new(),
                    });
                    None
                }
                _ => None,
            };
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('g') => Some(UiCommand::Refresh),
                KeyCode::Char('s') => {
                    let pane = self.selected_pane.clone()?;
                    self.prompt = Some(PromptState {
                        mode: PromptMode::Send { pane },
                        input: String::new(),
                    });
                    None
                }
                KeyCode::Char('r') => {
                    self.confirm = self.selected_pane.clone().map(ConfirmAction::Respawn);
                    None
                }
                KeyCode::Char('k') => {
                    self.confirm = self.selected_pane.clone().map(ConfirmAction::Kill);
                    None
                }
                KeyCode::Char('m') => {
                    let pane = self.selected_pane.clone()?;
                    self.prompt = Some(PromptState {
                        mode: PromptMode::MarkKind { pane },
                        input: String::new(),
                    });
                    None
                }
                KeyCode::Char('u') => self.selected_pane.clone().map(UiCommand::Unmark),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(UiCommand::Quit),
            KeyCode::Enter => self.selected_target().map(UiCommand::Focus),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    FocusPanel::Spaces => FocusPanel::Agents,
                    FocusPanel::Agents => FocusPanel::Spaces,
                };
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.move_selection(isize::MIN);
                None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.move_selection(isize::MAX);
                None
            }
            KeyCode::Char('a') => {
                self.show_plain = !self.show_plain;
                self.ensure_visible_selection();
                self.status = if self.show_plain {
                    "showing agents and ordinary panes".into()
                } else {
                    "showing agents only".into()
                };
                None
            }
            KeyCode::Char('r') => Some(UiCommand::Refresh),
            _ => None,
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Option<UiCommand> {
        match key.code {
            KeyCode::Esc => {
                self.prompt = None;
                None
            }
            KeyCode::Backspace => {
                self.prompt.as_mut()?.input.pop();
                None
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.prompt.as_mut()?.input.push(ch);
                None
            }
            KeyCode::Enter => {
                let prompt = self.prompt.take()?;
                let input = prompt.input.trim().to_owned();
                if input.is_empty() {
                    return None;
                }
                match prompt.mode {
                    PromptMode::Send { pane } => Some(UiCommand::Send { pane, text: input }),
                    PromptMode::SpawnCommand => {
                        self.prompt = Some(PromptState {
                            mode: PromptMode::SpawnName {
                                command: input.clone(),
                            },
                            input: input
                                .split_whitespace()
                                .next()
                                .unwrap_or("agent")
                                .to_owned(),
                        });
                        None
                    }
                    PromptMode::SpawnName { command } => {
                        let kind = detect_agent_command(&command).unwrap_or_else(|| input.clone());
                        Some(UiCommand::Spawn {
                            kind,
                            command: Some(command),
                            name: Some(input),
                        })
                    }
                    PromptMode::MarkKind { pane } => {
                        self.prompt = Some(PromptState {
                            mode: PromptMode::MarkName {
                                pane,
                                kind: input.clone(),
                            },
                            input,
                        });
                        None
                    }
                    PromptMode::MarkName { pane, kind } => Some(UiCommand::Mark {
                        pane,
                        kind,
                        name: input,
                    }),
                }
            }
            _ => None,
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<UiCommand> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                match self.confirm.take()? {
                    ConfirmAction::Kill(pane) => Some(UiCommand::Kill(pane)),
                    ConfirmAction::Respawn(pane) => Some(UiCommand::Respawn(pane)),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm = None;
                None
            }
            _ => None,
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
                true
            }
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, index)) = self
                    .space_rows
                    .iter()
                    .find(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    self.focus = FocusPanel::Spaces;
                    self.space_index = *index;
                    return true;
                }
                if let Some((_, pane)) = self
                    .agent_rows
                    .iter()
                    .find(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    self.focus = FocusPanel::Agents;
                    self.selected_pane = Some(pane.clone());
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        self.animation_frame = self.animation_frame.wrapping_add(1);
        self.space_rows.clear();
        self.agent_rows.clear();
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(2)])
            .split(frame.area());
        let body = if preview_is_visible(outer[0].width, self.preview_min_width) {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
                .split(outer[0]);
            self.render_preview(frame, columns[1]);
            columns[0]
        } else {
            outer[0]
        };

        let workspace_percent = (self.workspace_ratio * 100.0).round() as u16;
        let sidebar = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(workspace_percent),
                Constraint::Percentage(100 - workspace_percent),
            ])
            .split(body);
        self.render_spaces(frame, sidebar[0]);
        self.render_agents(frame, sidebar[1]);
        self.render_footer(frame, outer[1]);
        if self.prompt.is_some() {
            self.render_prompt(frame);
        }
        if self.confirm.is_some() {
            self.render_confirm(frame);
        }
    }

    fn render_spaces(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let active = self.focus == FocusPanel::Spaces;
        let items: Vec<_> = self
            .snapshot
            .spaces
            .iter()
            .map(|space| {
                let state = state_style(space.state);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled(format!("{} ", state_dot(space.state)), state),
                        Span::styled(
                            space.name.clone(),
                            Style::default().fg(SUBTEXT0).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(space.state.label(), state),
                        Span::styled(
                            format!(" · {} agents", space.agent_count),
                            Style::default().fg(OVERLAY0).add_modifier(Modifier::DIM),
                        ),
                    ]),
                    Line::default(),
                ])
            })
            .collect();
        if area.height > 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    " spaces",
                    Style::default()
                        .fg(if active { ACCENT } else { OVERLAY0 })
                        .add_modifier(Modifier::BOLD),
                )),
                Rect::new(area.x, area.y, area.width, 1),
            );
        }
        let inner = Rect::new(
            area.x,
            area.y.saturating_add(2),
            area.width.saturating_sub(1),
            area.height.saturating_sub(2),
        );
        let list = List::new(items)
            .highlight_symbol("▎")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));
        let mut state = ListState::default();
        if !self.snapshot.spaces.is_empty() {
            state.select(Some(self.space_index));
        }
        frame.render_stateful_widget(list, inner, &mut state);
        let offset = state.offset();
        let mut row = inner.y;
        for index in offset..self.snapshot.spaces.len() {
            if row >= inner.y.saturating_add(inner.height) {
                break;
            }
            self.space_rows
                .push((Rect::new(inner.x, row, inner.width, 2), index));
            row = row.saturating_add(2);
        }
    }

    fn render_agents(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let active = self.focus == FocusPanel::Agents;
        let visible = self.visible_pane_indices();
        let selected = self.selected_visible_index();
        let items: Vec<_> = visible
            .iter()
            .map(|index| {
                let pane = &self.snapshot.panes[*index];
                let state = state_style(pane.state);
                let primary =
                    truncate(&format!("{} · {}", pane.session_name, pane.window_name), 34);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled(
                            format!("{} ", state_icon(pane.state, self.animation_frame)),
                            state,
                        ),
                        Span::styled(
                            primary,
                            Style::default().fg(SUBTEXT0).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("   "),
                        Span::styled(pane.state.label(), state),
                        Span::styled(" · ", Style::default().fg(OVERLAY0)),
                        Span::styled(
                            truncate(pane.display_name(), 24),
                            Style::default().fg(OVERLAY0).add_modifier(Modifier::DIM),
                        ),
                        Span::styled(
                            format!(" · {}", pane.command),
                            Style::default().fg(OVERLAY0).add_modifier(Modifier::DIM),
                        ),
                    ]),
                    Line::default(),
                ])
            })
            .collect();
        if area.height > 0 {
            frame.render_widget(
                Paragraph::new("─".repeat(area.width as usize))
                    .style(Style::default().fg(SURFACE_DIM)),
                Rect::new(area.x, area.y, area.width, 1),
            );
        }
        if area.height > 1 {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        " agents",
                        Style::default()
                            .fg(if active { ACCENT } else { OVERLAY0 })
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if self.show_plain {
                            "  all panes"
                        } else {
                            "  priority"
                        },
                        Style::default().fg(OVERLAY0).add_modifier(Modifier::DIM),
                    ),
                ])),
                Rect::new(area.x, area.y + 1, area.width, 1),
            );
        }
        let inner = Rect::new(
            area.x,
            area.y.saturating_add(3),
            area.width.saturating_sub(1),
            area.height.saturating_sub(3),
        );
        let list = List::new(items)
            .highlight_symbol("▎")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));
        let mut state = ListState::default();
        if !visible.is_empty() {
            state.select(Some(selected));
        }
        frame.render_stateful_widget(list, inner, &mut state);

        let offset = state.offset();
        let mut row = inner.y;
        for index in visible.into_iter().skip(offset) {
            if row >= inner.y + inner.height {
                break;
            }
            self.agent_rows.push((
                Rect::new(inner.x, row, inner.width, 3.min(inner.height)),
                self.snapshot.panes[index].pane_id.clone(),
            ));
            row = row.saturating_add(3);
        }
    }

    fn render_preview(&self, frame: &mut Frame<'_>, area: Rect) {
        let name = self
            .selected()
            .map(PaneRecord::display_name)
            .unwrap_or("pane");
        if area.width > 0 {
            let buf = frame.buffer_mut();
            for y in area.y..area.y.saturating_add(area.height) {
                buf[(area.x, y)].set_symbol("│");
                buf[(area.x, y)].set_style(Style::default().fg(SURFACE_DIM));
            }
        }
        let title_area = Rect::new(
            area.x.saturating_add(2),
            area.y,
            area.width.saturating_sub(3),
            1.min(area.height),
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    truncate(name, 32),
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  live · following tail", Style::default().fg(OVERLAY0)),
            ])),
            title_area,
        );
        let inner = Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(2),
            area.width.saturating_sub(3),
            area.height.saturating_sub(2),
        );
        let scroll = preview_tail_scroll(&self.preview, inner.width, inner.height);
        let paragraph = Paragraph::new(self.preview.clone())
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }

    fn render_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        let keys = Line::from(vec![
            Span::styled("Tab", key_style()),
            Span::raw(" panel  "),
            Span::styled("j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("Enter", key_style()),
            Span::raw(" focus  "),
            Span::styled("Alt-c/l/o/n", key_style()),
            Span::raw(" spawn  "),
            Span::styled("Ctrl-s/r/k/m/u", key_style()),
            Span::raw(" act  "),
            Span::styled("a", key_style()),
            Span::raw(" all  "),
            Span::styled("q", key_style()),
            Span::raw(" close"),
        ]);
        let status = Line::from(Span::styled(
            truncate(&self.status, area.width as usize),
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(vec![keys, status]), area);
    }

    fn render_prompt(&self, frame: &mut Frame<'_>) {
        let Some(prompt) = &self.prompt else {
            return;
        };
        let title = match &prompt.mode {
            PromptMode::Send { .. } => " send prompt ",
            PromptMode::SpawnCommand => " custom command ",
            PromptMode::SpawnName { .. } => " agent name ",
            PromptMode::MarkKind { .. } => " agent kind ",
            PromptMode::MarkName { .. } => " agent name ",
        };
        let area = centered_rect(70, 5, frame.area());
        frame.render_widget(Clear, area);
        let input = format!("> {}█", prompt.input);
        frame.render_widget(
            Paragraph::new(input)
                .block(
                    Block::default()
                        .title(title)
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Rgb(38, 198, 218))),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_confirm(&self, frame: &mut Frame<'_>) {
        let Some(confirm) = &self.confirm else {
            return;
        };
        let message = match confirm {
            ConfirmAction::Kill(pane) => format!("Kill {pane}? [y/N]"),
            ConfirmAction::Respawn(pane) => format!("Respawn {pane}? [y/N]"),
        };
        let area = centered_rect(55, 5, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(message).block(
                Block::default()
                    .title(" confirm ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(255, 112, 67))),
            ),
            area,
        );
    }
}

fn spawn(kind: &str) -> UiCommand {
    UiCommand::Spawn {
        kind: kind.into(),
        command: None,
        name: None,
    }
}

fn offset_index(index: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if delta == isize::MIN {
        return 0;
    }
    if delta == isize::MAX {
        return len - 1;
    }
    ((index as isize + delta).rem_euclid(len as isize)) as usize
}

fn state_icon(state: AgentState, frame: usize) -> &'static str {
    match state {
        AgentState::Blocked => "◉",
        AgentState::Done => "●",
        AgentState::Working => {
            const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            SPINNER[(frame / 2) % SPINNER.len()]
        }
        AgentState::Idle => "✓",
        AgentState::Unknown => "○",
        AgentState::Dead => "×",
        AgentState::Plain => "·",
    }
}

fn state_dot(state: AgentState) -> &'static str {
    match state {
        AgentState::Blocked | AgentState::Working | AgentState::Done => "●",
        AgentState::Idle => "○",
        AgentState::Unknown | AgentState::Dead | AgentState::Plain => "·",
    }
}

fn state_style(state: AgentState) -> Style {
    let color = match state {
        AgentState::Blocked => RED,
        AgentState::Done => TEAL,
        AgentState::Working => YELLOW,
        AgentState::Idle => GREEN,
        AgentState::Unknown | AgentState::Dead | AgentState::Plain => OVERLAY0,
    };
    Style::default()
        .fg(color)
        .add_modifier(if state.is_attention() {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

fn key_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

fn preview_is_visible(width: u16, configured_minimum: u16) -> bool {
    width >= configured_minimum
}

fn preview_tail_scroll(text: &Text<'_>, width: u16, height: u16) -> u16 {
    if width == 0 || height == 0 {
        return 0;
    }
    let width = usize::from(width);
    let visual_rows = text.lines.iter().fold(0usize, |rows, line| {
        let line_width = line.width();
        rows + line_width.max(1).div_ceil(width)
    });
    visual_rows
        .saturating_sub(usize::from(height))
        .min(usize::from(u16::MAX)) as u16
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn truncate(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut result = String::new();
    for ch in value.chars() {
        if UnicodeWidthStr::width(result.as_str())
            + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
            > width - 1
        {
            break;
        }
        result.push(ch);
    }
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_wraps() {
        assert_eq!(offset_index(0, -1, 3), 2);
        assert_eq!(offset_index(2, 1, 3), 0);
        assert_eq!(offset_index(1, isize::MIN, 3), 0);
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("日本語abcdef", 7), "日本語…");
    }

    #[test]
    fn preview_scroll_follows_wrapped_tail() {
        assert_eq!(preview_tail_scroll(&Text::raw("one\ntwo\nthree"), 20, 2), 1);
        assert_eq!(preview_tail_scroll(&Text::raw("123456789"), 4, 2), 1);
        assert_eq!(preview_tail_scroll(&Text::raw("short"), 20, 5), 0);
    }

    #[test]
    fn narrow_panes_hide_preview_at_configured_boundary() {
        assert!(!preview_is_visible(119, 120));
        assert!(preview_is_visible(120, 120));
    }

    #[test]
    fn ansi_preview_preserves_color_and_modifiers() {
        let text = b"plain \x1b[31mred\x1b[0m \x1b[38;2;1;2;3;48;5;25;1;4mstyled\x1b[0m"
            .as_slice()
            .into_text()
            .unwrap();
        assert_eq!(text.lines[0].spans[1].style.fg, Some(Color::Red));
        let styled = &text.lines[0].spans[3].style;
        assert_eq!(styled.fg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(styled.bg, Some(Color::Indexed(25)));
        assert!(styled.add_modifier.contains(Modifier::BOLD));
        assert!(styled.add_modifier.contains(Modifier::UNDERLINED));
    }
}
