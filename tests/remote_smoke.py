"""End-to-end mirror authentication and filtering using an isolated tmux server.
Run after cargo build: python3 tests/remote_smoke.py
"""
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/agentmux'
sock = f'agentmux-remote-test-{os.getpid()}'
def tmux(*args):
    return subprocess.check_output(['tmux', '-L', sock, *args], text=True).strip()
with tempfile.TemporaryDirectory(prefix='agentmux-remote-') as temp:
    token = 'test-token-' + 'a' * 40
    token_file = pathlib.Path(temp) / 'token'
    token_file.write_text(token)
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    port = listener.getsockname()[1]
    listener.close()
    process = None
    try:
        tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'test', 'sleep 60')
        pane = tmux('list-panes', '-F', '#{pane_id}')
        process = subprocess.Popen([str(binary), '--socket-name', sock, 'serve', '--bind', f'127.0.0.1:{port}', '--token-file', str(token_file)], stderr=subprocess.DEVNULL)
        def get(path, auth=True):
            request = urllib.request.Request(f'http://127.0.0.1:{port}{path}', headers={'Authorization': f'Bearer {token}'} if auth else {})
            try:
                with urllib.request.urlopen(request, timeout=3) as response:
                    return response.status, response.read()
            except urllib.error.HTTPError as error:
                return error.code, error.read()
        for attempt in range(50):
            try:
                assert get('/', False)[0] == 200
                break
            except urllib.error.URLError:
                time.sleep(.1)
        else:
            raise AssertionError('server did not start')
        assert get('/api/agents', False)[0] == 401
        assert json.loads(get('/api/agents')[1])['panes'] == []
        assert get('/api/view/' + pane)[0] == 404
        transcript = pathlib.Path(temp) / 'transcript'
        transcript.write_text('─ Assistant\nLAN mirror marker\n')
        subprocess.run([str(binary), '--socket-name', sock, 'publish', pane, '--kind', 'copilot', '--name', 'ACP', '--state', 'idle', '--owner', 'lazyagent', '--owner-pid', str(os.getpid()), '--preview-path', str(transcript)], check=True)
        assert len(json.loads(get('/api/agents')[1])['panes']) == 1
        assert 'LAN mirror marker' in json.loads(get('/api/view/' + pane)[1])['ansi']
        transcript.unlink()
        assert get('/api/view/' + pane)[0] == 404
        print('LAN mirror: authentication, empty list, agent selection, ACP preview, missing transcript passed')
    finally:
        if process:
            process.terminate()
            process.wait(timeout=5)
        subprocess.run(['tmux', '-L', sock, 'kill-server'], capture_output=True)
