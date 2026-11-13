#!/usr/bin/env python3
"""SSH worker for a disposable LAN playback measurement; stdout is a private JSON pipe."""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import platform
import re
import secrets
import select
import socket
import subprocess
import sys
import tempfile
import time
import threading
from urllib.error import HTTPError

from bench_application import api, stop_server, wait_for_server
from validate_n100 import source_hash


def port_for(host):
    with socket.socket() as sock:
        sock.bind((host, 0))
        return sock.getsockname()[1]


def sample_playback(pid, stop, metrics):
    clock_ticks = os.sysconf('SC_CLK_TCK')
    page_bytes = os.sysconf('SC_PAGE_SIZE')
    seen = {}
    while not stop.is_set():
        core_rss, _ = sample_proc(pid)
        metrics['core_peak_rss_bytes'] = max(metrics['core_peak_rss_bytes'], core_rss)
        children = set()
        try:
            for path in Path(f'/proc/{pid}/task').glob('*/children'):
                try:
                    children.update(path.read_text().split())
                except OSError:
                    continue
        except OSError:
            pass
        worker_rss = worker_count = 0
        for child in children:
            try:
                raw = Path(f'/proc/{child}/stat').read_text()
                end = raw.rindex(')')
                fields = raw[end+2:].split()
                if raw[raw.index('(')+1:end] != 'ffmpeg' or int(fields[1]) != pid:
                    continue
                key = (child, fields[19])  # PID plus process start tick.
                cpu = (int(fields[11]) + int(fields[12])) / clock_ticks
                seen[key] = max(cpu, seen.get(key, 0))
                worker_rss += int(fields[21]) * page_bytes
                worker_count += 1
            except (OSError, ValueError, IndexError):
                continue
        metrics['ffmpeg_peak_workers'] = max(metrics['ffmpeg_peak_workers'], worker_count)
        metrics['ffmpeg_peak_rss_bytes'] = max(metrics['ffmpeg_peak_rss_bytes'], worker_rss)
        metrics['ffmpeg_sampled_cpu_lower_bound_seconds'] = sum(seen.values())
        metrics['samples'] += 1
        stop.wait(.1)


def sample_proc(pid):
    try:
        raw = Path(f'/proc/{pid}/stat').read_text()
        fields = raw[raw.rindex(')')+2:].split()
        return (int(fields[21]) * os.sysconf('SC_PAGE_SIZE'),
                (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK'))
    except (OSError, ValueError, IndexError):
        return 0, 0


def observe_scans(log_path, offset):
    with log_path.open('rb') as stream:
        stream.seek(offset)
        data = stream.read(1 << 20)
    if len(data) >= 1 << 20:
        raise RuntimeError('scan trace exceeded measurement limit')
    return {'completed_ms': [int(ms) for ms in re.findall(rb'scan finished[^\n]*elapsed_ms=(\d+)', data)],
            'incomplete': data.count(b'scan incomplete; records preserved')}


def await_client(log_path, log_offset, background, restarted):
    deadline = time.monotonic() + (360 if background == "scan" else 240)
    snapshots = 0
    pending = bytearray()
    while time.monotonic() < deadline:
        ready, _, _ = select.select([sys.stdin], [], [], max(0, deadline - time.monotonic()))
        if not ready:
            break
        chunk = os.read(sys.stdin.fileno(), 64 - len(pending))
        if not chunk:
            break
        pending.extend(chunk)
        if b'\n' not in pending:
            if len(pending) >= 64:
                break
            continue
        line, _, trailing = pending.partition(b'\n')
        if trailing:
            break
        pending.clear()
        command = line.strip()
        if command == b'done':
            return True
        if command != b'scan-snapshot' or not background or snapshots >= (15 if background == 'scan' else 10):
            break
        snapshots += 1
        print(json.dumps({**observe_scans(log_path, log_offset),
                          'uptime_ms': (time.monotonic() - restarted) * 1000}), flush=True)
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--listen', required=True, type=ipaddress.IPv4Address)
    parser.add_argument('--unbuffered', action='store_true',
                        help='Generate a bounded padded 60-second fixture for a far seek')
    parser.add_argument('--sustained', action='store_true',
                        help='Generate the 36-second sustained browser playback fixture')
    parser.add_argument('--background', choices=('idle', 'scan'),
                        help='Add a 10k-entry browse/search library for the mixed benchmark')
    args = parser.parse_args()
    if args.background and not args.sustained:
        parser.error('background workload requires sustained playback')
    if args.sustained and args.unbuffered:
        parser.error('choose sustained or unbuffered')
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
        duration = 36 if args.sustained else 60 if args.unbuffered else 6
        width, height = (1280, 720) if args.sustained else (640, 360)
        moving = (f'testsrc2=size={width}x{height}:rate=24:d={duration-2}' if args.sustained
                  else f'color=blue:size={width}x{height}:rate=24:d={duration-2}')
        seek_target = 55 if args.unbuffered else 3
        padding = (['-preset', 'ultrafast', '-b:v', '10M', '-minrate', '10M',
                    '-maxrate', '10M', '-bufsize', '2M',
                    '-x264-params', 'nal-hrd=cbr:force-cfr=1']
                   if args.unbuffered else ['-preset', 'veryfast', '-b:v', '2800k'] if args.sustained else [])
        subprocess.run([
            'ffmpeg', '-nostdin', '-v', 'error',
            '-f', 'lavfi', '-i', f'color=red:size={width}x{height}:rate=24:d=2',
            '-f', 'lavfi', '-i', moving,
            '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000',
            '-filter_complex', '[0:v][1:v]concat=n=2:v=1:a=0[v]',
            '-map', '[v]', '-map', '2:a:0', '-t', str(duration),
            '-c:v', 'libx264', '-threads', '2', '-pix_fmt', 'yuv420p',
            '-g', '24', *padding, '-c:a', 'aac', '-movflags', '+faststart',
            '-fs', str(80 << 20), str(fixture),
        ], check=True, timeout=60)
        fixture_bytes = fixture.stat().st_size
        if fixture_bytes >= 80 << 20 or (args.unbuffered and fixture_bytes < 50 << 20):
            raise RuntimeError('fixture size outside the expected bounds')
        with fixture.open('rb') as stream:
            fixture_hash = hashlib.sha256()
            for chunk in iter(lambda: stream.read(1 << 20), b''):
                fixture_hash.update(chunk)
        seek_keyframe = None
        if args.unbuffered:
            packet = json.loads(subprocess.check_output([
                'ffprobe', '-v', 'error', '-select_streams', 'v:0',
                '-read_intervals', f'{seek_target}%+#1', '-show_packets',
                '-show_entries', 'packet=pts_time,pos,flags', '-of', 'json', str(fixture),
            ], timeout=5))['packets'][0]
            seek_keyframe = {'position': int(packet['pos']), 'seconds': float(packet['pts_time'])}
            if ('K' not in packet['flags'] or not seek_target - 1 <= seek_keyframe['seconds'] <= seek_target
                    or not fixture_bytes * .75 <= seek_keyframe['position'] < fixture_bytes):
                raise RuntimeError('unexpected target keyframe location')
        fixture.chmod(0o444)
        scan_root = None
        scan_count = 10000 if args.background else 0
        if scan_count:
            scan_root = root / 'scan-media'
            scan_root.mkdir()
            for index in range(1, scan_count + 1):
                os.link(fixture, scan_root / f'Scan {index:06}.mp4')
        web = root / 'web'
        web.mkdir()
        (web / 'index.html').write_text(
            '<!doctype html><video width="640" height="360" muted playsinline></video>')
        hls_hash = None
        if args.sustained:
            asset = Path('scripts/bench-hls.min.js')
            with asset.open('rb') as stream:
                bundle = stream.read(2 << 20)
            if not bundle or len(bundle) >= 2 << 20:
                raise RuntimeError('HLS benchmark bundle is missing or too large')
            (web / 'bench-hls.js').write_bytes(bundle)
            hls_hash = hashlib.sha256(bundle).hexdigest()
        env = {**os.environ, 'OVL_DATA': str(root / 'data'), 'OVL_WEB': str(web),
               'OVL_SCAN_INTERVAL': '24h', 'OVL_SECURE_COOKIES': 'false',
               'OVL_ACCELERATOR': 'cpu', 'OVL_MAX_STREAMS': '2',
               'OVL_TRANSCODE_BYTES': str(128 << 20), 'RUST_LOG': 'error'}
        port = port_for('127.0.0.1')
        log_path = root / 'server.log'
        with log_path.open('wb') as log:
            def start(address, port, measuring=False):
                settings = {**env, 'OVL_LISTEN': f'{address}:{port}'}
                if measuring and args.background:
                    settings.update(RUST_LOG='ovl_server::scan=info', NO_COLOR='1',
                                    OVL_SCAN_INTERVAL='60s' if args.background == 'scan' else '24h')
                return subprocess.Popen([str(binary)], env=settings,
                                        stdout=log, stderr=log)

            server = start('127.0.0.1', port)
            monitor = None
            stop = threading.Event()
            try:
                wait_for_server(server, port)
                password = secrets.token_urlsafe(24)
                credentials = {'Name': 'benchmark', 'Password': password}
                api(port, 'setup', data=credentials)
                admin = api(port, 'login', data=credentials)['token']
                library = api(port, 'libraries', admin, {'Name': 'Fixture', 'Path': str(media), 'Kind': 'movie'})
                scan_library = (api(port, 'libraries', admin,
                                    {'Name': 'Scan fixture', 'Path': str(scan_root), 'Kind': 'movie'})['id']
                                if scan_root else None)
                viewer = {'Name': 'viewer', 'Password': secrets.token_urlsafe(24)}
                api(port, 'users', admin, {**viewer, 'Libraries': [library['id']] + ([scan_library] if scan_library else [])})
                api(port, 'scan', admin, {})
                deadline = time.monotonic() + (45 if args.background else 15)
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
                items = api(port, f'items?library={library["id"]}', token)
                if scan_library:
                    final_page = api(port, f'items?library={scan_library}&limit=1&offset={scan_count-1}', token)
                    if len(final_page) != 1 or final_page[0]['title'] != f'Scan {scan_count:06}':
                        raise RuntimeError('background library initial scan is incomplete')
                if len(items) != 1:
                    raise RuntimeError('expected exactly one scanned item')
                stop_server(server)
                if server.returncode != 0:
                    raise RuntimeError('loopback bootstrap did not shut down cleanly')
                port = port_for(host)
                log_offset = log_path.stat().st_size
                restarted = time.monotonic()
                server = start(host, port, measuring=True)
                # The client checks readiness on the real LAN connection.
                before = sample_proc(server.pid)
                metrics = {'core_peak_rss_bytes': 0, 'ffmpeg_peak_workers': 0,
                           'ffmpeg_peak_rss_bytes': 0, 'ffmpeg_sampled_cpu_lower_bound_seconds': 0, 'samples': 0}
                if args.sustained:
                    monitor = threading.Thread(target=sample_playback, args=(server.pid, stop, metrics))
                    monitor.start()
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
                        'fixture_sha256': fixture_hash.hexdigest(),
                        'fixture_bytes': fixture_bytes,
                        'fixture_seconds': duration,
                        'fixture_dimensions': [width, height],
                        'background_mode': args.background,
                        'scan_library': scan_library,
                        'scan_library_entries': scan_count,
                        'hls_bundle_sha256': hls_hash,
                        'seek_target_seconds': seek_target,
                        'seek_keyframe': seek_keyframe,
                        'unbuffered_required': args.unbuffered,
                    },
                }), flush=True)
                # EOF, a completion line or a bounded timeout ends the LAN listener.
                finished = await_client(log_path, log_offset, args.background, restarted)
                after = sample_proc(server.pid)
            finally:
                stop.set()
                if monitor:
                    monitor.join()
                stop_server(server)
            if not finished or server.returncode != 0:
                raise RuntimeError('client did not finish or LAN server did not shut down cleanly')
            print(json.dumps({'shutdown_exit': server.returncode,
                              'core_cpu_seconds': max(0, after[1]-before[1]),
                              'core_rss_end_bytes': after[0],
                              **(metrics if args.sustained else {})}), flush=True)


if __name__ == '__main__':
    main()
