"""Verify project/task context, refresh, and two-line selection in a real tmux TUI."""
import fcntl
import os
import pathlib
import pty
import select
import shlex
import struct
import subprocess
import tempfile
import termios
import time

binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/agentmux'
sock = f'agentmux-sidebar-test-{os.getpid()}'
child = master = None


def tmux(*args):
    return subprocess.check_output(['tmux', '-L', sock, *args], text=True).rstrip('\n')


def cli(*args):
    return subprocess.check_output([str(binary), '--socket-name', sock, *args], text=True)


def wait_for(predicate, message, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if master is not None:
            while select.select([master], [], [], 0)[0]:
                os.read(master, 65536)
        result = predicate()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError(message)


with tempfile.TemporaryDirectory(prefix='agentmux-sidebar-') as directory:
    try:
        root = pathlib.Path(directory) / 'project-one'
        nested = root / 'nested'
        nested.mkdir(parents=True)
        subprocess.run(['git', 'init', '--quiet', '--initial-branch=feature/sidebar', str(root)], check=True)
        tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'fixture', '-x', '160', '-y', '44',
             '-c', str(nested), '-n', 'one', 'sleep 120')
        pane = tmux('list-panes', '-F', '#{pane_id}')
        cli('mark', pane, '--kind', 'codex', '--name', 'Codex')
        tmux('select-pane', '-t', pane, '-T', '⠋ サイドバーの表示改善')
        second_root = pathlib.Path(directory) / 'project-two'
        second_root.mkdir()
        subprocess.run(['git', 'init', '--quiet', '--initial-branch=review', str(second_root)], check=True)
        second = tmux('new-window', '-d', '-P', '-F', '#{pane_id}', '-c', str(second_root), '-n', 'two', 'sleep 120')
        cli('mark', second, '--kind', 'claude', '--name', 'Claude')
        ui = tmux('new-window', '-P', '-F', '#{pane_id}', '-n', 'dashboard',
                  f'{shlex.quote(str(binary))} --socket-name {sock} ui {pane}')
        child, master = pty.fork()
        if child == 0:
            os.environ['TERM'] = 'xterm-256color'
            os.execvp('tmux', ['tmux', '-L', sock, 'attach-session', '-t', 'fixture'])
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 44, 160, 0, 0))

        def screen():
            return tmux('capture-pane', '-p', '-t', ui)

        wait_for(lambda: 'project-one · feature/sidebar' in screen(), 'repo root/branch did not appear')
        rendered = screen()
        assert 'project-two · review' in rendered, rendered
        assert 'Codex · サイドバーの表示改善' in rendered, rendered
        assert f'root  {root}' in rendered, rendered
        assert f'cwd   {nested}' in rendered, rendered
        assert 'tmux  fixture:0.0 · one' in rendered, rendered
        assert '⠋ サイドバー' not in rendered, 'title spinner was not removed'
        # Two different repositories in one session must retain their own branch.
        subprocess.run(['git', '-C', str(root), 'symbolic-ref', 'HEAD', 'refs/heads/renamed'], check=True)
        wait_for(lambda: 'project-one · renamed' in screen(), 'branch cache did not refresh')
        assert 'project-two · review' in screen()
        # The same selected details remain available without a preview.
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 36, 70, 0, 0))
        wait_for(lambda: tmux('display-message', '-p', '-t', ui, '#{pane_width}') == '70'
                 and 'project-two · review' in screen()
                 and 'live · auto-follow' not in screen(), 'narrow layout did not render without the preview')
        tmux('set-option', '-g', 'mouse', 'on')
        rows = screen().splitlines()
        row = next(i for i, line in enumerate(rows) if 'project-two · review' in line)
        # Click the second content line of the second agent (1-based SGR coordinates).
        os.write(master, f'\x1b[<0;5;{row + 2}M\x1b[<0;5;{row + 2}m'.encode())
        wait_for(lambda: f'root  {second_root}' in screen(), 'two-line mouse hit testing selected the wrong agent')
        assert 'tmux  fixture:1.0 · two' in screen()
        tmux('send-keys', '-t', ui, 'q')
        wait_for(lambda: ui not in tmux('list-panes', '-a', '-F', '#{pane_id}').splitlines(), 'q did not close dashboard')
        print('Sidebar: per-pane repo/branch, task title, selected details, branch refresh, narrow layout and mouse selection passed')
    finally:
        subprocess.run(['tmux', '-L', sock, 'kill-server'], capture_output=True)
        if master is not None:
            os.close(master)
        if child:
            os.waitpid(child, 0)
