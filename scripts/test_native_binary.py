#!/usr/bin/env python3
"""Native smoke of a Linux server binary (for example one extracted from an image).

Mirrors scripts/test_container.py's API checks without a container runtime:
disposable data, a read-only media fixture, authenticated setup/login,
unauthorised media denial, scan, range bytes, viewer grants, queue revision
conflicts, fixture unchanged and SIGTERM exit zero. It runs the binary on the
host's CPU and libraries: no container isolation, image FFmpeg or GPU claim.
"""
import argparse, datetime, hashlib, json, os, secrets, signal, socket, subprocess, tempfile, time, urllib.error, urllib.request
from pathlib import Path


def cancel(signum, frame):
    raise SystemExit(128 + signum)


def main():
    # An SSH drop or Ctrl-C must still stop the server.
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(sig, cancel)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    report = {'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'native Linux binary API/range/read-only/shutdown smoke; no container, GPU, browser or capacity claim',
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'machine': os.uname().machine,
              'runtime': json.loads(subprocess.check_output([str(binary), '--version-json'], timeout=15)),
              'checks': [], 'complete': False}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    proc = None
    try:
        with tempfile.TemporaryDirectory(prefix='ovl-native-') as tmp:
            media = Path(tmp) / 'media'
            media.mkdir()
            fixture = media / 'Example.S01E01.mp4'
            fixture.write_bytes(b'0123456789' * 100)
            fixture.chmod(0o444)
            media.chmod(0o555)
            digest = hashlib.sha256(fixture.read_bytes()).hexdigest()
            with socket.socket() as s:
                s.bind(('127.0.0.1', 0))
                port = s.getsockname()[1]
            env = {**os.environ, 'OVL_DATA': str(Path(tmp) / 'data'), 'OVL_LISTEN': f'127.0.0.1:{port}',
                   'OVL_SCAN_INTERVAL': '24h', 'OVL_ACCELERATOR': 'cpu', 'RUST_LOG': 'error'}
            proc = subprocess.Popen([str(binary)], env=env, start_new_session=True,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

            def api(path, token='', data=None, method=None, headers=None):
                h = {'Content-Type': 'application/json', 'X-OVL-Request': '1', **(headers or {})}
                if token:
                    h['Authorization'] = 'Bearer ' + token
                request = urllib.request.Request(f'http://127.0.0.1:{port}/api/v1/{path}', headers=h, method=method,
                                                 data=None if data is None else json.dumps(data).encode())
                try:
                    response = urllib.request.urlopen(request, timeout=10)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    body = response.read()
                    if 'application/json' in response.headers.get('Content-Type', ''):
                        body = json.loads(body)
                    return response.status, body

            deadline = time.monotonic() + 45
            while True:
                try:
                    if api('status')[0] == 200:
                        break
                except OSError:
                    pass
                if proc.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError('native server readiness failed')
                time.sleep(.1)
            password = secrets.token_urlsafe(24)
            assert api('setup', data={'Name': 'native-admin', 'Password': password})[0] == 201
            token = api('login', data={'Name': 'native-admin', 'Password': password})[1]['token']
            assert api('items')[0] == 401
            assert api('libraries', token, {'Name': 'Read only', 'Path': str(media), 'Kind': 'tv'})[0] == 201
            assert api('scan', token, {}, 'POST')[0] == 202
            deadline = time.monotonic() + 20
            while api('scan', token)[1]['running']:
                if time.monotonic() >= deadline:
                    raise RuntimeError('scan timed out')
                time.sleep(.05)
            items = api('items', token)[1]
            assert len(items) == 1
            item = items[0]['id']
            status, body = api(f'items/{item}/file', token, headers={'Range': 'bytes=2-5'})
            assert status == 206 and body == b'2345'
            assert api('users', token, {'Name': 'viewer', 'Password': password})[0] == 201
            viewer = api('login', data={'Name': 'viewer', 'Password': password})[1]['token']
            assert api('items', viewer)[1] == []
            assert api(f'items/{item}/file', viewer)[0] == 404
            assert api('lists', token, {'Kind': 'queue', 'Name': 'Native queue', 'ItemIDs': [item, item]})[0] == 201
            assert api('lists/1/cursor', token, {'Revision': 1, 'Cursor': 1}, 'PUT')[0] == 200
            assert api('lists/1/cursor', token, {'Revision': 1, 'Cursor': 0}, 'PUT')[0] == 409
            assert hashlib.sha256(fixture.read_bytes()).hexdigest() == digest and not os.access(fixture, os.W_OK)
            report['checks'] = ['authenticated setup/login', 'unauthorized media denied', 'read-only fixture scan',
                                'range bytes verified', 'viewer grants enforced', 'queue revision conflicts',
                                'fixture mode read-only (same user, no read-only mount) and unchanged']
            os.killpg(proc.pid, signal.SIGTERM)
            assert proc.wait(timeout=20) == 0
            report['checks'].append('SIGTERM exit zero')
            report['complete'] = True
            media.chmod(0o755)
            fixture.chmod(0o644)
            stop(proc)
    finally:
        stop(proc)
        output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


def stop(proc):
    """Stop the server before its temporary data directory is removed."""
    if proc and proc.poll() is None:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.wait()


if __name__ == '__main__':
    main()
