// SPDX-License-Identifier: AGPL-3.0-or-later
//
// This state detection engine is adapted from Herdr v0.7.3's manifest
// detector. See NOTICE for the exact upstream revision and attribution.

use crate::model::AgentState;
use regex::Regex;
use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

use super::DetectedStatus;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    #[allow(dead_code)]
    version: Option<String>,
    min_engine_version: Option<u32>,
    #[serde(rename = "updated_at")]
    #[allow(dead_code)]
    updated_at: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    rules: Vec<Rule>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Rule {
    id: String,
    state: ManifestState,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_region")]
    region: String,
    #[allow(dead_code)]
    #[serde(default)]
    visible_idle: bool,
    #[allow(dead_code)]
    #[serde(default)]
    visible_blocker: bool,
    #[allow(dead_code)]
    #[serde(default)]
    visible_working: bool,
    #[serde(default)]
    skip_state_update: bool,
    #[serde(default)]
    all: Vec<Gate>,
    #[serde(default)]
    any: Vec<Gate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<Gate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
struct Gate {
    #[serde(default)]
    all: Vec<Gate>,
    #[serde(default)]
    any: Vec<Gate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<Gate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ManifestState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl From<ManifestState> for AgentState {
    fn from(value: ManifestState) -> Self {
        match value {
            ManifestState::Idle => Self::Idle,
            ManifestState::Working => Self::Working,
            ManifestState::Blocked => Self::Blocked,
            ManifestState::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug)]
struct CompiledManifest {
    id: String,
    aliases: Vec<String>,
    rules: Vec<CompiledRule>,
}

#[derive(Debug)]
struct CompiledRule {
    rule: Rule,
    gate: CompiledGate,
}

#[derive(Debug, Default)]
struct CompiledGate {
    all: Vec<CompiledGate>,
    any: Vec<CompiledGate>,
    not_gate: Vec<CompiledGate>,
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
}

const BUNDLED: &[(&str, &str)] = &[
    ("amp", include_str!("manifests/amp.toml")),
    ("agy", include_str!("manifests/antigravity.toml")),
    ("claude", include_str!("manifests/claude.toml")),
    ("cline", include_str!("manifests/cline.toml")),
    ("codex", include_str!("manifests/codex.toml")),
    ("cursor", include_str!("manifests/cursor.toml")),
    ("devin", include_str!("manifests/devin.toml")),
    ("droid", include_str!("manifests/droid.toml")),
    ("gemini", include_str!("manifests/gemini.toml")),
    ("copilot", include_str!("manifests/github-copilot.toml")),
    ("grok", include_str!("manifests/grok.toml")),
    ("hermes", include_str!("manifests/hermes.toml")),
    ("kilo", include_str!("manifests/kilo.toml")),
    ("kimi", include_str!("manifests/kimi.toml")),
    ("kiro", include_str!("manifests/kiro.toml")),
    ("opencode", include_str!("manifests/opencode.toml")),
    ("maki", include_str!("manifests/maki.toml")),
    ("muse", include_str!("manifests/muse.toml")),
    ("qwen", include_str!("manifests/qwen.toml")),
    ("pi", include_str!("manifests/pi.toml")),
    ("qodercli", include_str!("manifests/qodercli.toml")),
];

static MANIFESTS: OnceLock<HashMap<String, CompiledManifest>> = OnceLock::new();

pub(super) fn canonical_kind(label: &str) -> Option<&'static str> {
    let lower = label.trim().to_ascii_lowercase();
    manifests().iter().find_map(|(id, manifest)| {
        (manifest.id == lower || manifest.aliases.iter().any(|alias| alias == &lower))
            .then_some(id.as_str())
    })
}

pub(super) fn detect(kind: &str, screen: &str, osc_title: &str) -> Option<DetectedStatus> {
    let canonical = canonical_kind(kind)?;
    let manifest = manifests().get(canonical)?;
    let mut best: Option<&CompiledRule> = None;

    for rule in &manifest.rules {
        let text = region(screen, osc_title, &rule.rule.region);
        if gate_matches(&rule.gate, text)
            && best.is_none_or(|previous| previous.rule.priority < rule.rule.priority)
        {
            best = Some(rule);
        }
    }

    match best {
        Some(rule) if rule.rule.skip_state_update => Some(DetectedStatus {
            state: AgentState::Unknown,
            source: format!("manifest:{}:{}:skip", manifest.id, rule.rule.id),
        }),
        Some(rule) => Some(DetectedStatus {
            state: rule.rule.state.into(),
            source: format!("manifest:{}:{}", manifest.id, rule.rule.id),
        }),
        None => Some(DetectedStatus {
            state: AgentState::Idle,
            source: format!("manifest:{}:idle-fallback", manifest.id),
        }),
    }
}

fn manifests() -> &'static HashMap<String, CompiledManifest> {
    MANIFESTS.get_or_init(|| {
        BUNDLED
            .iter()
            .map(|(id, source)| {
                let parsed: Manifest = toml::from_str(source)
                    .unwrap_or_else(|error| panic!("invalid bundled {id} manifest: {error}"));
                assert!(
                    parsed.min_engine_version.unwrap_or(1) <= 3,
                    "unsupported engine version in {id}"
                );
                let rules = parsed
                    .rules
                    .into_iter()
                    .map(|rule| {
                        let gate = compile_gate(Gate {
                            all: rule.all.clone(),
                            any: rule.any.clone(),
                            not_gate: rule.not_gate.clone(),
                            contains: rule.contains.clone(),
                            regex: rule.regex.clone(),
                            line_regex: rule.line_regex.clone(),
                        })
                        .unwrap_or_else(|error| panic!("invalid {id} rule {}: {error}", rule.id));
                        CompiledRule { rule, gate }
                    })
                    .collect();
                (
                    (*id).to_owned(),
                    CompiledManifest {
                        id: parsed.id,
                        aliases: parsed.aliases,
                        rules,
                    },
                )
            })
            .collect()
    })
}

fn compile_gate(gate: Gate) -> Result<CompiledGate, regex::Error> {
    Ok(CompiledGate {
        all: gate
            .all
            .into_iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        any: gate
            .any
            .into_iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        not_gate: gate
            .not_gate
            .into_iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        contains: gate
            .contains
            .into_iter()
            .map(|value| value.to_lowercase())
            .collect(),
        regex: gate
            .regex
            .into_iter()
            .map(|pattern| Regex::new(&pattern))
            .collect::<Result<_, _>>()?,
        line_regex: gate
            .line_regex
            .into_iter()
            .map(|pattern| Regex::new(&pattern))
            .collect::<Result<_, _>>()?,
    })
}

fn gate_matches(gate: &CompiledGate, text: &str) -> bool {
    gate_matches_lower(gate, text, &text.to_lowercase())
}

fn gate_matches_lower(gate: &CompiledGate, text: &str, lower: &str) -> bool {
    gate.contains.iter().all(|needle| lower.contains(needle))
        && gate.regex.iter().all(|regex| regex.is_match(text))
        && gate
            .line_regex
            .iter()
            .all(|regex| text.lines().any(|line| regex.is_match(line)))
        && gate
            .all
            .iter()
            .all(|nested| gate_matches_lower(nested, text, lower))
        && (gate.any.is_empty()
            || gate
                .any
                .iter()
                .any(|nested| gate_matches_lower(nested, text, lower)))
        && !gate
            .not_gate
            .iter()
            .any(|nested| gate_matches_lower(nested, text, lower))
}

fn default_region() -> String {
    "whole_recent".to_owned()
}

fn region<'a>(screen: &'a str, osc_title: &'a str, spec: &str) -> &'a str {
    match spec.trim() {
        "osc_title" => osc_title,
        "osc_progress" => "",
        "whole_recent" => screen,
        "after_last_prompt_marker" => after_last_prompt_marker(screen),
        "before_current_prompt_marker" => before_current_prompt_marker(screen),
        "whole_recent_without_current_prompt_marker" => {
            if current_codex_prompt_index(&screen.lines().collect::<Vec<_>>()).is_some() {
                ""
            } else {
                screen
            }
        }
        "current_prompt_block_marker" => current_prompt_block_marker(screen).unwrap_or(""),
        "after_current_prompt_block_marker" => {
            after_current_prompt_block_marker(screen).unwrap_or("")
        }
        "prompt_box_body" => prompt_box_body(screen).unwrap_or(""),
        "above_prompt_box" => above_prompt_box(screen),
        "last_non_empty_above_prompt_box" => last_non_empty_line(above_prompt_box(screen)),
        "after_last_horizontal_rule" => after_last_horizontal_rule(screen),
        value => region_count(value, "bottom_lines")
            .map(|count| bottom_lines(screen, count))
            .or_else(|| {
                region_count(value, "bottom_non_empty_lines")
                    .map(|count| bottom_non_empty_lines(screen, count))
            })
            .or_else(|| {
                region_count(value, "top_non_empty_lines")
                    .map(|count| top_non_empty_lines(screen, count))
            })
            .unwrap_or(""),
    }
}

fn region_count(spec: &str, name: &str) -> Option<usize> {
    spec.strip_prefix(name)?
        .strip_prefix('(')?
        .strip_suffix(')')?
        .parse()
        .ok()
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let mut remaining = count;
    let mut end = 0;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        if remaining == 0 {
            break;
        }
        offset += line.len();
        if !line.trim().is_empty() {
            end = offset;
            remaining -= 1;
        }
    }
    &content[..end]
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines: Vec<_> = content.lines().collect();
    slice_from_line(content, &lines, lines.len().saturating_sub(count))
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<_> = content.lines().collect();
    let start = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index);
    start.map_or("", |index| slice_from_line(content, &lines, index))
}

fn after_last_prompt_marker(content: &str) -> &str {
    let lines: Vec<_> = content.lines().collect();
    lines
        .iter()
        .rposition(|line| codex_prompt_line(line))
        .map_or(content, |index| slice_from_line(content, &lines, index + 1))
}

fn before_current_prompt_marker(content: &str) -> &str {
    let lines: Vec<_> = content.lines().collect();
    current_codex_prompt_index(&lines).map_or(content, |index| {
        &content[..line_offset(content, &lines, index)]
    })
}

fn current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<_> = content.lines().collect();
    let prompt = current_codex_prompt_index(&lines)?;
    lines[..prompt]
        .iter()
        .rev()
        .find(|line| codex_block_marker_line(line))
        .copied()
}

fn after_current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<_> = content.lines().collect();
    let prompt = current_codex_prompt_index(&lines)?;
    let block = lines[..prompt]
        .iter()
        .rposition(|line| codex_block_marker_line(line))?;
    Some(slice_from_line(content, &lines, block))
}

fn current_codex_prompt_index(lines: &[&str]) -> Option<usize> {
    let prompt = lines.iter().rposition(|line| codex_prompt_line(line))?;
    (!lines[prompt + 1..]
        .iter()
        .any(|line| codex_block_marker_line(line)))
    .then_some(prompt)
}

fn codex_prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn codex_block_marker_line(line: &str) -> bool {
    line.starts_with(['•', '■', '✗', '✓'])
}

fn prompt_box_body(content: &str) -> Option<&str> {
    let lines: Vec<_> = content.lines().collect();
    let top = prompt_box_top(&lines)?;
    let end = lines[top + 1..]
        .iter()
        .position(|line| is_horizontal_rule(line))
        .map_or(lines.len(), |relative| top + 1 + relative);
    Some(&content[line_offset(content, &lines, top + 1)..line_offset(content, &lines, end)])
}

fn above_prompt_box(content: &str) -> &str {
    let lines: Vec<_> = content.lines().collect();
    prompt_box_top(&lines).map_or(content, |top| &content[..line_offset(content, &lines, top)])
}

fn prompt_box_top(lines: &[&str]) -> Option<usize> {
    let mut seen = 0;
    for index in (0..lines.len()).rev() {
        if is_horizontal_rule(lines[index]) {
            seen += 1;
            if seen == 2 {
                return Some(index);
            }
        }
    }
    None
}

fn after_last_horizontal_rule(content: &str) -> &str {
    let lines: Vec<_> = content.lines().collect();
    lines
        .iter()
        .rposition(|line| is_horizontal_rule(line))
        .map_or(content, |index| slice_from_line(content, &lines, index + 1))
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let count = trimmed
        .chars()
        .take_while(|character| *character == '─')
        .count();
    count > 0 && (count >= 3 || trimmed.chars().all(|character| character == '─'))
}

fn last_non_empty_line(content: &str) -> &str {
    content
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
}

fn slice_from_line<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    &content[line_offset(content, lines, index)..]
}

fn line_offset(content: &str, lines: &[&str], index: usize) -> usize {
    lines.get(index).map_or(content.len(), |line| {
        line.as_ptr() as usize - content.as_ptr() as usize
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_upstream_ui_states() {
        let fixtures = [
            ("claude", "", "◐ project", AgentState::Working),
            (
                "claude",
                "MCP server \"tools\" requests your input\n❯ Accept\nEsc to cancel",
                "",
                AgentState::Blocked,
            ),
            (
                "claude",
                "✻ Waiting for 2 background agents to finish\n────\n❯\n────",
                "",
                AgentState::Working,
            ),
            (
                "copilot",
                "◎ Waiting for background agents · 10s",
                "",
                AgentState::Working,
            ),
            (
                "codex",
                "Update available!\n1. Update now\n2. Skip until next version\nPress enter to continue",
                "",
                AgentState::Blocked,
            ),
            (
                "codex",
                "> You are in /tmp/project\nDo you trust the contents of this directory?",
                "",
                AgentState::Blocked,
            ),
            (
                "codex",
                "• Working (2s · esc to interrupt)",
                "project",
                AgentState::Working,
            ),
            ("codex", "› Explain the text [y/n]", "", AgentState::Idle),
            ("grok", "· 2 │ project", "grok", AgentState::Working),
            ("amp", "", "Plugin confirmation needed", AgentState::Blocked),
            ("hermes", "", "⏳ project", AgentState::Working),
            ("maki", " ⠋ [BUILD] project", "", AgentState::Working),
            ("maki", " [PLAN] project", "", AgentState::Idle),
            (
                "muse-cli",
                "Enter to select · Tab for an optional note · Esc to interrupt",
                "",
                AgentState::Blocked,
            ),
            (
                "muse",
                "◆ Working (2s · esc to interrupt)",
                "",
                AgentState::Working,
            ),
            ("muse", "⟩", "", AgentState::Idle),
            (
                "qwen-code",
                "⠏ ユーザーの確認を待っています...",
                "",
                AgentState::Blocked,
            ),
            ("qwen", "", "◐ project", AgentState::Working),
        ];
        for (kind, screen, title, expected) in fixtures {
            assert_eq!(
                detect(kind, screen, title).unwrap().state,
                expected,
                "{kind}: {screen}"
            );
        }
    }

    #[test]
    fn regions_preserve_crlf_and_unicode_boundaries() {
        let screen = "日本語\r\n\r\n次の行\r\n末尾";
        assert_eq!(region(screen, "", "bottom_lines(2)"), "次の行\r\n末尾");
        assert_eq!(
            region(screen, "", "top_non_empty_lines(2)"),
            "日本語\r\n\r\n次の行\r\n"
        );
        assert_eq!(region(screen, "", "top_non_empty_lines(0)"), "");
        assert_eq!(region(" \n\n", "", "top_non_empty_lines(2)"), "");
        assert_eq!(region("a\n\n", "", "top_non_empty_lines(2)"), "a\n");
        assert_eq!(region(screen, "", "bottom_lines(0)"), "");
        assert_eq!(
            region("same\nsame\nlast", "", "top_non_empty_lines(2)"),
            "same\nsame\n"
        );
    }

    #[test]
    fn every_bundled_manifest_compiles() {
        assert_eq!(manifests().len(), BUNDLED.len());
        assert!(
            manifests()
                .values()
                .all(|manifest| !manifest.rules.is_empty())
        );
    }

    #[test]
    fn copilot_cancel_hint_is_working() {
        let result = detect("github-copilot", "Thinking\nEsc again to cancel", "").unwrap();
        assert_eq!(result.state, AgentState::Working);
        assert_eq!(result.source, "manifest:copilot:working_cancel_hint");
    }

    #[test]
    fn devin_footer_states_are_distinguished() {
        let idle = detect("devin", "────────\n❭\n────────\nContext: 2k", "").unwrap();
        assert_eq!(idle.state, AgentState::Idle);
        let blocked = detect(
            "devin",
            "❭ 1 Yes (Approve once)\n↑↓ select · ↵ confirm · esc cancel",
            "",
        )
        .unwrap();
        assert_eq!(blocked.state, AgentState::Blocked);
    }

    #[test]
    fn osc_title_rules_are_used() {
        assert_eq!(
            detect("codex", "", "⠋ project").unwrap().state,
            AgentState::Working
        );
        assert_eq!(
            detect("codex", "", "[ . ] Action Required | project")
                .unwrap()
                .state,
            AgentState::Blocked
        );
    }

    #[test]
    fn codex_ignores_stale_shell_confirmation_before_current_prompt() {
        let screen = ":: Proceed with installation? [Y/n]\npackages installed\n\n› Improve documentation\n\n  gpt-5.6-sol · ~/project\n";
        let detected = detect("codex", screen, "project").unwrap();
        assert_eq!(detected.state, AgentState::Idle);
        assert_ne!(detected.source, "manifest:codex:weak_blocker");

        let live = detect(
            "codex",
            "› run tests\n• Running command\nDo you want to continue? [y/n]",
            "",
        )
        .unwrap();
        assert_eq!(live.state, AgentState::Blocked);
        assert_eq!(live.source, "manifest:codex:weak_blocker");
    }

    #[test]
    fn aliases_resolve_to_canonical_ids() {
        assert_eq!(canonical_kind("ghcs"), Some("copilot"));
        assert_eq!(canonical_kind("antigravity-cli"), Some("agy"));
        assert_eq!(canonical_kind("open-code"), Some("opencode"));
    }

    #[test]
    fn every_agent_manifest_recognizes_representative_live_ui() {
        let fixtures = [
            (
                "amp",
                "Waiting for approval: run this command?",
                AgentState::Blocked,
            ),
            (
                "agy",
                "Requesting permission for:\nDo you want to proceed?",
                AgentState::Blocked,
            ),
            (
                "claude",
                "Run a dynamic workflow? Esc to cancel",
                AgentState::Blocked,
            ),
            ("cline", "Let Cline use this tool", AgentState::Blocked),
            ("codex", "›\nAllow command?", AgentState::Blocked),
            (
                "cursor",
                "Write to this file? Proceed (y)\nReject & propose changes",
                AgentState::Blocked,
            ),
            (
                "devin",
                "Running tools · 5s (esc to interrupt)\nGuide Devin while it works",
                AgentState::Working,
            ),
            (
                "droid",
                "> Yes, allow\nEnter to select · ↑↓ to navigate · Esc to cancel",
                AgentState::Blocked,
            ),
            ("gemini", "│ Apply this change", AgentState::Blocked),
            (
                "copilot",
                "Thinking\nEsc again to cancel",
                AgentState::Working,
            ),
            ("grok", "┃  2 (○) Yes, proceed", AgentState::Blocked),
            (
                "hermes",
                "Dangerous command\nEnter to confirm",
                AgentState::Blocked,
            ),
            ("kilo", "△ Permission required", AgentState::Blocked),
            ("kimi", "🌕", AgentState::Working),
            ("kiro", "Kiro is working", AgentState::Working),
            ("opencode", "△ Permission required", AgentState::Blocked),
            ("pi", "Working...", AgentState::Working),
            ("qodercli", "Permission required", AgentState::Blocked),
        ];

        for (kind, screen, expected) in fixtures {
            let detected = detect(kind, screen, "").unwrap();
            assert_eq!(detected.state, expected, "fixture failed for {kind}");
            assert!(detected.source.starts_with(&format!("manifest:{kind}:")));
        }
    }
}
