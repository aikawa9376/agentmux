"""Exercise history navigation in a real TUI on an isolated tmux server.

Run after cargo build: python3 tests/preview_smoke.py
"""
import fcntl
import json
import os
import pathlib
import pty
import re
import select
import shlex
import struct
import subprocess
import tempfile
import termios
import time

binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/agentmux'
sock = f'agentmux-preview-test-{os.getpid()}'
child = master = popup = None


def tmux(*args):
    return subprocess.check_output(['tmux', '-L', sock, *args], text=True).rstrip('\n')


def cli(*args):
    return subprocess.check_output([str(binary), '--socket-name', sock, *args], text=True).strip()


def drain():
    while select.select([master], [], [], 0)[0]:
        os.read(master, 65536)


def wait_for(predicate, message, timeout=6):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if master is not None:
            drain()
        result = predicate()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError(message)


def read_until(needle, timeout=6):
    output = b''
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if select.select([master], [], [], .05)[0]:
            output += os.read(master, 65536)
            if needle in output:
                return
    raise AssertionError(f'popup did not render {needle!r}: {output[-1000:]!r}')


with tempfile.TemporaryDirectory(prefix='agentmux-preview-') as directory:
    try:
        log = pathlib.Path(directory) / 'terminal.log'
        log.write_text(''.join(f'\x1b[31mterminal-row-{i:04d} 日本語\x1b[0m\n' for i in range(1, 201)))
        tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'fixture', '-x', '160', '-y', '44',
             f'tail -n +1 -f {shlex.quote(str(log))}')
        pane = tmux('list-panes', '-F', '#{pane_id}')
        cli('mark', pane, '--kind', 'codex', '--name', 'terminal-agent')
        transcript = pathlib.Path(directory) / 'transcript.log'
        transcript.write_text(''.join(f'acp-row-{i:04d} 日本語\n' for i in range(1, 301)))
        acp = tmux('new-window', '-d', '-P', '-F', '#{pane_id}', '-n', 'editor', 'sleep 120')
        cli('publish', acp, '--kind', 'copilot', '--name', 'acp-agent', '--state', 'idle',
            '--owner', 'lazyagent', '--owner-pid', str(os.getpid()), '--preview-path', str(transcript))
        ui = tmux('new-window', '-P', '-F', '#{pane_id}', '-n', 'dashboard',
                  f'{shlex.quote(str(binary))} --socket-name {sock} ui {pane}')
        child, master = pty.fork()
        if child == 0:
            os.environ['TERM'] = 'xterm-256color'
            os.execvp('tmux', ['tmux', '-L', sock, 'attach-session', '-t', 'fixture'])
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 44, 160, 0, 0))

        def screen():
            return tmux('capture-pane', '-p', '-t', ui)

        def keys(*values):
            tmux('send-keys', '-t', ui, *values)

        def preview_rows():
            # Sidebar occupies the left 38%; only compare output, not counters.
            return '\n'.join(line[63:] for line in screen().splitlines()[2:-2])

        def history():
            return 'history ·' in screen()

        wait_for(lambda: 'terminal-row-0200' in screen(), 'terminal preview did not render')
        keys('Tab', 'C-u')  # agents -> preview; Ctrl-u must scroll, not unregister
        wait_for(history, 'Ctrl-u did not scroll the output panel')
        registered = next(p for p in json.loads(cli('list', '--json'))['panes']
                          if p['pane_id'] == pane)
        assert registered['agent_kind'] == 'codex', 'Ctrl-u removed the selected agent'
        keys('C-d')
        wait_for(lambda: 'live · auto-follow' in screen(), 'Ctrl-d did not scroll back to the live bottom')
        keys('PPage')
        wait_for(history, 'PageUp did not enter history')
        before = preview_rows()
        assert 'terminal-row-0200' not in before
        # Paused snapshots survive fresh output and status reconciliation.
        with log.open('a') as stream:
            stream.write('terminal-new-output\n')
        wait_for(lambda: 'terminal-new-output' in tmux('capture-pane', '-p', '-t', pane),
                 'fixture did not produce new output')
        time.sleep(.35)
        drain()
        assert preview_rows() == before, 'new output moved the paused viewport'
        keys('Home')
        wait_for(lambda: 'terminal-row-0001' in screen(), 'Home did not reach tmux history start')
        assert '日本語' in preview_rows()
        colored = next(line for line in tmux('capture-pane', '-ep', '-t', ui).splitlines()
                       if 'terminal-row-0001' in line)
        assert re.search(r'\x1b\[(?:31|38;5;1)m[^\x1b]*terminal-row-0001', colored), repr(colored)
        keys('NPage')
        wait_for(lambda: 'terminal-row-0001' not in preview_rows(), 'PageDown did not advance')
        keys('End')
        wait_for(lambda: 'live · auto-follow' in screen() and 'terminal-new-output' in screen(),
                 'End did not resume live output')
        # Wheel over preview scrolls it; it must not select another agent.
        client = next(line.split('\t')[0] for line in tmux('list-clients', '-F',
                      '#{client_name}\t#{client_control_mode}').splitlines() if line.endswith('\t0'))
        tmux('set-option', '-g', 'mouse', 'on')
        os.write(master, b'\x1b[<64;100;10M')
        wait_for(history, 'preview wheel did not scroll')
        assert re.search(r'terminal-agent\s+history', screen()), 'wheel changed the selected agent'
        # Resizing while reading keeps the paused snapshot.
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 150, 0, 0))
        wait_for(lambda: tmux('display-message', '-p', '-t', ui, '#{pane_width}') == '150' and history(),
                 'resize did not preserve the history viewport')
        # Switching agent resets history rather than showing the previous agent's log.
        keys('Tab', 'Tab', 'j')  # preview -> spaces -> agents, then next agent
        wait_for(lambda: 'acp-row-0300' in screen() and 'live · auto-follow' in screen(),
                 'agent selection did not reset history')
        keys('PPage', 'Home')
        wait_for(lambda: 'acp-row-0001' in screen(), 'ACP history start is inaccessible')
        before = preview_rows()
        with transcript.open('a') as stream:
            stream.write('acp-new-output\n')
        time.sleep(.4)
        drain()
        assert preview_rows() == before, 'ACP output moved the paused viewport'
        keys('End')
        wait_for(lambda: 'acp-new-output' in screen() and 'live · auto-follow' in screen(),
                 'ACP did not resume live output')
        keys('PPage')
        wait_for(history, 'ACP did not enter history again')
        # Source replacement on the same pane must discard the old frozen log.
        replacement = pathlib.Path(directory) / 'replacement.log'
        replacement.write_text('replacement-conversation\n')
        tmux('set-option', '-p', '-t', acp, '@agent_preview_path', str(replacement))
        wait_for(lambda: 'replacement-conversation' in screen() and 'live · auto-follow' in screen(),
                 'conversation replacement retained stale history')
        # Outside preview, Ctrl-u must ask before removing registration.
        keys('Tab', 'Tab', 'C-u')
        wait_for(lambda: 'Remove agent registration' in screen(), 'Ctrl-u did not ask for confirmation')
        assert tmux('show-option', '-pv', '-t', acp, '@agent_kind') == 'copilot'
        keys('Enter')
        wait_for(lambda: 'Remove agent registration' not in screen(), 'Enter did not cancel registration removal')
        assert tmux('show-option', '-pv', '-t', acp, '@agent_kind') == 'copilot'
        keys('C-u', 'Escape')
        wait_for(lambda: 'Remove agent registration' not in screen(), 'Escape did not cancel registration removal')
        assert tmux('show-option', '-pv', '-t', acp, '@agent_kind') == 'copilot'
        keys('C-u', 'y')
        wait_for(lambda: not tmux('display-message', '-p', '-t', acp, '#{@agent_kind}'),
                 'explicit y did not remove registration')
        keys('q')
        wait_for(lambda: ui not in tmux('list-panes', '-a', '-F', '#{pane_id}').splitlines(),
                 'q did not close the dashboard')
        # A display-popup has its own PTY rather than a capture-able tmux pane.
        # Verify its output and keyboard navigation through the attached client.
        drain()
        popup = subprocess.Popen([str(binary), '--socket-name', sock, 'ui', pane,
                                  '--client', client, '--popup', '--width', '95%', '--height', '90%'],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        read_until(b'terminal-agent')
        drain()
        os.write(master, b'\x1b[5~')
        read_until(b'history')
        drain()
        os.write(master, b'\x1b[H')
        # Incremental TUI frames may only emit changed numeric suffixes. Ask
        # tmux to redraw the popup grid before checking the complete row label.
        time.sleep(.1)
        tmux('refresh-client', '-t', client)
        read_until(b'terminal-row-0001')
        drain()
        os.write(master, b'\x1b[F')
        time.sleep(.1)
        tmux('refresh-client', '-t', client)
        read_until(b'terminal-new-output')
        os.write(master, b'q')
        _, error = popup.communicate(timeout=6)
        assert popup.returncode == 0, error
        print('Preview: tmux/ACP history, Ctrl-u/d, wheel, pages, Home/End, paused output, ANSI/CJK, resize, selection/source reset')
        print('Registration: preview Ctrl-u preserves it; list Ctrl-u asks, Enter/Escape cancel, y confirms')
        print('Popup preview: PageUp enters history, Home reaches oldest output, End resumes live output')
    finally:
        if popup is not None and popup.poll() is None:
            popup.terminate()
            popup.wait(timeout=5)
        subprocess.run([str(binary), '--socket-name', sock, 'daemon', 'stop'], capture_output=True)
        subprocess.run(['tmux', '-L', sock, 'kill-server'], capture_output=True)
        if master is not None:
            os.close(master)
        if child:
            os.waitpid(child, 0)
