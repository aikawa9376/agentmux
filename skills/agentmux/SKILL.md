---
name: agentmux
description: Orchestrate coding agents and commands in sibling tmux panes using tmux-native layout/process operations plus agentmux identity and state. Use when running inside tmux to delegate work, spawn another agent, run tests or servers concurrently, inspect pane output, send prompts, or wait for output or agent state.
---

# agentmux orchestration

Use tmux as the pane/session/window authority. Use agentmux for agent discovery, semantic state, safe text submission, and target resolution. Do not recreate tmux layout operations through agentmux.

## Guard and discover

Before controlling panes, require all of these checks to pass:

```sh
test -n "${TMUX:-}" && test -n "${TMUX_PANE:-}"
command -v tmux >/dev/null
command -v agentmux >/dev/null
tmux display-message -p -t "$TMUX_PANE" '#{pane_id}'
```

Stop if a check fails. Treat `$TMUX_PANE` as the current agent's pane. Never close, respawn, replace, or send input to it unless the user explicitly asks.

Discover current panes and semantic agent state:

```sh
agentmux list --json
tmux list-panes -a -F '#{pane_id}\t#{session_name}:#{window_index}.#{pane_index}\t#{pane_current_command}\t#{pane_current_path}'
```

Re-read pane IDs before destructive actions. tmux pane IDs last for the pane lifetime but disappear when the pane closes.

## Read a pane

Read the visible screen:

```sh
tmux capture-pane -p -t %12
```

Read recent unwrapped output or preserve ANSI colors:

```sh
tmux capture-pane -pJ -S -200 -t %12
tmux capture-pane -epJ -S -200 -t %12
```

Use `agentmux explain %12 --json` when identity or state looks wrong.

## Split and run without stealing focus

Resolve the target cwd first, then let tmux create the sibling:

```sh
TARGET=${TMUX_PANE}
CWD=$(tmux display-message -p -t "$TARGET" '#{pane_current_path}')
NEW_PANE=$(tmux split-window -d -h -t "$TARGET" -c "$CWD" -P -F '#{pane_id}' 'codex')
```

Use `-h` for a left/right split and `-v` for an above/below split. Keep `-d` so the human's active pane is not changed. Capture the returned ID; never guess it.

For a server, test, or log process, replace the final command:

```sh
SERVER_PANE=$(tmux split-window -d -v -t "$TMUX_PANE" -c "$CWD" -P -F '#{pane_id}' 'npm run dev')
```

For an agent command that process detection cannot identify, mark it explicitly:

```sh
agentmux mark "$NEW_PANE" --kind custom-agent --name reviewer
```

## Send a task

Prefer agentmux for natural-language prompts. It sends literal text followed by Enter and publishes a short working state:

```sh
agentmux send "$NEW_PANE" --text 'Review src/api and report concrete correctness issues.'
```

Use tmux only for individual control keys or non-agent shell input:

```sh
tmux send-keys -t "$NEW_PANE" C-c
tmux send-keys -t "$NEW_PANE" -l -- 'literal shell input'
tmux send-keys -t "$NEW_PANE" Enter
```

Never interpolate untrusted text into a tmux shell command. Use `agentmux send` or `tmux send-keys -l` for text.

## Wait

Locate this skill's helper at `${CODEX_HOME:-$HOME/.codex}/skills/agentmux/scripts/wait.py`. If installed elsewhere, resolve `scripts/wait.py` relative to this `SKILL.md`.

Inspect current output before waiting for future output:

```sh
python "$HOME/.codex/skills/agentmux/scripts/wait.py" output "$SERVER_PANE" \
  --match 'ready on port 3000' --timeout 30000
```

Use regex when necessary:

```sh
python "$HOME/.codex/skills/agentmux/scripts/wait.py" output "$SERVER_PANE" \
  --match 'test result: (ok|FAILED)' --regex --timeout 120000
```

Wait for semantic state by pane ID, unique agent name, location, or unique kind:

```sh
python "$HOME/.codex/skills/agentmux/scripts/wait.py" status "$NEW_PANE" \
  --status idle --timeout 120000
```

`done` currently requires a native integration that publishes done. Screen-only agents usually settle at `idle`, so prefer `idle` unless `agentmux list --json` shows native done support.

## Coordinate another agent

Use this sequence:

1. List panes and save the current `$TMUX_PANE`.
2. Split with `-d`, capture the returned pane ID, and start the agent.
3. Read its current screen until its prompt is ready.
4. Send one bounded task with `agentmux send`.
5. Wait for `idle`, `blocked`, or expected output.
6. Read recent output and integrate or verify the result yourself.
7. Keep the pane for follow-up unless the user asked for cleanup.

If the sibling becomes `blocked`, read its output and report the requested permission or input. Do not approve risky actions on the user's behalf.

## Sessions and windows

Use native tmux commands and avoid switching the attached client unless requested:

```sh
tmux new-session -d -s review -c "$CWD"
tmux new-window -d -t review -n tests -c "$CWD"
tmux list-sessions -F '#{session_id}\t#{session_name}'
tmux list-windows -a -F '#{session_name}:#{window_index}\t#{window_name}'
```

Do not kill sessions, windows, or panes you did not create without explicit user authorization.
