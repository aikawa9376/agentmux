"""Exercise actual popup rendering, keyboard focus, close, and unchanged layout.
Run after cargo build: python3 tests/popup_smoke.py
"""
import fcntl
import json
import os
import pathlib
import pty
import select
import socket
import struct
import subprocess
import termios
import time

binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/agentmux'
sock = f'agentmux-popup-test-{os.getpid()}'
child = master = popup = None

def tmux(*args):
    return subprocess.check_output(['tmux', '-L', sock, *args], text=True).strip()

def cli(*args):
    return subprocess.check_output([str(binary), '--socket-name', sock, *args], text=True).strip()

def read_until(needle, timeout=6):
    output = b''
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if select.select([master], [], [], .05)[0]:
            output += os.read(master, 65536)
            if needle in output:
                return output
    raise AssertionError(f'popup did not render {needle!r}: {output[-2000:]!r}')

try:
    tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'fixture', '-x', '140', '-y', '40', 'cat')
    origin = tmux('list-panes', '-F', '#{pane_id}')
    target = tmux('split-window', '-d', '-h', '-P', '-F', '#{pane_id}', 'cat')
    for pane, name in [(origin, 'origin-agent'), (target, 'target-agent')]:
        cli('mark', pane, '--kind', 'codex', '--name', name)
        cli('status', pane, 'idle')
    child, master = pty.fork()
    if child == 0:
        os.environ['TERM'] = 'xterm-256color'
        os.execvp('tmux', ['tmux', '-L', sock, 'attach-session', '-t', 'fixture'])
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        clients = tmux('list-clients', '-F', '#{client_name}\t#{client_control_mode}').splitlines()
        client = next((line.split('\t')[0] for line in clients if line.endswith('\t0')), None)
        if client:
            break
        time.sleep(.05)
    else:
        raise AssertionError('interactive client did not attach')
    # The client is listed before its first SIGWINCH has necessarily resized the window.
    deadline = time.monotonic() + 4
    while time.monotonic() < deadline:
        if tmux('display-message', '-p', '-t', origin, '#{window_width}x#{window_height}') == '140x39':
            break
        time.sleep(.05)
    else:
        raise AssertionError('attached client did not set the expected grid')
    def geometry():
        return tmux('list-panes', '-F', '#{pane_id}:#{pane_left},#{pane_top},#{pane_width},#{pane_height}')
    layout = geometry()
    panes = tmux('list-panes', '-a', '-F', '#{pane_id}')

    def open_popup(explicit_client=False):
        # Drain the old frame so a stale label cannot satisfy the rendering check.
        while select.select([master], [], [], 0)[0]:
            os.read(master, 65536)
        args = [str(binary), '--socket-name', sock, 'ui', origin, '--popup', '--width', '90%', '--height', '85%']
        if explicit_client:
            args += ['--client', client]
        return subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    popup = open_popup(explicit_client=True)
    read_until(b'origin-agent')  # popup must not mistake the origin pane for its own PTY
    cli('status', origin, 'working')
    cli('status', origin, 'idle')
    # Reading the dashboard must not mark the real pane hidden beneath it seen.
    path = json.loads(cli('daemon', 'status'))['socket']
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(5)
        stream.connect(path)
        stream.sendall(json.dumps(dict(method='snapshot', client=client)).encode() + b'\n')
        snapshot = json.loads(stream.makefile('rb').readline())['result']
    assert next(p for p in snapshot['panes'] if p['pane_id'] == origin)['state'] == 'done'
    os.write(master, b'q')
    _, error = popup.communicate(timeout=6)
    assert popup.returncode == 0, error
    assert geometry() == layout
    assert tmux('list-panes', '-a', '-F', '#{pane_id}') == panes
    cli('focus', origin, '--client', client)
    popup = open_popup()
    read_until(b'origin-agent')
    os.write(master, b'j\r')
    _, error = popup.communicate(timeout=6)
    assert popup.returncode == 0, error
    assert tmux('display-message', '-p', '-c', client, '#{pane_id}') == target
    assert geometry() == layout
    assert tmux('list-panes', '-a', '-F', '#{pane_id}') == panes
    # Completion notifications run independently of an open dashboard.
    cli('status', origin, 'working')
    cli('status', origin, 'idle')
    needle = 'agentmux: origin-agent done'
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        messages = tmux('show-messages', '-t', client)
        if needle in messages:
            break
        time.sleep(.025)
    else:
        raise AssertionError('background completion did not notify the attached client')
    count = messages.count(needle)
    cli('focus', origin, '--client', client)
    cli('status', origin, 'working')
    cli('status', origin, 'idle')
    time.sleep(.2)
    assert tmux('show-messages', '-t', client).count(needle) == count, 'focused pane notification was not suppressed'
    print('Popup: renders origin, q closes, Enter focuses and closes, pane count/layout unchanged')
    print('Notifications: background completion delivered, active pane suppressed')
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
