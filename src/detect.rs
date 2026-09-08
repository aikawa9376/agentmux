use crate::model::AgentState;
use regex::Regex;
use std::sync::OnceLock;

mod manifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedStatus {
    pub state: AgentState,
    pub source: String,
}

pub fn normalize_status(value: &str) -> Option<AgentState> {
    match value.to_ascii_lowercase().as_str() {
        "thinking" | "working" | "running" | "busy" | "active" | "starting" => {
            Some(AgentState::Working)
        }
        "waiting" | "blocked" | "permission" | "approval" | "input" => Some(AgentState::Blocked),
        "completed" | "complete" | "done" | "ready" => Some(AgentState::Done),
        "idle" | "stopped" => Some(AgentState::Idle),
        "dead" | "exited" => Some(AgentState::Dead),
        "unknown" => Some(AgentState::Unknown),
        _ => None,
    }
}

pub fn detect_agent_command(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    if lower.contains("cursor agent") || has_token(&lower, "cursor-agent") {
        return Some("cursor".into());
    }

    const AGENTS: &[(&str, &str)] = &[
        ("muse-code", "muse"),
        ("muse-cli", "muse"),
        ("qwen-code", "qwen"),
        ("maki", "maki"),
        ("muse", "muse"),
        ("qwen", "qwen"),
        ("github-copilot", "copilot"),
        ("antigravity-cli", "agy"),
        ("cursor-agent", "cursor"),
        ("qoderclicn", "qodercli"),
        ("qodercli", "qodercli"),
        ("claude-code", "claude"),
        ("devin-cli", "devin"),
        ("grok-build", "grok"),
        ("hermes-agent", "hermes"),
        ("kilo-code", "kilo"),
        ("kimi-code", "kimi"),
        ("kiro-cli", "kiro"),
        ("mastra-code", "mastracode"),
        ("open-code", "opencode"),
        ("amp-local", "amp"),
        ("antigravity", "agy"),
        ("mastracode", "mastracode"),
        ("opencode", "opencode"),
        ("copilot", "copilot"),
        ("ghcs", "copilot"),
        ("claude", "claude"),
        ("codex", "codex"),
        ("devin", "devin"),
        ("droid", "droid"),
        ("qodercn", "qodercli"),
        ("qoder", "qodercli"),
        ("kimi", "kimi"),
        ("kilo", "kilo"),
        ("hermes", "hermes"),
        ("gemini", "gemini"),
        ("grok", "grok"),
        ("kiro", "kiro"),
        ("cline", "cline"),
        ("aiagent", "aiagent"),
        ("amp", "amp"),
        ("agy", "agy"),
        ("omp", "omp"),
        ("pi", "pi"),
    ];

    AGENTS
        .iter()
        .find_map(|(token, label)| has_token(&lower, token).then(|| (*label).to_owned()))
}

fn has_token(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let end = start + needle.len();
        let after = haystack[end..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

pub fn screen_status(kind: &str, screen: &str) -> DetectedStatus {
    screen_status_with_title(kind, screen, "")
}

pub fn screen_status_with_title(kind: &str, screen: &str, pane_title: &str) -> DetectedStatus {
    if let Some(detected) = manifest::detect(kind, screen, pane_title) {
        return detected;
    }

    let active = ACTIVE_RE.get_or_init(|| {
        Regex::new(
            r"(?i)(esc to interrupt|ctrl-c to interrupt|press esc to interrupt|working \([0-9]+[smh]|thinking(?:\.\.\.|…)|running tool|executing tool)",
        )
        .expect("valid active regex")
    });
    if active.is_match(screen) {
        return DetectedStatus {
            state: AgentState::Working,
            source: "ui:active indicator".into(),
        };
    }

    let blocked = BLOCKED_RE.get_or_init(|| {
        Regex::new(
            r"(?i)(do you want to (?:proceed|continue|allow)|would you like to|approve (?:this|the)|requesting permission|waiting for (?:your )?(?:input|approval)|press enter to confirm|choose (?:an|one) option|confirm folder trust|do you trust the files|enter to select|\[[yY]/[nN]\]|\([yY]/[nN]\))",
        )
        .expect("valid blocked regex")
    });
    if blocked.is_match(screen) {
        return DetectedStatus {
            state: AgentState::Blocked,
            source: "ui:input requested".into(),
        };
    }

    let prompt_ready = screen.lines().rev().take(8).any(|line| {
        let trimmed = line.trim_start();
        match kind {
            "codex" | "copilot" => trimmed.starts_with('›') || trimmed.starts_with('❯'),
            "claude" => trimmed.starts_with('>'),
            _ => false,
        }
    });
    if prompt_ready {
        return DetectedStatus {
            state: AgentState::Idle,
            source: "ui:prompt ready".into(),
        };
    }

    DetectedStatus {
        state: AgentState::Idle,
        source: "generic:alive, no active indicator".into(),
    }
}

pub fn strip_ansi(value: &str) -> String {
    let csi = CSI_RE.get_or_init(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").unwrap());
    let osc = OSC_RE.get_or_init(|| Regex::new(r"\x1b\][^\x07]*(?:\x07|\x1b\\)").unwrap());
    osc.replace_all(&csi.replace_all(value, ""), "")
        .into_owned()
}

pub fn recent_meaningful(screen: &str, lines: usize) -> String {
    let meaningful: Vec<_> = screen
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    meaningful[meaningful.len().saturating_sub(lines)..].join("\n")
}

pub fn last_line(screen: &str) -> String {
    screen
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| one_line(line, 180))
        .unwrap_or_default()
}

pub fn one_line(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.chars().take(max_chars).collect()
}

static ACTIVE_RE: OnceLock<Regex> = OnceLock::new();
static BLOCKED_RE: OnceLock<Regex> = OnceLock::new();
static CSI_RE: OnceLock<Regex> = OnceLock::new();
static OSC_RE: OnceLock<Regex> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_names_match_on_boundaries() {
        assert_eq!(
            detect_agent_command("node /bin/codex"),
            Some("codex".into())
        );
        assert_eq!(
            detect_agent_command("cursor-agent chat"),
            Some("cursor".into())
        );
        for (command, kind) in [
            ("maki", "maki"),
            ("muse-cli", "muse"),
            ("muse-code", "muse"),
            ("node /bin/qwen", "qwen"),
            ("qwen-code", "qwen"),
        ] {
            assert_eq!(detect_agent_command(command).as_deref(), Some(kind));
        }
        assert_eq!(detect_agent_command("amusement"), None);
        assert_eq!(detect_agent_command("polybar example"), None);
        assert_eq!(detect_agent_command("vampire"), None);
        assert_eq!(detect_agent_command("ghcs"), Some("copilot".into()));
        assert_eq!(detect_agent_command("antigravity-cli"), Some("agy".into()));
        assert_eq!(detect_agent_command("qoderclicn"), Some("qodercli".into()));
    }

    #[test]
    fn blocked_requires_a_known_prompt_shape() {
        let status = screen_status("codex", "Do you want to proceed? [y/N]");
        assert_eq!(status.state, AgentState::Blocked);
        let status = screen_status("unknown-agent", "documentation mentions blocked states\n› ");
        assert_eq!(status.state, AgentState::Idle);
    }

    #[test]
    fn active_indicator_wins() {
        let status = screen_status("unknown-agent", "Thinking…\nEsc to interrupt");
        assert_eq!(status.state, AgentState::Working);
    }

    #[test]
    fn copilot_trust_dialog_is_blocked() {
        let screen = "Confirm folder trust\nDo you trust the files in this folder?\n❯ 1. Yes\nenter to select";
        let status = screen_status("copilot", screen);
        assert_eq!(status.state, AgentState::Blocked);
    }

    #[test]
    fn copilot_prompt_is_idle() {
        let status = screen_status("copilot", "~/project [main]\n❯\n/ commands · ? help");
        assert_eq!(status.state, AgentState::Idle);
        assert_eq!(status.source, "manifest:copilot:idle-fallback");
    }

    #[test]
    fn normalizes_native_states() {
        assert_eq!(normalize_status("thinking"), Some(AgentState::Working));
        assert_eq!(normalize_status("waiting"), Some(AgentState::Blocked));
        assert_eq!(normalize_status("complete"), Some(AgentState::Done));
    }
}
