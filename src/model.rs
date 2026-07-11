use serde::Serialize;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Blocked,
    Done,
    Working,
    Idle,
    Unknown,
    Dead,
    Plain,
}

impl AgentState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
            Self::Dead => "dead",
            Self::Plain => "pane",
        }
    }

    pub fn priority(self) -> u8 {
        match self {
            Self::Blocked => 6,
            Self::Done => 5,
            Self::Working => 4,
            Self::Idle => 3,
            Self::Unknown => 2,
            Self::Dead => 1,
            Self::Plain => 0,
        }
    }

    pub fn is_attention(self) -> bool {
        matches!(self, Self::Blocked | Self::Done)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PaneRecord {
    pub session_id: String,
    pub session_name: String,
    pub window_id: String,
    pub window_index: u32,
    pub window_name: String,
    pub pane_id: String,
    pub pane_index: u32,
    pub command: String,
    pub cwd: String,
    pub title: String,
    pub tty: String,
    pub dead: bool,
    pub window_active: bool,
    pub pane_active: bool,
    pub agent_kind: Option<String>,
    pub agent_name: Option<String>,
    pub agent_command: Option<String>,
    pub identity_source: Option<String>,
    pub process_agent: Option<String>,
    pub screen_checked: bool,
    pub state: AgentState,
    pub state_source: String,
    pub state_message: Option<String>,
    pub last_line: String,
}

impl PaneRecord {
    pub fn is_agent(&self) -> bool {
        self.agent_kind.is_some()
    }

    pub fn display_name(&self) -> &str {
        self.agent_name
            .as_deref()
            .or(self.agent_kind.as_deref())
            .unwrap_or("pane")
    }

    pub fn location(&self) -> String {
        format!(
            "{}:{}.{}",
            self.session_name, self.window_index, self.pane_index
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceSummary {
    pub session_id: String,
    pub name: String,
    pub state: AgentState,
    pub agent_count: usize,
    pub pane_count: usize,
    pub active_pane_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub spaces: Vec<SpaceSummary>,
    pub panes: Vec<PaneRecord>,
}

impl Snapshot {
    pub fn agents(&self) -> impl Iterator<Item = &PaneRecord> {
        self.panes.iter().filter(|pane| pane.is_agent())
    }

    pub fn pane(&self, id: &str) -> Option<&PaneRecord> {
        self.panes.iter().find(|pane| pane.pane_id == id)
    }

    pub fn build_spaces(panes: &[PaneRecord]) -> Vec<SpaceSummary> {
        let mut spaces = Vec::<SpaceSummary>::new();
        for pane in panes {
            let index = spaces
                .iter()
                .position(|space| space.session_id == pane.session_id)
                .unwrap_or_else(|| {
                    spaces.push(SpaceSummary {
                        session_id: pane.session_id.clone(),
                        name: pane.session_name.clone(),
                        state: AgentState::Plain,
                        agent_count: 0,
                        pane_count: 0,
                        active_pane_id: None,
                    });
                    spaces.len() - 1
                });
            let space = &mut spaces[index];
            space.pane_count += 1;
            if pane.window_active && pane.pane_active {
                space.active_pane_id = Some(pane.pane_id.clone());
            }
            if pane.is_agent() {
                space.agent_count += 1;
                if pane.state.priority() > space.state.priority() {
                    space.state = pane.state;
                }
            }
        }
        spaces.sort_by(|left, right| natural_cmp(&left.name, &right.name));
        spaces
    }

    pub fn sort_panes(panes: &mut [PaneRecord]) {
        panes.sort_by(|left, right| {
            right
                .is_agent()
                .cmp(&left.is_agent())
                .then_with(|| natural_cmp(&left.session_name, &right.session_name))
                .then_with(|| left.window_index.cmp(&right.window_index))
                .then_with(|| left.pane_index.cmp(&right.pane_index))
        });
    }
}

fn natural_cmp(left: &str, right: &str) -> Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(state: AgentState, kind: Option<&str>) -> PaneRecord {
        PaneRecord {
            session_id: "$1".into(),
            session_name: "project".into(),
            window_id: "@1".into(),
            window_index: 1,
            window_name: "main".into(),
            pane_id: "%1".into(),
            pane_index: 1,
            command: "zsh".into(),
            cwd: "/tmp".into(),
            title: String::new(),
            tty: "/dev/pts/1".into(),
            dead: false,
            window_active: true,
            pane_active: true,
            agent_kind: kind.map(str::to_owned),
            agent_name: None,
            agent_command: None,
            identity_source: kind.map(|_| "test".into()),
            process_agent: None,
            screen_checked: kind.is_some(),
            state,
            state_source: "test".into(),
            state_message: None,
            last_line: String::new(),
        }
    }

    #[test]
    fn blocked_rolls_up_above_done_and_working() {
        let panes = vec![
            pane(AgentState::Working, Some("codex")),
            pane(AgentState::Done, Some("claude")),
            pane(AgentState::Blocked, Some("opencode")),
        ];
        let spaces = Snapshot::build_spaces(&panes);
        assert_eq!(spaces[0].state, AgentState::Blocked);
        assert_eq!(spaces[0].agent_count, 3);
    }
}
