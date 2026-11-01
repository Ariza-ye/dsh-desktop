#!/usr/bin/env python3
"""Lifecycle integration checks. Uses a fake DSH unless --real is passed."""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import selectors
import signal
import socket
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def wait_for_url(process, timeout=40):
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline and process.poll() is None:
            for key, _ in selector.select(.2):
                line = key.fileobj.readline().strip()
                if line.startswith('dsh web: http://127.0.0.1:'):
                    return line[len('dsh web: '):]
        raise AssertionError('DSH did not publish a local URL before exiting or timing out')
    finally:
        selector.close()


def assert_port_closed(port):
    with socket.socket() as sock:
        assert sock.connect_ex(('127.0.0.1', port)) != 0, 'DSH port is still open'


def run_case(binary, env, shutdown, real=False):
    process = subprocess.Popen([str(binary)], env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    try:
        url = wait_for_url(process)
        port = urllib.parse.urlparse(url).port
        # DSH exchanges the bootstrap token for a cookie using a 302 redirect.
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        with opener.open(url, timeout=5) as response:
            assert response.status == 200
        if shutdown == 'request':
            process.stdin.write('shutdown\n')
            process.stdin.flush()
        elif shutdown == 'eof':
            process.stdin.close()
        else:
            process.send_signal(signal.SIGTERM)
        assert process.wait(timeout=8) == 0
        assert_port_closed(port)
        if not real:
            descendant_port = int(Path(env['DSH_TEST_DESCENDANT_PORT']).read_text())
            assert_port_closed(descendant_port)
        print(f'PASS: {"real" if real else "mock"} DSH HTTP ready; {shutdown} cleans process tree and ports')
    finally:
        if process.poll() is None:
            if not process.stdin.closed:
                process.stdin.close()
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--real', action='store_true', help='Also run the installed DSH (uses your DSH profile)')
    args = parser.parse_args()
    target = subprocess.check_output(['rustc', '-vV'], text=True).split('host: ')[1].splitlines()[0]
    binary = ROOT / 'src-tauri' / 'binaries' / f'dsh-sidecar-{target}'
    assert binary.is_file(), 'Run npm run build:sidecar first'
    with tempfile.TemporaryDirectory(prefix='dsh-desktop-test-') as temporary:
        directory = Path(temporary)
        executable = directory / 'dsh'
        executable.write_text('''#!/usr/bin/env python3
import http.server, os, socket, subprocess, sys
assert sys.argv[1:] == ['web', '--no-open', '--host', '127.0.0.1', '--port', '0']
# A descendant that ignores SIGTERM proves the force-cleanup path works.
subprocess.Popen([sys.executable, '-c', """
import os, signal, socket, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
s = socket.socket()
s.bind(('127.0.0.1', 0))
s.listen()
with open(os.environ['DSH_TEST_DESCENDANT_PORT'], 'w') as f: f.write(str(s.getsockname()[1]))
time.sleep(60)
"""])
server = http.server.HTTPServer(('127.0.0.1', 0), http.server.SimpleHTTPRequestHandler)
print(f'dsh web: http://127.0.0.1:{server.server_port}/', flush=True)
server.serve_forever()
''')
        executable.chmod(0o755)
        env = dict(os.environ, DSH_EXECUTABLE=str(executable), DSH_TEST_DESCENDANT_PORT=str(directory / 'port'))
        for shutdown in ['request', 'eof', 'signal']:
            run_case(binary, env, shutdown)
        result = subprocess.run([str(binary)], input='', text=True, capture_output=True,
                                env=dict(env, DSH_EXECUTABLE=str(directory / 'missing')), timeout=8)
        assert result.returncode != 0 and 'DSH_EXECUTABLE' in result.stderr
        print('PASS: missing executable produces actionable failure')
        if args.real:
            run_case(binary, dict(os.environ), 'request', real=True)
            run_case(binary, dict(os.environ), 'eof', real=True)


if __name__ == '__main__':
    main()
