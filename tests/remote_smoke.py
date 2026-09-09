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
        def action(pane, binding, kind, text='', auth=True):
            request = urllib.request.Request(f'http://127.0.0.1:{port}/api/action',
                data=json.dumps(dict(pane=pane, binding=binding, action=kind, text=text)).encode(),
                headers={'Content-Type': 'application/json', **({'Authorization': f'Bearer {token}'} if auth else {})})
            try:
                with urllib.request.urlopen(request, timeout=5) as response: return response.status
            except urllib.error.HTTPError as error: return error.code
        # Terminal actions preserve multiline literal content and cannot target plain panes.
        terminal = tmux('new-window', '-d', '-P', '-F', '#{pane_id}', '-n', 'control', 'cat')
        assert action(terminal, 'invalid', 'send', 'not sent') == 409
        subprocess.run([str(binary), '--socket-name', sock, 'mark', terminal, '--kind', 'codex', '--name', 'test'], check=True)
        info = json.loads(get('/api/agents')[1])['controls'][terminal]
        assert info['available']
        assert action(terminal, info['binding'], 'send', 'not sent', auth=False) == 401
        assert action(terminal, info['binding'], 'send', 'bad\x1btext') == 409
        assert action(terminal, 'old-binding', 'send', 'not sent') == 409
        assert action(terminal, info['binding'], 'send', 'mobile first line\nsecond line') == 200
        time.sleep(.1)
        screen = tmux('capture-pane', '-p', '-t', terminal)
        assert 'mobile first line' in screen and 'second line' in screen
        assert action(terminal, info['binding'], 'interrupt') == 200
        # ACP goes through the published owner RPC, never its editor buffer.
        nvim_socket = pathlib.Path(temp) / 'nvim.sock'
        init = pathlib.Path(temp) / 'init.lua'
        transcript.write_text('ACP remote test')
        delivered = pathlib.Path(temp) / 'delivered.json'
        init.write_text("""
local backend = {
  paste_and_submit = function(id, text)
    vim.fn.writefile({vim.json.encode({id=id, text=text})}, %s); return true
  end,
  send_keys = function(id, key)
    vim.fn.writefile({vim.json.encode({id=id, key=key})}, %s); return true
  end,
}
package.loaded['lazyagent.logic.state'] = {sessions={test={backend='buffer_acp', pane_id='acp-test', acp_transcript_path=%s}}}
package.loaded['lazyagent.logic.backend'] = {resolve_backend_for_agent=function() return 'buffer_acp', backend end}
""" % (json.dumps(str(delivered)), json.dumps(str(delivered)), json.dumps(str(transcript))))
        editor = subprocess.Popen(['nvim', '--headless', '--clean', '--listen', str(nvim_socket), '-u', str(init)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(50):
                if nvim_socket.exists(): break
                time.sleep(.05)
            subprocess.run([str(binary), '--socket-name', sock, 'publish', pane, '--kind', 'copilot', '--name', 'ACP', '--state', 'idle', '--owner', 'lazyagent', '--owner-pid', str(editor.pid), '--preview-path', str(transcript)], env={**os.environ, 'NVIM':str(nvim_socket)}, check=True)
            info = json.loads(get('/api/agents')[1])['controls'][pane]
            assert info['available'] and info['mode'] == 'acp'
            assert action(pane, info['binding'], 'send', "quoted ' \"日本語\nnext line") == 200
            assert json.loads(delivered.read_text()) == {'id':'acp-test', 'text':"quoted ' \"日本語\nnext line"}
            assert action(pane, info['binding'], 'interrupt') == 200
            assert json.loads(delivered.read_text()) == {'id':'acp-test', 'key':'C-c'}
            tmux('set-option', '-p', '-t', pane, '@agent_preview_path', '/different-thread')
            assert action(pane, info['binding'], 'send', 'stale request') == 409
        finally:
            editor.terminate(); editor.wait(timeout=5)
        print('LAN mirror: auth, preview, terminal send/interrupt, ACP RPC, and stale-target rejection passed')
    finally:
        if process:
            process.terminate()
            process.wait(timeout=5)
        subprocess.run(['tmux', '-L', sock, 'kill-server'], capture_output=True)
