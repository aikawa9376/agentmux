"""Exercise shared state, restart, events, and prompt waits on a private tmux server.
Run after cargo build: python3 tests/observer_smoke.py
"""
import json
import os
import pathlib
import shlex
import socket
import subprocess
import tempfile
import time

binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/agentmux'
sock = f'agentmux-observer-test-{os.getpid()}'

def tmux(*args):
    return subprocess.check_output(['tmux', '-L', sock, *args], text=True).strip()

def cli(*args, check=True):
    result = subprocess.run([str(binary), '--socket-name', sock, *args], text=True, capture_output=True, timeout=15)
    if check and result.returncode:
        raise AssertionError(f'{args}: {result.stderr}')
    return result

def eventually(predicate, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.025)
    raise AssertionError('condition did not become true')

with tempfile.TemporaryDirectory(prefix='agentmux-observer-') as temp:
    try:
        tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'fixture', 'cat')
        pane = tmux('list-panes', '-F', '#{pane_id}')
        starters = [subprocess.Popen([str(binary), '--socket-name', sock, 'daemon', 'start'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(3)]
        paths = []
        for starter in starters:
            out, err = starter.communicate(timeout=10)
            assert starter.returncode == 0, err
            paths.append(out.strip())
        assert len(set(paths)) == 1
        path = pathlib.Path(paths[0])
        assert path.stat().st_mode & 0o777 == 0o600

        def rpc(method, **params):
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(8)
                stream.connect(str(path))
                stream.sendall(json.dumps(dict(method=method, **params)).encode() + b'\n')
                response = json.loads(stream.makefile('rb').readline())
            if response.get('error'):
                raise RuntimeError(response['error'])
            return response['result']

        cli('mark', pane, '--kind', 'codex', '--name', 'reviewer')
        cli('status', pane, 'idle')
        assert rpc('snapshot')['panes'][0]['state'] == 'idle'
        info = rpc('snapshot')['observer']
        cli('status', pane, 'working')
        cli('status', pane, 'idle')
        completed = rpc('snapshot')['panes'][0]
        assert completed['state'] == 'done' and completed['completion_seq']
        assert rpc('snapshot')['panes'][0]['state'] == 'done', 'reads must be passive'
        assert any(event['state'] == 'idle' for event in rpc('events', epoch=info['epoch'], since=info['seq']))
        rpc('seen', pane=pane, client='alice')
        assert rpc('snapshot')['panes'][0]['state'] == 'idle'
        assert rpc('snapshot', client='bob')['panes'][0]['state'] == 'done'

        old_epoch = rpc('status')['epoch']
        cli('daemon', 'stop')
        eventually(lambda: not path.exists())
        cli('daemon', 'start')
        assert rpc('status')['epoch'] != old_epoch
        assert rpc('snapshot')['panes'][0]['completion_seq'] == completed['completion_seq']
        try:
            rpc('events', epoch=old_epoch, since=0)
            raise AssertionError('restart was not reported')
        except RuntimeError as error:
            assert 'observer_restarted' in str(error)

        cli('status', pane, 'blocked')
        refused = cli('send', pane, '--text', 'MUST_NOT_BE_SENT', check=False)
        assert refused.returncode != 0 and 'blocked' in refused.stderr
        assert 'MUST_NOT_BE_SENT' not in tmux('capture-pane', '-p', '-t', pane)

        fixture = pathlib.Path(temp) / 'agent.py'
        fixture.write_text('''import subprocess,sys,time
binary,sock,pane=sys.argv[1:]
def status(state):
    subprocess.run([binary,'--socket-name',sock,'status',pane,state],check=True)
for line in sys.stdin:
    if line.strip() == 'stall': continue
    status('working')
    time.sleep(.25)
    status('idle')
''')
        target = tmux('new-window', '-d', '-P', '-F', '#{pane_id}', 'cat')
        command = shlex.join(['python3', str(fixture), str(binary), sock, target])
        tmux('respawn-pane', '-k', '-t', target, command)
        cli('mark', target, '--kind', 'fixture', '--name', 'worker')
        cli('status', target, 'idle')
        start = time.monotonic()
        result = json.loads(cli('send', target, '--text', 'work', '--wait', '--timeout', '5000').stdout)
        assert time.monotonic() - start >= .2 and result['state'] == 'done'
        stalled = cli('send', target, '--text', 'stall', '--wait', '--timeout', '300', check=False)
        assert stalled.returncode != 0 and 'timeout' in stalled.stderr, 'old done must not satisfy a new prompt'

        waiter = subprocess.Popen([str(binary), '--socket-name', sock, 'wait', target, '--until', 'blocked', '--timeout', '4000'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        time.sleep(.15)
        tmux('respawn-pane', '-k', '-t', target, command)
        rpc('refresh')
        _, error = waiter.communicate(timeout=6)
        assert waiter.returncode != 0 and 'changed' in error
        print('Observer: single owner, private IPC, retained done, client seen, restart, events, blocked guard, prompt wait, generation guard passed')
    finally:
        cli('daemon', 'stop', check=False)
        subprocess.run(['tmux', '-L', sock, 'kill-server'], capture_output=True)
