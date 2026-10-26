use crate::{
    config::LoadedConfig,
    detect::{detect_agent_command, strip_ansi},
    model::{AgentState, PaneRecord, Snapshot},
    project::{ProjectCache, directory_name},
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
    layout::{Constraint, Direction, Layout, Position, Rect},
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

// Solarized accents, while the background stays terminal-native.
const SURFACE_DIM: Color = Color::Rgb(7, 54, 66);
const OVERLAY0: Color = Color::Rgb(88, 110, 117);
const TEXT: Color = Color::Rgb(147, 161, 161);
const SUBTEXT1: Color = Color::Rgb(131, 148, 150);
const SUBTEXT0: Color = Color::Rgb(101, 123, 131);
const ACCENT: Color = Color::Rgb(38, 139, 210);
const GREEN: Color = Color::Rgb(133, 153, 0);
const YELLOW: Color = Color::Rgb(181, 137, 0);
const RED: Color = Color::Rgb(220, 50, 47);
const CYAN: Color = Color::Rgb(42, 161, 152);
const VIOLET: Color = Color::Rgb(108, 113, 196);
const AGENT_ROW_HEIGHT: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPanel {
    Spaces,
    Agents,
    Preview,
}

#[derive(Debug, Clone, Copy)]
enum PreviewMotion {
    Lines(isize),
    Pages(isize),
    HalfPages(isize),
    Top,
    Bottom,
}

#[derive(Debug, Default)]
struct PreviewScroll {
    // None follows live output. Some(top) reads a frozen history snapshot.
    top: Option<usize>,
}

impl PreviewScroll {
    fn offset(&self, height: usize, viewport: u16) -> usize {
        let bottom = height.saturating_sub(usize::from(viewport));
        self.top.unwrap_or(bottom).min(bottom)
    }

    fn move_by(&mut self, motion: PreviewMotion, height: usize, viewport: u16) {
        let bottom = height.saturating_sub(usize::from(viewport));
        let current = self.offset(height, viewport);
        let next = match motion {
            PreviewMotion::Lines(delta) => current.saturating_add_signed(delta),
            PreviewMotion::Pages(delta) => current.saturating_add_signed(
                delta.saturating_mul(viewport.saturating_sub(1).max(1) as isize),
            ),
            PreviewMotion::HalfPages(delta) => {
                current.saturating_add_signed(delta.saturating_mul((viewport / 2).max(1) as isize))
            }
            PreviewMotion::Top => 0,
            PreviewMotion::Bottom => bottom,
        }
        .min(bottom);
        self.top = (next < bottom).then_some(next);
    }
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
    Unmark(String),
}

#[derive(Debug)]
enum UiCommand {
    Quit,
    Refresh,
    ScrollPreview(PreviewMotion),
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
    projects: ProjectCache,
    ui_pane_id: Option<String>,
    focus: FocusPanel,
    space_index: usize,
    selected_pane: Option<String>,
    show_plain: bool,
    preview: Text<'static>,
    preview_width: u16,
    preview_height: usize,
    preview_cursor: Option<(u16, u16)>,
    preview_identity: Option<(String, String)>,
    preview_scroll: PreviewScroll,
    preview_area: Rect,
    status: String,
    prompt: Option<PromptState>,
    confirm: Option<ConfirmAction>,
    space_rows: Vec<(Rect, usize)>,
    agent_rows: Vec<(Rect, String)>,
    workspace_ratio: f32,
    preview_min_width: u16,
    preview_visible: bool,
    animation_frame: usize,
    close_on_focus: bool,
}

pub fn run(
    tmux: Tmux,
    origin: Option<String>,
    client: Option<String>,
    loaded: LoadedConfig,
    popup: bool,
) -> Result<()> {
    // agentmux is a color-semantic TUI: state and captured pane colors carry
    // information, so preserve them even when the parent shell exports NO_COLOR.
    force_color_output(true);
    // `origin` is the pane from which the sidebar was opened.  It is not the
    // pane that now hosts this TUI: split-window/new-window give agentmux a new
    // TMUX_PANE.  Keep both identities so the all-panes view cannot preview the
    // TUI's own captured screen recursively.
    let ui_pane_id = (!popup).then(|| env::var("TMUX_PANE").ok()).flatten();
    let origin = origin
        .or_else(|| env::var("TMUX_PANE").ok())
        .context("agentmux ui must run inside tmux or receive --origin")?;
    let snapshot = tmux.snapshot()?;
    let mut app = App::new(snapshot, &tmux, &origin, ui_pane_id, &loaded)?;
    app.close_on_focus = popup;
    let pane_ids = app
        .snapshot
        .panes
        .iter()
        .map(|pane| pane.pane_id.clone())
        .collect();
    let watcher = match if tmux.observer_socket().is_some() {
        Ok(None)
    } else {
        watcher::start(&tmux, &origin, pane_ids).map(Some)
    } {
        Ok(receiver) => receiver,
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

    loop {
        if tmux.observer_socket().is_some() {
            pending_status = true;
        }
        let mut paths = Vec::new();
        if let Some(pane) = app.selected() {
            paths.push(pane.cwd.clone());
        }
        paths.extend(
            app.visible_pane_indices()
                .iter()
                .map(|index| app.snapshot.panes[*index].cwd.clone()),
        );
        app.projects.refresh(paths);
        terminal.draw(|frame| app.render(frame))?;

        if let Some(watcher) = watcher {
            for _ in 0..256 {
                match watcher.try_recv() {
                    Ok(WatcherEvent::Output(_)) => {
                        pending_status = true;
                    }
                    Ok(WatcherEvent::Topology) => pending_status = true,
                    Ok(WatcherEvent::Error(error)) => {
                        app.status = format!("event watcher stopped: {error}");
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
        }

        // Published previews (for example LazyAgent's live ACP transcript) can
        // change without producing bytes on the enclosing tmux pane. Poll the
        // selected preview at the configured cadence so hidden/scratch buffers
        // keep following the actual conversation.
        if last_preview.elapsed() >= preview_interval {
            app.reload_preview(tmux);
            last_preview = Instant::now();
        }
        if status_refresh_due(pending_status, last_status.elapsed(), status_interval) {
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
            Event::Mouse(mouse) => app.handle_mouse(mouse),
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
            UiCommand::ScrollPreview(motion) => app.scroll_preview(tmux, motion),
            UiCommand::Refresh => {
                app.reload(tmux);
                app.reload_preview(tmux);
                last_status = Instant::now();
            }
            UiCommand::Focus(pane) => match tmux.focus_pane(&pane, client) {
                Ok(()) => {
                    if app.close_on_focus {
                        break;
                    }
                    app.status = format!("Focused {pane}");
                }
                Err(error) => app.status = error.to_string(),
            },
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
                let send = (|| -> Result<()> {
                    let target = tmux.resolve_pane(&pane)?;
                    let info = crate::control::capabilities(tmux, &target)?;
                    crate::control::execute(
                        tmux,
                        crate::control::Request {
                            pane: pane.clone(),
                            binding: info.binding,
                            action: crate::control::Action::Send,
                            text,
                        },
                    )
                })();
                if let Err(error) = send {
                    app.status = error.to_string();
                } else {
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
        if let Some(path) = tmux.observer_socket() {
            if let Err(error) = crate::observer::refresh(path) {
                app.status = error.to_string();
            }
        }
    }
    Ok(())
}

impl App {
    fn new(
        snapshot: Snapshot,
        tmux: &Tmux,
        origin: &str,
        ui_pane_id: Option<String>,
        loaded: &LoadedConfig,
    ) -> Result<Self> {
        let selected_pane = snapshot
            .agents()
            .next()
            .or_else(|| snapshot.panes.first())
            .map(|pane| pane.pane_id.clone());
        let mut app = Self {
            snapshot,
            projects: ProjectCache::new(),
            ui_pane_id,
            focus: FocusPanel::Agents,
            space_index: 0,
            selected_pane,
            show_plain: loaded.config.ui.show_plain_panes,
            preview: Text::default(),
            preview_width: 0,
            preview_height: 0,
            preview_cursor: None,
            preview_identity: None,
            preview_scroll: PreviewScroll::default(),
            preview_area: Rect::default(),
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
            preview_visible: false,
            animation_frame: 0,
            close_on_focus: false,
        };
        if app.snapshot.pane(origin).is_some() {
            app.selected_pane = Some(origin.to_owned());
        }
        app.ensure_visible_selection();
        app.reload_preview(tmux);
        Ok(app)
    }

    fn visible_pane_indices(&self) -> Vec<usize> {
        let mut indices: Vec<_> = self
            .snapshot
            .panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| {
                pane_is_visible(
                    &pane.pane_id,
                    pane.is_agent(),
                    self.ui_pane_id.as_deref(),
                    self.show_plain,
                )
            })
            .map(|(index, _)| index)
            .collect();
        if !self.show_plain {
            indices.sort_by(|left, right| {
                let left = &self.snapshot.panes[*left];
                let right = &self.snapshot.panes[*right];
                right
                    .state
                    .priority()
                    .cmp(&left.state.priority())
                    .then_with(|| right.state_seq.cmp(&left.state_seq))
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
            FocusPanel::Agents | FocusPanel::Preview => self.selected_pane.clone(),
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
        let panes: Vec<_> = visible
            .iter()
            .map(|index| self.snapshot.panes[*index].clone())
            .collect();
        self.snapshot.spaces = Snapshot::build_spaces(&panes);
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
            FocusPanel::Preview => {}
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
        let identity = self
            .selected()
            .map(|pane| (pane.pane_id.clone(), pane.instance_id.clone()));
        if self.preview_identity != identity {
            self.preview_identity = identity;
            self.preview_scroll.top = None;
        }
        if !self.preview_visible {
            return;
        }
        // New output must not move text underneath someone reading history.
        if self.preview_scroll.top.is_some() {
            return;
        }
        let Some(pane) = self.selected_pane.clone() else {
            self.preview = Text::default();
            self.preview_width = 0;
            self.preview_height = 0;
            self.preview_cursor = None;
            return;
        };
        match tmux.pane_view(&pane) {
            Ok(view) => {
                self.preview = view
                    .ansi
                    .as_bytes()
                    .into_text()
                    .unwrap_or_else(|_| Text::raw(strip_ansi(&view.ansi)));
                self.preview_width = view.width;
                self.preview_height = usize::from(view.height);
                self.preview_cursor = Some((view.cursor_x, view.cursor_y));
            }
            Err(error) => {
                self.preview = Text::raw(error.to_string());
                self.preview_width = 0;
                self.preview_height = 0;
                self.preview_cursor = None;
            }
        }
    }

    fn scroll_preview(&mut self, tmux: &Tmux, motion: PreviewMotion) {
        if !self.preview_visible || self.preview_area.height <= 2 {
            return;
        }
        self.focus = FocusPanel::Preview;
        self.reload_preview(tmux);
        if matches!(motion, PreviewMotion::Bottom) {
            self.preview_scroll.top = None;
            self.reload_preview(tmux);
            return;
        }
        if self.preview_scroll.top.is_none() {
            if matches!(
                motion,
                PreviewMotion::Lines(0..)
                    | PreviewMotion::Pages(0..)
                    | PreviewMotion::HalfPages(0..)
            ) {
                return;
            }
            let Some(pane) = self.selected_pane.as_deref() else {
                return;
            };
            match tmux.pane_history(pane) {
                Ok(ansi) => {
                    self.preview = ansi
                        .as_bytes()
                        .into_text()
                        .unwrap_or_else(|_| Text::raw(strip_ansi(&ansi)));
                    self.preview_height = self.preview.lines.len();
                    self.preview_cursor = None;
                }
                Err(error) => {
                    self.status = format!("history unavailable: {error}");
                    return;
                }
            }
        }
        self.preview_scroll.move_by(
            motion,
            self.preview_height,
            self.preview_area.height.saturating_sub(2),
        );
        if self.preview_scroll.top.is_none() {
            self.reload_preview(tmux);
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
                KeyCode::Char('u') if self.focus == FocusPanel::Preview => {
                    Some(UiCommand::ScrollPreview(PreviewMotion::HalfPages(-1)))
                }
                KeyCode::Char('d') if self.focus == FocusPanel::Preview => {
                    Some(UiCommand::ScrollPreview(PreviewMotion::HalfPages(1)))
                }
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
                KeyCode::Char('u') => {
                    self.confirm = self.selected_pane.clone().map(ConfirmAction::Unmark);
                    None
                }
                _ => None,
            };
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(UiCommand::Quit),
            KeyCode::Enter => self.selected_target().map(UiCommand::Focus),
            KeyCode::PageUp => Some(UiCommand::ScrollPreview(PreviewMotion::Pages(-1))),
            KeyCode::PageDown => Some(UiCommand::ScrollPreview(PreviewMotion::Pages(1))),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match (self.focus, key.code == KeyCode::BackTab) {
                    (FocusPanel::Spaces, false) | (FocusPanel::Preview, true) => FocusPanel::Agents,
                    (FocusPanel::Agents, false) | (FocusPanel::Spaces, true)
                        if self.preview_visible =>
                    {
                        FocusPanel::Preview
                    }
                    (FocusPanel::Spaces, true) => FocusPanel::Agents,
                    _ => FocusPanel::Spaces,
                };
                None
            }
            KeyCode::Down | KeyCode::Char('j') if self.focus == FocusPanel::Preview => {
                Some(UiCommand::ScrollPreview(PreviewMotion::Lines(1)))
            }
            KeyCode::Up | KeyCode::Char('k') if self.focus == FocusPanel::Preview => {
                Some(UiCommand::ScrollPreview(PreviewMotion::Lines(-1)))
            }
            KeyCode::Home | KeyCode::Char('g') if self.focus == FocusPanel::Preview => {
                Some(UiCommand::ScrollPreview(PreviewMotion::Top))
            }
            KeyCode::End | KeyCode::Char('G') if self.focus == FocusPanel::Preview => {
                Some(UiCommand::ScrollPreview(PreviewMotion::Bottom))
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
        // Registration removal defaults to cancel, including a bare Enter.
        if key.code == KeyCode::Enter && matches!(self.confirm, Some(ConfirmAction::Unmark(_))) {
            self.confirm = None;
            return None;
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                match self.confirm.take()? {
                    ConfirmAction::Kill(pane) => Some(UiCommand::Kill(pane)),
                    ConfirmAction::Respawn(pane) => Some(UiCommand::Respawn(pane)),
                    ConfirmAction::Unmark(pane) => Some(UiCommand::Unmark(pane)),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm = None;
                None
            }
            _ => None,
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> Option<UiCommand> {
        if self.prompt.is_some() || self.confirm.is_some() {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                if self.preview_visible && contains(self.preview_area, mouse.column, mouse.row) {
                    return Some(UiCommand::ScrollPreview(PreviewMotion::Lines(delta * 3)));
                }
                if self
                    .space_rows
                    .iter()
                    .any(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    self.focus = FocusPanel::Spaces;
                } else if self
                    .agent_rows
                    .iter()
                    .any(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    self.focus = FocusPanel::Agents;
                }
                self.move_selection(delta);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, index)) = self
                    .space_rows
                    .iter()
                    .find(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    self.focus = FocusPanel::Spaces;
                    self.space_index = *index;
                    return None;
                }
                if let Some((_, pane)) = self
                    .agent_rows
                    .iter()
                    .find(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    self.focus = FocusPanel::Agents;
                    self.selected_pane = Some(pane.clone());
                    return None;
                }
                if self.preview_visible && contains(self.preview_area, mouse.column, mouse.row) {
                    self.focus = FocusPanel::Preview;
                }
                None
            }
            _ => None,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        self.animation_frame = self.animation_frame.wrapping_add(1);
        self.space_rows.clear();
        self.agent_rows.clear();
        let details = self.selected_details(usize::from(frame.area().width));
        let detail_height = u16::from(self.selected().is_some());
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),
                Constraint::Length(detail_height),
                Constraint::Length(2),
            ])
            .split(frame.area());
        self.preview_visible = preview_is_visible(outer[0].width, self.preview_min_width);
        self.preview_area = Rect::default();
        if !self.preview_visible && self.focus == FocusPanel::Preview {
            self.focus = FocusPanel::Agents;
        }
        let body = if self.preview_visible {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
                .split(outer[0]);
            self.preview_area = columns[1];
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
        frame.render_widget(
            Paragraph::new(details).style(Style::default().fg(SUBTEXT0)),
            outer[1],
        );
        self.render_footer(frame, outer[2]);
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
                            Style::default().fg(SUBTEXT1),
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
                        .fg(if active { ACCENT } else { SUBTEXT0 })
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
                let project = self.projects.get(&pane.cwd);
                let root = project
                    .map(|project| project.root.as_str())
                    .unwrap_or(&pane.cwd);
                let name = clean_label(directory_name(root));
                let name = if name.is_empty() {
                    clean_label(&pane.session_name)
                } else {
                    name
                };
                let available = usize::from(area.width.saturating_sub(4));
                let mut primary =
                    project_label(&name, project.and_then(|p| p.branch.as_deref()), available);
                primary.spans.splice(
                    0..0,
                    [
                        Span::raw(" "),
                        Span::styled(
                            format!("{} ", state_icon(pane.state, self.animation_frame)),
                            state,
                        ),
                    ],
                );
                let mut secondary = agent_label(pane, available);
                secondary.spans.insert(0, Span::raw("   "));
                ListItem::new(vec![primary, secondary, Line::default()])
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
                            .fg(if active { ACCENT } else { SUBTEXT0 })
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if self.show_plain {
                            "  all panes"
                        } else {
                            "  priority"
                        },
                        Style::default().fg(SUBTEXT1),
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
                Rect::new(
                    inner.x,
                    row,
                    inner.width,
                    AGENT_ROW_HEIGHT.min(inner.y.saturating_add(inner.height).saturating_sub(row)),
                ),
                self.snapshot.panes[index].pane_id.clone(),
            ));
            row = row.saturating_add(AGENT_ROW_HEIGHT);
        }
    }

    fn selected_details(&self, width: usize) -> String {
        let Some(pane) = self.selected() else {
            return String::new();
        };
        let root = self
            .projects
            .get(&pane.cwd)
            .map(|project| project.root.as_str())
            .unwrap_or("—");
        let root = clean_label(root);
        let cwd = clean_label(&pane.cwd);
        let location = format!(
            "{} · {} · {} · {}",
            clean_label(&pane.location()),
            clean_label(&pane.window_name),
            pane.pane_id,
            pane.state.label()
        );
        // Labels and separators take 25 cells. Share the rest, returning any
        // unused path budget to its neighbor before abbreviating from the left.
        let available = width.saturating_sub(25);
        let location_width = UnicodeWidthStr::width(location.as_str()).min(available / 3);
        let path_width = available - location_width;
        let root_width = UnicodeWidthStr::width(root.as_str()).min(path_width / 2);
        let cwd_width = UnicodeWidthStr::width(cwd.as_str()).min(path_width - root_width);
        let root_width = path_width - cwd_width;
        truncate(
            &format!(
                " root  {} · cwd   {} · tmux  {}",
                compact_path(&root, root_width),
                compact_path(&cwd, cwd_width),
                truncate(&location, location_width)
            ),
            width,
        )
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
        let viewport_height = area.height.saturating_sub(2);
        let scroll = self
            .preview_scroll
            .offset(self.preview_height, viewport_height);
        let mode = if self.preview_scroll.top.is_some() {
            format!(
                "  history · {}-{}/{} · End: live",
                scroll + 1,
                (scroll + usize::from(viewport_height)).min(self.preview_height),
                self.preview_height
            )
        } else {
            "  live · auto-follow".into()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    truncate(name, 32),
                    Style::default()
                        .fg(if self.focus == FocusPanel::Preview {
                            ACCENT
                        } else {
                            TEXT
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(mode, Style::default().fg(SUBTEXT1)),
            ])),
            title_area,
        );
        let inner = Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(2),
            area.width.saturating_sub(3),
            area.height.saturating_sub(2),
        );
        // Slice before rendering: Paragraph's scroll offset is u16, while tmux
        // and ACP histories can contain more than 65,535 lines.
        let visible: Vec<_> = self
            .preview
            .lines
            .iter()
            .skip(scroll)
            .take(usize::from(inner.height))
            .cloned()
            .collect();
        let paragraph = Paragraph::new(Text::from(visible));
        frame.render_widget(paragraph, inner);
        if self.preview_scroll.top.is_none() && self.prompt.is_none() && self.confirm.is_none() {
            if let Some((cursor_x, cursor_y)) = self.preview_cursor {
                let cursor_y = usize::from(cursor_y);
                if cursor_x < self.preview_width.min(inner.width)
                    && cursor_y >= scroll
                    && cursor_y - scroll < usize::from(inner.height)
                {
                    frame.set_cursor_position(Position::new(
                        inner.x.saturating_add(cursor_x),
                        inner.y.saturating_add((cursor_y - scroll) as u16),
                    ));
                }
            }
        }
    }

    fn render_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        let keys = Line::from(vec![
            Span::styled("Tab", key_style()),
            Span::raw(" panel  "),
            Span::styled("j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("Enter", key_style()),
            Span::raw(" focus  "),
            Span::styled(
                if self.focus == FocusPanel::Preview {
                    "PgUp/Dn Ctrl-u/d"
                } else {
                    "PgUp/Dn"
                },
                key_style(),
            ),
            Span::raw(" scroll  "),
            Span::styled("Alt-c/l/o/n", key_style()),
            Span::raw(" spawn  "),
            Span::styled(
                if self.focus == FocusPanel::Preview {
                    "Ctrl-s/r/k/m"
                } else {
                    "Ctrl-s/r/k/m/u"
                },
                key_style(),
            ),
            Span::raw(" act  "),
            Span::styled("a", key_style()),
            Span::raw(" all  "),
            Span::styled("q", key_style()),
            Span::raw(" close"),
        ]);
        let status = Line::from(Span::styled(
            truncate(&self.status, area.width as usize),
            Style::default().fg(SUBTEXT0),
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
                        .border_style(Style::default().fg(ACCENT)),
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
            ConfirmAction::Unmark(pane) => format!("Remove agent registration for {pane}? [y/N]"),
        };
        let area = centered_rect(55, 5, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(message).block(
                Block::default()
                    .title(" confirm ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(YELLOW)),
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
        AgentState::Done => CYAN,
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

// Pane-option reports, TTL expiry, exited owners and a disconnected watcher
// need reconciliation even when the terminal emits no output.
fn status_refresh_due(pending: bool, elapsed: Duration, interval: Duration) -> bool {
    elapsed >= interval && (pending || elapsed >= Duration::from_secs(2))
}

fn preview_is_visible(width: u16, configured_minimum: u16) -> bool {
    width >= configured_minimum
}

fn pane_is_visible(
    pane_id: &str,
    is_agent: bool,
    ui_pane_id: Option<&str>,
    show_plain: bool,
) -> bool {
    ui_pane_id != Some(pane_id) && (show_plain || is_agent)
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

fn clean_label(value: &str) -> String {
    strip_ansi(value)
        .chars()
        .filter(|ch| !ch.is_control() || ch.is_whitespace())
        .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn compact_path(path: &str, width: usize) -> String {
    if UnicodeWidthStr::width(path) <= width {
        return path.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut used = 1;
    let mut suffix = Vec::new();
    for ch in path.chars().rev() {
        let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cells > width {
            break;
        }
        suffix.push(ch);
        used += cells;
    }
    let suffix: String = suffix.into_iter().rev().collect();
    let suffix = suffix
        .find('/')
        .map(|start| &suffix[start..])
        .unwrap_or(&suffix);
    format!("…{suffix}")
}

fn context_label(
    name: &str,
    extra: Option<&str>,
    width: usize,
    name_style: Style,
    extra_style: Style,
) -> Line<'static> {
    let Some(extra) = extra.map(clean_label).filter(|extra| !extra.is_empty()) else {
        return Line::from(Span::styled(truncate(name, width), name_style));
    };
    if width < 9 {
        return Line::from(Span::styled(truncate(name, width), name_style));
    }
    // Keep both identifiers visible when one of them is unusually long.
    let name_width = UnicodeWidthStr::width(name).min((width - 3) / 2);
    let extra_width = UnicodeWidthStr::width(extra.as_str()).min(width - 3 - name_width);
    let name_width = width - 3 - extra_width;
    Line::from(vec![
        Span::styled(truncate(name, name_width), name_style),
        Span::styled(" · ", Style::default().fg(OVERLAY0)),
        Span::styled(truncate(&extra, extra_width), extra_style),
    ])
}

fn project_label(name: &str, branch: Option<&str>, width: usize) -> Line<'static> {
    context_label(
        name,
        branch,
        width,
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        Style::default().fg(VIOLET),
    )
}

fn agent_label(pane: &PaneRecord, width: usize) -> Line<'static> {
    let name = clean_label(pane.display_name());
    context_label(
        &name,
        task_label(pane).as_deref(),
        width,
        Style::default().fg(ACCENT),
        Style::default().fg(TEXT),
    )
}

fn task_label(pane: &PaneRecord) -> Option<String> {
    let reported = pane
        .state_source
        .starts_with("native:")
        .then_some(pane.state_message.as_deref())
        .flatten();
    let command = pane.command.to_ascii_lowercase();
    let editor = command.ends_with("vim")
        || command.contains("nvim")
        || matches!(command.as_str(), "vi" | "view" | "emacs" | "emacsclient");
    let generic = [
        "idle",
        "working",
        "blocked",
        "done",
        "unknown",
        "ready",
        "codex",
        "claude",
        "claude code",
        "bash",
        "zsh",
        "fish",
        "sh",
        "nvim",
        "vim",
    ];
    static HOSTNAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let hostname = HOSTNAME.get_or_init(|| {
        env::var("HOSTNAME")
            .or_else(|_| env::var("HOST"))
            .unwrap_or_else(|_| std::fs::read_to_string("/etc/hostname").unwrap_or_default())
            .trim()
            .to_owned()
    });
    // Editor titles describe buffers, not the ACP conversation they host.
    let titles = (!editor).then_some(pane.title.as_str());
    for value in reported.into_iter().chain(titles) {
        let value = clean_label(value);
        let value = value.trim_start_matches(|ch: char| {
            matches!(
                ch,
                '\u{2800}'
                    ..='\u{28ff}'
                        | '✳'
                        | '✻'
                        | '✽'
                        | '✶'
                        | '✢'
                        | '●'
                        | '◉'
                        | '✓'
                        | '✔'
                        | '⏺'
                        | '⏵'
            ) || ch.is_whitespace()
        });
        let duplicate = [
            pane.display_name(),
            pane.command.as_str(),
            pane.cwd.as_str(),
            directory_name(&pane.cwd),
            hostname.as_str(),
            hostname.split('.').next().unwrap_or_default(),
        ]
        .iter()
        .any(|name| value.eq_ignore_ascii_case(name));
        if !value.is_empty()
            && !duplicate
            && !generic.iter().any(|name| value.eq_ignore_ascii_case(name))
        {
            return Some(value.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_refresh_reconciles_silent_updates_and_limits_bursts() {
        let interval = Duration::from_millis(250);
        assert!(!status_refresh_due(
            true,
            Duration::from_millis(249),
            interval
        ));
        assert!(status_refresh_due(true, interval, interval));
        assert!(!status_refresh_due(false, interval, interval));
        assert!(status_refresh_due(false, Duration::from_secs(2), interval));
        assert!(!status_refresh_due(
            false,
            Duration::from_secs(2),
            Duration::from_secs(3)
        ));
    }

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
    fn context_labels_keep_branch_and_unicode_within_available_width() {
        for width in 0..50 {
            let label = project_label(
                "日本語-project-with-a-long-name",
                Some("feature/日本語-sidebar"),
                width,
            )
            .to_string();
            assert!(UnicodeWidthStr::width(label.as_str()) <= width);
            if width >= 9 {
                assert!(label.contains(" · "));
            }
        }
        assert_eq!(
            project_label("agentmux", Some("main"), 30).to_string(),
            "agentmux · main"
        );
        assert_eq!(project_label("agentmux", None, 30).to_string(), "agentmux");
    }

    #[test]
    fn task_labels_prefer_fresh_reports_and_exclude_editor_and_generic_titles() {
        let mut pane = PaneRecord {
            agent_kind: Some("codex".into()),
            command: "codex".into(),
            cwd: "/tmp/project".into(),
            title: "⠋ サイドバーの表示改善".into(),
            ..Default::default()
        };
        assert_eq!(task_label(&pane).as_deref(), Some("サイドバーの表示改善"));
        pane.state_message = Some("認証のレビュー".into());
        pane.state_source = "native:working".into();
        assert_eq!(task_label(&pane).as_deref(), Some("認証のレビュー"));
        pane.state_message = Some("working".into());
        assert_eq!(task_label(&pane).as_deref(), Some("サイドバーの表示改善"));
        pane.state_source = "editor:no native status".into();
        pane.command = "nvim".into();
        assert_eq!(task_label(&pane), None);
        pane.command = "codex".into();
        for title in [
            "codex",
            "project",
            "/tmp/project",
            "✓ Ready",
            "\x1b[31mworking\x1b[0m",
        ] {
            pane.title = title.into();
            assert_eq!(task_label(&pane), None, "{title}");
        }
        pane.title = "\x1b[31m修正\x1b[0m\nテスト\x07".into();
        assert_eq!(task_label(&pane).as_deref(), Some("修正 テスト"));
    }

    #[test]
    fn agent_only_never_falls_back_to_plain_panes() {
        assert!(!pane_is_visible("%8", false, None, false));
        assert!(pane_is_visible("%8", false, None, true));
    }

    #[test]
    fn preview_crops_visible_screen_from_the_bottom() {
        let scroll = PreviewScroll::default();
        assert_eq!(scroll.offset(40, 24), 16);
        assert_eq!(scroll.offset(24, 40), 0);
        assert_eq!(scroll.offset(100, 24), 76);
    }

    #[test]
    fn history_pages_overlap_and_resume_follow_at_bottom() {
        let mut scroll = PreviewScroll::default();
        scroll.move_by(PreviewMotion::Pages(-1), 100, 20);
        assert_eq!(scroll.top, Some(61));
        scroll.move_by(PreviewMotion::Lines(-3), 100, 20);
        assert_eq!(scroll.top, Some(58));
        scroll.move_by(PreviewMotion::Pages(1), 100, 20);
        assert_eq!(scroll.top, Some(77));
        scroll.move_by(PreviewMotion::Lines(3), 100, 20);
        assert_eq!(scroll.top, None);
        assert_eq!(scroll.offset(130, 20), 110);
    }

    #[test]
    fn history_half_pages_preserve_overlap_and_return_to_live() {
        let mut scroll = PreviewScroll::default();
        scroll.move_by(PreviewMotion::HalfPages(-1), 100, 21);
        assert_eq!(scroll.top, Some(69));
        scroll.move_by(PreviewMotion::HalfPages(1), 100, 21);
        assert_eq!(scroll.top, None);
        scroll.move_by(PreviewMotion::HalfPages(-1), 100, 1);
        assert_eq!(scroll.top, Some(98));
    }

    #[test]
    fn history_handles_large_scrollback_and_viewport_resizes() {
        let mut scroll = PreviewScroll::default();
        scroll.move_by(PreviewMotion::Lines(-3), 100_000, 20);
        assert_eq!(scroll.top, Some(99_977));
        assert_eq!(scroll.offset(100_000, 10), 99_977);
        assert_eq!(scroll.offset(100_000, 30), 99_970);
        scroll.move_by(PreviewMotion::Top, 100_000, 30);
        assert_eq!(scroll.top, Some(0));
        scroll.move_by(PreviewMotion::Lines(isize::MIN), 100_000, 30);
        assert_eq!(scroll.top, Some(0));
        scroll.move_by(PreviewMotion::Bottom, 100_000, 30);
        assert_eq!(scroll.top, None);
        scroll.move_by(PreviewMotion::Top, 3, 30);
        assert_eq!(scroll.top, None);
    }

    #[test]
    fn narrow_panes_hide_preview_at_configured_boundary() {
        assert!(!preview_is_visible(119, 120));
        assert!(preview_is_visible(120, 120));
    }

    #[test]
    fn all_panes_excludes_the_agentmux_ui_pane() {
        assert!(!pane_is_visible("%9", false, Some("%9"), true));
        assert!(pane_is_visible("%8", false, Some("%9"), true));
        assert!(pane_is_visible("%8", true, Some("%9"), false));
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
