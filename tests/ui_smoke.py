"""Verify Enter changes focus without terminating the dashboard (isolated tmux)."""
import os
import pathlib
import pty
import subprocess
import time

binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/agentmux'
sock = f'agentmux-ui-test-{os.getpid()}'
def tmux(*args):
    return subprocess.check_output(['tmux', '-L', sock, *args], text=True).strip()
child = None
master = None
try:
    tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'test', 'sleep 60')
    origin = tmux('list-panes', '-F', '#{pane_id}')
    subprocess.run([str(binary), '--socket-name', sock, 'mark', origin, '--kind', 'codex', '--name', 'test-agent'], check=True)
    ui = tmux('new-window', '-P', '-F', '#{pane_id}', '-n', 'dashboard', f'{binary} --socket-name {sock} ui {origin}')
    child, master = pty.fork()
    if child == 0:
        os.environ['TERM'] = 'xterm-256color'
        os.execvp('tmux', ['tmux', '-L', sock, 'attach-session', '-t', 'test'])
    for _ in range(50):
        screen = tmux('capture-pane', '-p', '-t', ui)
        if 'agents' in screen:
            break
        time.sleep(.1)
    else:
        raise AssertionError('dashboard did not render')
    tmux('send-keys', '-t', ui, 'Enter')
    for _ in range(30):
        if tmux('display-message', '-p', '#{pane_id}') == origin:
            break
        time.sleep(.1)
    else:
        raise AssertionError('Enter did not focus target')
    assert ui in tmux('list-panes', '-a', '-F', '#{pane_id}').splitlines(), 'dashboard exited on focus'
    tmux('select-window', '-t', ui)
    tmux('send-keys', '-t', ui, 'q')
    for _ in range(30):
        if ui not in tmux('list-panes', '-a', '-F', '#{pane_id}').splitlines():
            break
        time.sleep(.1)
    else:
        raise AssertionError('dashboard no longer responds to q')
    print('Dashboard: Enter focuses target, UI stays alive, q still exits')
finally:
    subprocess.run(['tmux', '-L', sock, 'kill-server'], capture_output=True)
    if master is not None:
        os.close(master)
    if child:
        os.waitpid(child, 0)
