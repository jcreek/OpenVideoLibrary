#!/usr/bin/env python3
"""Disposable real Nginx/TLS + Rust live-backup/restore acceptance.

Uses an already cached Nginx image; never pulls or prunes. Starts its own Rust
process on a temporary port, fixture media and application data. A locally
created certificate is explicitly trusted by the test client.
"""
import argparse
import datetime
import http.cookiejar
import hashlib
import json
import os
import platform
from pathlib import Path
import secrets
import socket
import ssl
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/ovl-server')
    parser.add_argument('--image', default='nginx:stable')
    parser.add_argument('--platform', help='Optional Docker platform override; omit to use the cached image native platform')
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    inspect_command=['docker','image','inspect']
    if args.platform:
        inspect_command += ['--platform',args.platform]
    metadata=json.loads(subprocess.check_output(inspect_command+['--format','{{json .}}',args.image],text=True))
    image_id=metadata['Id']
    runtime_platform=args.platform or 'linux/'+metadata['Architecture']
    report = {'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'Real disposable Nginx TLS proxy and live Rust backup/restore; not a production deployment or playback-quality benchmark',
              'host': platform.platform(), 'nginx_platform': runtime_platform,
              'binary_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
              'runtime': json.loads(subprocess.check_output([binary,'--version-json'],text=True)),
              'nginx_image_id': image_id, 'checks': [], 'complete': False}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    container = 'ovl-nginx-' + secrets.token_hex(6)
    server = None
    proxy = False
    with tempfile.TemporaryDirectory(prefix='ovl-nginx-') as tmp:
        root = Path(tmp)
        root.chmod(0o755)
        media = root / 'media'
        media.mkdir()
        (media / 'Example.mp4').write_bytes(b'0123456789')
        proxy_port, backend_port = port(), port()
        env = {**os.environ, 'OVL_DATA': str(root / 'data'), 'OVL_WEB': str(Path('web/dist').resolve()),
               'OVL_LISTEN': f'0.0.0.0:{backend_port}', 'OVL_SECURE_COOKIES': 'true',
               'OVL_SCAN_INTERVAL': '24h', 'RUST_LOG': 'error'}
        cert, key = root / 'cert.pem', root / 'key.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                        '-subj', '/CN=localhost', '-addext', 'subjectAltName=IP:127.0.0.1,DNS:localhost',
                        '-keyout', str(key), '-out', str(cert)], capture_output=True, check=True, timeout=15)
        config = root / 'nginx.conf'
        config.write_text(f'''pid /tmp/nginx.pid;
error_log /dev/stderr warn;
events {{ worker_connections 128; }}
http {{
 access_log off;
 client_body_temp_path /tmp/client_temp;
 proxy_temp_path /tmp/proxy_temp;
 fastcgi_temp_path /tmp/fastcgi_temp;
 uwsgi_temp_path /tmp/uwsgi_temp;
 scgi_temp_path /tmp/scgi_temp;
 server {{
  listen 8443 ssl;
  ssl_certificate /acceptance/cert.pem;
  ssl_certificate_key /acceptance/key.pem;
  client_max_body_size 64k;
  location / {{
   proxy_pass http://host.docker.internal:{backend_port};
   proxy_set_header Host $host;
   proxy_set_header X-Forwarded-Proto $scheme;
   proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
   proxy_http_version 1.1;
   proxy_buffering off;
   proxy_read_timeout 60s;
  }}
 }}
}}
''')
        log = (root / 'server.log').open('w+')
        context = ssl.create_default_context(cafile=str(cert))
        cookies = http.cookiejar.CookieJar()
        client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies), urllib.request.HTTPSHandler(context=context))
        def api(path, data=None, method=None, token=None, csrf=True, headers=None):
            h = {'Content-Type': 'application/json', **(headers or {})}
            if csrf:
                h['X-OVL-Request'] = '1'
            if token:
                h['Authorization'] = 'Bearer ' + token
            request = urllib.request.Request(f'https://127.0.0.1:{proxy_port}/api/v1/{path}', headers=h, method=method,
                                             data=None if data is None else json.dumps(data).encode())
            try:
                response = client.open(request, timeout=10)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                body = response.read()
                if 'application/json' in response.headers.get('Content-Type', ''):
                    body = json.loads(body)
                return response.status, body, response.headers
        def ready():
            deadline = time.monotonic() + 30
            while True:
                try:
                    if api('status')[0] == 200:
                        return
                except OSError:
                    pass
                if server.poll() is not None:
                    raise RuntimeError('Rust server exited during readiness')
                state=subprocess.check_output(['docker','inspect','--format','{{.State.Running}}',container],text=True,timeout=5).strip()
                if state != 'true':
                    raise RuntimeError('Nginx exited during readiness; see captured logs')
                if time.monotonic() > deadline:
                    raise RuntimeError('proxy/backend readiness timeout')
                time.sleep(.1)
        def stop():
            server.terminate()
            try:
                server.wait(timeout=15)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
                raise RuntimeError('server required SIGKILL')
            assert server.returncode == 0
        try:
            server = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
            command = ['docker', 'run', '--pull=never', '-d', '--name', container, '--read-only', '--cap-drop', 'ALL',
                       '--security-opt', 'no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}',
                       '--tmpfs', '/tmp:rw,nosuid,nodev,size=16m', '--entrypoint', 'nginx',
                       '--mount', f'type=bind,source={root},target=/acceptance,readonly',
                       '-p', f'127.0.0.1:{proxy_port}:8443']
            if args.platform:
                command += ['--platform',args.platform]
            if os.uname().sysname == 'Linux':
                command += ['--add-host', 'host.docker.internal:host-gateway']
            subprocess.run(command + [args.image, '-c', '/acceptance/nginx.conf', '-g', 'daemon off;'], capture_output=True, check=True, timeout=30)
            proxy = True
            ready()
            password = secrets.token_urlsafe(24)
            assert api('setup', {'Name': 'proxy-admin', 'Password': password})[0] == 201
            status, login, headers = api('login', {'Name': 'proxy-admin', 'Password': password})
            assert status == 200
            assert all(v in headers.get('Set-Cookie', '') for v in ['Secure', 'HttpOnly', 'SameSite=Strict'])
            token = login['token']
            assert api('logout', {}, csrf=False)[0] == 403
            assert api('libraries', {'Name': 'Fixture', 'Path': str(media), 'Kind': 'movie'})[0] == 201
            assert api('scan', {}, 'POST')[0] == 202
            deadline = time.monotonic() + 20
            while api('scan')[1]['running']:
                if time.monotonic() > deadline:
                    raise RuntimeError('scan timeout')
                time.sleep(.05)
            item = api('items')[1][0]['id']
            status, content, headers = api(f'items/{item}/file', headers={'Range': 'bytes=2-5'})
            assert status == 206 and content == b'2345'
            assert headers.get('Cache-Control') == 'no-store'
            assert headers.get('Content-Range') == 'bytes 2-5/10'
            assert api('setup', {'Name': 'x' * 65536, 'Password': password})[0] == 413
            assert api('lists', {'Kind': 'queue', 'Name': 'Restore queue', 'ItemIDs': [item, item]})[0] == 201
            assert api('lists/1/cursor', {'Revision': 1, 'Cursor': 1}, 'PUT')[0] == 200
            report['checks'] += ['trusted TLS and Secure/HttpOnly/SameSite cookie', 'proxy CSRF enforcement',
                                 'range bytes/content-range/no-store', '64KiB request limit', 'persistent queue']
            backup, restored = root / 'backup', root / 'restored'
            subprocess.run(['python3', 'scripts/backup.py', 'backup', '--data', env['OVL_DATA'], '--output', str(backup)], check=True, timeout=30)
            subprocess.run(['python3', 'scripts/backup.py', 'restore', '--source', str(backup), '--data', str(restored)], check=True, timeout=30)
            stop()
            env['OVL_DATA'] = str(restored)
            server = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
            ready()
            assert api('me', token=token)[0] == 401
            assert api('login', {'Name': 'proxy-admin', 'Password': password})[0] == 200
            saved = api('lists/1')[1]
            assert saved['cursor'] == 1 and len(saved['entries']) == 2
            assert api(f'items/{item}/file', headers={'Range': 'bytes=2-5'})[1] == b'2345'
            stop()
            report['checks'] += ['live WAL backup and fresh restore', 'restored sessions revoked',
                                 'restored credentials, queue and media preserved', 'both SIGTERM exits zero']
            report['complete'] = True
        finally:
            if server and server.poll() is None:
                server.terminate()
                try:
                    server.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()
            if proxy:
                if not report['complete']:
                    diagnostic=subprocess.run(['docker','logs','--tail','100',container],capture_output=True,text=True,timeout=10)
                    report['nginx_stdout']=diagnostic.stdout
                    report['nginx_stderr']=diagnostic.stderr
                subprocess.run(['docker', 'rm', '-f', container], capture_output=True, timeout=30)
            log.close()
            output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
