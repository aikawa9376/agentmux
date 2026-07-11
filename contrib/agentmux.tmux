# Clear stale bindings first: source-file applies incrementally and does not remove
# bindings that disappeared from a config file.
unbind-key -n M-a
unbind-key a
unbind-key A

# Open a Herdr-style sidebar as a regular tmux pane. q exits agentmux and the
# sidebar pane closes naturally without popup teardown.
bind-key -n M-a split-window -h -l 48% -c "#{pane_current_path}" \
  "agentmux ui '#{pane_id}' --client '#{client_name}'"

# Full-window dashboard, also backed by an ordinary tmux pane.
bind-key a new-window -n agentmux -c "#{pane_current_path}" \
  "agentmux ui '#{pane_id}' --client '#{client_name}'"
