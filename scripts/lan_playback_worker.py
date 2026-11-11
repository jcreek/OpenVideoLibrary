#!/usr/bin/env python3
"""SSH worker for a disposable LAN playback measurement; stdout is a private JSON pipe."""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import platform
import secrets
import select
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError

from bench_application import api, stop_server, wait_for_server
from bench_suite import sample
from validate_n100 import source_hash


def port_for(host):
    with socket.socket() as sock:
        sock.bind((host, 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--listen', required=True, type=ipaddress.IPv4Address)
    args = parser.parse_args()
    if not any(args.listen in ipaddress.ip_network(net)
               for net in ('10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16', '127.0.0.0/8')):
        parser.error('bind only to a private IPv4 or loopback address')
    host = str(args.listen)
    binary = Path('target/release/ovl-server').resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ovl-lan-playback-') as tmp:
        root = Path(tmp)
        media = root / 'media'
        media.mkdir()
        fixture = media / 'Playback Fixture.mp4'
        subprocess.run([
            'ffmpeg', '-nostdin', '-v', 'error',
            '-f', 'lavfi', '-i', 'color=red:size=640x360:rate=24:d=2',
            '-f', 'lavfi', '-i', 'color=blue:size=640x360:rate=24:d=4',
            '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000',
            '-filter_complex', '[0:v][1:v]concat=n=2:v=1:a=0[v]',
            '-map', '[v]', '-map', '2:a:0', '-t', '6',
            '-c:v', 'libx264', '-threads', '2', '-pix_fmt', 'yuv420p',
            '-g', '24', '-c:a', 'aac', '-movflags', '+faststart', str(fixture),
        ], check=True, timeout=30)
        fixture.chmod(0o444)
        web = root / 'web'
        web.mkdir()
        (web / 'index.html').write_text(
            '<!doctype html><video width="640" height="360" muted playsinline></video>')
        env = {**os.environ, 'OVL_DATA': str(root / 'data'), 'OVL_WEB': str(web),
               'OVL_SCAN_INTERVAL': '24h', 'OVL_SECURE_COOKIES': 'false',
               'OVL_ACCELERATOR': 'cpu', 'RUST_LOG': 'error'}
        port = port_for('127.0.0.1')
        with (root / 'server.log').open('wb') as log:
            def start(address, port):
                return subprocess.Popen([str(binary)], env={**env, 'OVL_LISTEN': f'{address}:{port}'},
                                        stdout=log, stderr=log)

            server = start('127.0.0.1', port)
            try:
                wait_for_server(server, port)
                password = secrets.token_urlsafe(24)
                credentials = {'Name': 'benchmark', 'Password': password}
                api(port, 'setup', data=credentials)
                admin = api(port, 'login', data=credentials)['token']
                library = api(port, 'libraries', admin, {'Name': 'Fixture', 'Path': str(media), 'Kind': 'movie'})
                viewer = {'Name': 'viewer', 'Password': secrets.token_urlsafe(24)}
                api(port, 'users', admin, {**viewer, 'Libraries': [library['id']]})
                api(port, 'scan', admin, {})
                deadline = time.monotonic() + 15
                while api(port, 'scan', admin)['running']:
                    if time.monotonic() >= deadline:
                        raise RuntimeError('fixture scan timed out')
                    time.sleep(.05)
                api(port, 'logout', admin, {})
                token = api(port, 'login', data=viewer)['token']
                try:
                    api(port, 'users', token)
                except HTTPError as error:
                    if error.code != 403:
                        raise
                else:
                    raise RuntimeError('fixture viewer unexpectedly has admin access')
                items = api(port, 'items', token)
                if len(items) != 1:
                    raise RuntimeError('expected exactly one scanned item')
                stop_server(server)
                if server.returncode != 0:
                    raise RuntimeError('loopback bootstrap did not shut down cleanly')
                port = port_for(host)
                server = start(host, port)
                # The client checks readiness on the real LAN connection.
                before = sample(server.pid)
                print(json.dumps({
                    'origin': f'http://{host}:{port}', 'token': token, 'item': items[0]['id'],
                    'server': {
                        'platform': platform.platform(),
                        'cpu': next((line.split(':', 1)[1].strip()
                                     for line in Path('/proc/cpuinfo').read_text().splitlines()
                                     if line.startswith('model name')), platform.machine()),
                        'logical_cpus': os.cpu_count(),
                        'memory': Path('/proc/meminfo').read_text().splitlines()[0],
                        'runtime': json.loads(subprocess.check_output([str(binary), '--version-json'], timeout=5)),
                        'ffmpeg': subprocess.check_output(['ffmpeg', '-version'], timeout=5, text=True).splitlines()[0],
                        'source_sha256': source_hash(),
                        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                        'worker_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                        'fixture_sha256': hashlib.sha256(fixture.read_bytes()).hexdigest(),
                        'fixture_bytes': fixture.stat().st_size,
                    },
                }), flush=True)
                # EOF, a completion line or a bounded timeout ends the LAN listener.
                ready, _, _ = select.select([sys.stdin], [], [], 240)
                finished = bool(ready and sys.stdin.readline().strip() == 'done')
                after = sample(server.pid)
            finally:
                stop_server(server)
            if not finished or server.returncode != 0:
                raise RuntimeError('client did not finish or LAN server did not shut down cleanly')
            print(json.dumps({'shutdown_exit': server.returncode,
                              'core_cpu_seconds': max(0, after[1]-before[1]),
                              'core_rss_end_bytes': after[0]}), flush=True)


if __name__ == '__main__':
    main()
