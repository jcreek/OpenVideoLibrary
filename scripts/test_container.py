#!/usr/bin/env python3
"""Real image smoke acceptance with disposable data and a read-only media mount.

Requires an already built local image. Does not pull, publish or prune images.
The media fixture contains range-test bytes, not an encoding/quality corpus.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True, timeout=45).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    report = {'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'Linux image API/range/read-only/shutdown smoke; no GPU, browser or stream capacity claim',
              'image': args.image, 'image_id': docker('image', 'inspect', '--format', '{{.Id}}', args.image),
              'runtime': json.loads(docker('run', '--rm', '--read-only', '--network', 'none', args.image, '--version-json')),
              'checks': [], 'complete': False}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    name = 'ovl-acceptance-' + secrets.token_hex(6)
    running = False
    try:
        with tempfile.TemporaryDirectory(prefix='ovl-image-') as tmp:
            media = Path(tmp) / 'media'
            media.mkdir(mode=0o755)
            Path(tmp).chmod(0o755)
            fixture = media / 'Example.S01E01.mp4'
            fixture.write_bytes(b'0123456789' * 100)
            fixture.chmod(0o444)
            digest = hashlib.sha256(fixture.read_bytes()).hexdigest()
            docker('run', '-d', '--name', name, '--read-only', '--cap-drop', 'ALL',
                   '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw,nosuid,nodev,size=16m',
                   '--tmpfs', '/data:rw,nosuid,nodev,size=64m,uid=10001,gid=10001',
                   '--mount', f'type=bind,source={media},target=/media,readonly',
                   '-e', 'RUST_LOG=error', '-e', 'OVL_SCAN_INTERVAL=24h',
                   '-p', '127.0.0.1::8096', args.image)
            running = True
            port = int(docker('port', name, '8096/tcp').split(':')[-1])
            def api(path, token='', data=None, method=None, headers=None):
                h = {'Content-Type': 'application/json', 'X-OVL-Request': '1', **(headers or {})}
                if token:
                    h['Authorization'] = 'Bearer ' + token
                request = urllib.request.Request(f'http://127.0.0.1:{port}/api/v1/{path}',
                                                 headers=h, method=method,
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
                if time.monotonic() >= deadline:
                    raise RuntimeError('image server readiness timed out')
                time.sleep(.1)
            password = secrets.token_urlsafe(24)
            assert api('setup', data={'Name': 'image-admin', 'Password': password})[0] == 201
            token = api('login', data={'Name': 'image-admin', 'Password': password})[1]['token']
            assert api('items')[0] == 401
            assert api('libraries', token, {'Name': 'Read only', 'Path': '/media', 'Kind': 'tv'})[0] == 201
            assert api('scan', token, {}, 'POST')[0] == 202
            deadline = time.monotonic() + 20
            while api('scan', token)[1]['running']:
                if time.monotonic() >= deadline:
                    raise RuntimeError('image scan timed out')
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
            assert api('lists', token, {'Kind': 'queue', 'Name': 'Image queue', 'ItemIDs': [item, item]})[0] == 201
            assert api('lists/1/cursor', token, {'Revision': 1, 'Cursor': 1}, 'PUT')[0] == 200
            assert api('lists/1/cursor', token, {'Revision': 1, 'Cursor': 0}, 'PUT')[0] == 409
            denied = subprocess.run(['docker', 'exec', name, 'sh', '-c', 'printf forbidden >> /media/Example.S01E01.mp4'], capture_output=True, timeout=10)
            assert denied.returncode != 0
            assert hashlib.sha256(fixture.read_bytes()).hexdigest() == digest
            report['checks'] = ['authenticated setup/login', 'unauthorized media denied', 'read-only mount scan',
                                'range bytes verified', 'viewer grants enforced', 'queue revision conflicts',
                                'media write denied and fixture unchanged']
            docker('stop', '-t', '15', name)
            state = json.loads(docker('inspect', '--format', '{{json .State}}', name))
            assert state['ExitCode'] == 0 and not state['OOMKilled']
            report['checks'].append('SIGTERM exit zero without OOM')
            report['complete'] = True
    finally:
        if running:
            docker('rm', '-f', name)
        output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
