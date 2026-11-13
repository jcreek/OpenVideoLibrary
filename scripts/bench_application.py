#!/usr/bin/env python3
"""Authenticated Rust browse/search benchmark with disposable synthetic databases.

Measures warm loopback and optionally the first authenticated query after a
process restart. This is not OS-cold, LAN, playback or Jellyfin acceptance. Never points at an existing application data directory.
"""
import argparse
import concurrent.futures
import datetime
import hashlib
import http.client
import itertools
import json
import os
from pathlib import Path
import platform
import random
import secrets
import socket
import sqlite3
import statistics
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from bench_suite import sample


def api(port, path, token='', data=None):
    headers = {'Content-Type': 'application/json', 'X-OVL-Request': '1'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    request = urllib.request.Request(f'http://127.0.0.1:{port}/api/v1/{path}',
                                     headers=headers,
                                     data=None if data is None else json.dumps(data).encode())
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


def seed(data, count):
    with sqlite3.connect(data / 'library.db', timeout=10) as db:
        db.execute('PRAGMA foreign_keys=ON')
        db.execute("INSERT INTO libraries(id,name,path,kind) VALUES(1,'Synthetic','/synthetic','movie')")
        db.executemany('INSERT INTO items(id,title) VALUES(?,?)',
                       ((i, f'Movie {i:08d}') for i in range(1, count + 1)))
        db.executemany('INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,1,?,0,0)',
                       ((i, f'/synthetic/{i}.mp4') for i in range(1, count + 1)))


def load(port, token, query, count, concurrency, duration=None):
    local = threading.local()
    connections = []
    lock = threading.Lock()

    def one(index):
        if not hasattr(local, 'conn'):
            local.conn = http.client.HTTPConnection('127.0.0.1', port, timeout=30)
            with lock:
                connections.append(local.conn)
        start = time.perf_counter()
        try:
            path = '/api/v1/items?' + urllib.parse.urlencode({'q': query, 'limit': 100, 'offset': (index % 10) * 100})
            local.conn.request('GET', path, headers={'Authorization': 'Bearer ' + token})
            response = local.conn.getresponse()
            payload = response.read()
            if response.status != 200:
                return (time.perf_counter() - start) * 1000, 'HTTP' + str(response.status)
            data = json.loads(payload)
            if not isinstance(data, list) or len(data) != 100:
                raise ValueError('invalid response')
            expected = (index % 10) * 100 + 1
            if [v['id'] for v in data] != list(range(expected, expected + 100)):
                raise ValueError('incorrect ordering or results')
            return (time.perf_counter() - start) * 1000, None
        except Exception as error:
            local.conn.close()
            return (time.perf_counter() - start) * 1000, type(error).__name__

    start = time.perf_counter()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
            if duration is None:
                values = list(pool.map(one, range(count)))
            else:
                deadline = start + duration
                indices = itertools.count()

                def worker(_):
                    rows = []
                    while time.perf_counter() < deadline:
                        index = next(indices)
                        if index >= count:
                            break
                        rows.append(one(index))
                    return rows

                values = [row for rows in pool.map(worker, range(concurrency)) for row in rows]
    finally:
        for connection in connections:
            connection.close()
    elapsed = time.perf_counter() - start
    latencies = sorted(ms for ms, error in values if error is None)
    def percentile(p):
        return latencies[min(len(latencies)-1, int((len(latencies)-1)*p))] if latencies else None
    errors = {}
    for _, error in values:
        if error:
            errors[error] = errors.get(error, 0) + 1
    return {'requests': len(values), 'seconds': elapsed, 'errors': len(values)-len(latencies),
            **({'target_seconds': duration, 'request_cap_reached': len(values) == count}
               if duration is not None else {}),
            'error_types': errors, 'successful_rps': len(latencies)/elapsed,
            'p50_ms': percentile(.5), 'p95_ms': percentile(.95), 'p99_ms': percentile(.99)}


def stop_server(process):
    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def wait_for_server(process, port):
    deadline = time.monotonic() + 15
    while process.poll() is None and time.monotonic() <= deadline:
        try:
            api(port, 'status')
            if process.poll() is None:
                return
        except OSError:
            time.sleep(.05)
    raise RuntimeError('benchmark server failed to start')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/ovl-server')
    parser.add_argument('--items', nargs='+', type=int, choices=[10000, 100000], default=[10000, 100000])
    parser.add_argument('--requests', type=int, default=10000,
                        help='Requests per trial, or hard request cap with --seconds')
    parser.add_argument('--seconds', type=int,
                        help='Start requests for this many seconds per trial, then drain in-flight requests')
    parser.add_argument('--repetitions', type=int, default=5)
    parser.add_argument('--concurrency', nargs='+', type=int, default=[1, 10, 25, 50, 100])
    parser.add_argument('--isolation-note', default='Shared host; caches not evicted; no harness media or background FFmpeg workloads')
    parser.add_argument('--process-cold', action='store_true',
                        help='Restart after seeding, then measure the first authenticated query separately')
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    if args.requests < 1 or args.repetitions < 3 or any(c < 1 or c > 1000 for c in args.concurrency):
        parser.error('positive requests, at least three repetitions and concurrency 1–1000 required')
    if args.seconds is not None and not (1 <= args.seconds <= 300 and args.requests <= 1_000_000):
        parser.error('timed trials require 1–300 seconds and at most 1,000,000 requests')
    binary = Path(args.binary).resolve()
    hardware = {'cpu': platform.processor(), 'logical_cpus': os.cpu_count()}
    if platform.system() == 'Darwin':
        for field, key in [('cpu_model','machdep.cpu.brand_string'),('model','hw.model'),('ram_bytes','hw.memsize')]:
            hardware[field] = subprocess.check_output(['sysctl','-n',key],text=True).strip()
    elif Path('/proc/meminfo').exists():
        hardware['meminfo'] = Path('/proc/meminfo').read_text().splitlines()[0]
        hardware['cpu_model'] = next((r.split(':',1)[1].strip() for r in Path('/proc/cpuinfo').read_text().splitlines() if r.startswith('model name')),platform.machine())
    source = hashlib.sha256()
    for path in sorted(Path('crates').rglob('*')):
        if path.is_file():
            source.update(str(path).encode());source.update(path.read_bytes())
    source.update(Path('Cargo.lock').read_bytes())
    source.update(Path('Cargo.toml').read_bytes())
    source.update(Path('rust-toolchain.toml').read_bytes())
    runtime = json.loads(subprocess.check_output([str(binary),'--version-json'],text=True))
    report = {'runtime':runtime,'compiler':subprocess.check_output(['rustc','--version'],text=True).strip(),'hardware':hardware, 'source_sha256':source.hexdigest(), 'kind': ('Rust production authenticated API; process-cold first query and warm loopback'
                       if args.process_cold else 'Rust production authenticated API; warm loopback synthetic browse/search'),
              'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'host': platform.platform(), 'cpu': platform.processor(),
              'logical_cpus': os.cpu_count(), 'sqlite_seed_version': sqlite3.sqlite_version,
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'configuration': vars(args), 'isolation': args.isolation_note,
              'results': [], 'summary': [], 'complete': False}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    rng = random.Random(42)
    for size in args.items:
        trials = [(rep, c, query) for rep in range(args.repetitions) for c in args.concurrency for query in ['', 'Movie']]
        rng.shuffle(trials)
        for rep, concurrency, query in trials:
            with tempfile.TemporaryDirectory(prefix='ovl-app-bench-') as tmp:
                root = Path(tmp)
                with socket.socket() as sock:
                    sock.bind(('127.0.0.1', 0))
                    port = sock.getsockname()[1]
                env = {**os.environ, 'OVL_DATA': str(root/'data'), 'OVL_LISTEN': f'127.0.0.1:{port}',
                       'OVL_SCAN_INTERVAL': '24h', 'OVL_SECURE_COOKIES': 'false', 'RUST_LOG': 'error'}
                with (root/'server.log').open('w+') as log:
                    process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
                    stop = threading.Event()
                    monitor = None
                    try:
                        wait_for_server(process, port)
                        password = secrets.token_urlsafe(24)
                        api(port, 'setup', data={'Name': 'benchmark', 'Password': password})
                        token = api(port, 'login', data={'Name': 'benchmark', 'Password': password})['token']
                        seed(root/'data', size)
                        first_query = None
                        if args.process_cold:
                            stop_server(process)
                            if process.returncode != 0:
                                raise RuntimeError('benchmark server did not shut down cleanly')
                            process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
                            wait_for_server(process, port)
                            cold_before = sample(process.pid)
                            first_query = load(port, token, query, 1, 1)
                            cold_after = sample(process.pid)
                            first_query.update(core_cpu_seconds=max(0, cold_after[1]-cold_before[1]),
                                               core_rss_before_bytes=cold_before[0],
                                               core_rss_after_bytes=cold_after[0])
                            if first_query['errors']:
                                report['failed_cold_trial'] = {
                                    'items': size, 'repetition': rep, 'concurrency': concurrency,
                                    'workload': 'search' if query else 'browse',
                                    'process_cold_first_query': first_query,
                                }
                                output.write_text(json.dumps(report,indent=2)+'\n')
                                raise RuntimeError('first authenticated query failed; see report')
                        canonical_path = 'items?' + urllib.parse.urlencode({'q': query})
                        canonical = None if args.process_cold else api(port, canonical_path, token)
                        before = sample(process.pid)
                        peak = [before[0]]
                        def observe():
                            while not stop.wait(.05):
                                peak[0] = max(peak[0], sample(process.pid)[0])
                        monitor = threading.Thread(target=observe)
                        monitor.start()
                        result = load(port, token, query, args.requests, concurrency, args.seconds)
                        after = sample(process.pid)
                        stop.set()
                        if args.process_cold:
                            canonical = api(port, canonical_path, token)
                        result.update({'items': size, 'repetition': rep, 'concurrency': concurrency,
                                       'workload': 'search' if query else 'browse',
                                       'response_sha256': hashlib.sha256(json.dumps(canonical,sort_keys=True).encode()).hexdigest(),
                                       'core_peak_rss_bytes': max(peak[0],after[0]),
                                       'core_cpu_seconds': max(0,after[1]-before[1]), 'ffmpeg_processes': 0})
                        if first_query is not None:
                            result['process_cold_first_query'] = first_query
                        report['results'].append(result)
                        output.write_text(json.dumps(report,indent=2)+'\n')
                        print(size, result['workload'], concurrency, 'errors', result['errors'], flush=True)
                    finally:
                        stop.set()
                        if monitor:
                            monitor.join(timeout=2)
                        stop_server(process)
    for size in args.items:
        for concurrency in args.concurrency:
            for workload in ['browse','search']:
                rows = [r for r in report['results'] if r['items']==size and r['concurrency']==concurrency and r['workload']==workload]
                p95s = [r['p95_ms'] for r in rows if r['p95_ms'] is not None]
                report['summary'].append({'items':size,'concurrency':concurrency,'workload':workload,
                                          'errors':sum(r['errors'] for r in rows),
                                          'p95_median_ms':statistics.median(p95s) if p95s else None,
                                          'p95_run_stdev_ms':statistics.stdev(p95s) if len(p95s)>1 else None})
                if args.seconds is not None:
                    rates = [r['successful_rps'] for r in rows]
                    report['summary'][-1].update({
                        'request_cap_hits': sum(r['request_cap_reached'] for r in rows),
                        'successful_rps_median': statistics.median(rates),
                        'successful_rps_stdev': statistics.stdev(rates) if len(rates) > 1 else None,
                    })
                if args.process_cold:
                    cold = [r['process_cold_first_query'] for r in rows]
                    values = [r['p95_ms'] for r in cold if r['p95_ms'] is not None]
                    report['summary'][-1]['process_cold_first_query'] = {
                        'attempts': len(cold), 'errors': sum(r['errors'] for r in cold),
                        'median_ms': statistics.median(values) if values else None,
                        'min_ms': min(values) if values else None,
                        'max_ms': max(values) if values else None,
                        'stdev_ms': statistics.stdev(values) if len(values)>1 else None,
                    }
    report['complete'] = True
    output.write_text(json.dumps(report,indent=2)+'\n')
    if any(r['errors'] or r.get('request_cap_reached') or r.get('process_cold_first_query', {}).get('errors')
           for r in report['results']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
