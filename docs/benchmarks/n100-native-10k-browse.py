import concurrent.futures
from contextlib import closing
import hashlib
import http.client
import json
import os
import pathlib
import secrets
import signal
import socket
import sqlite3
import statistics
import subprocess
import tempfile
import time
import urllib.parse
import xml.etree.ElementTree as ET

home = pathlib.Path.home()
root = home / 'ovl-pair-10k-g1whyyha'
jelly = home / 'ovl-jellyfin-12.4-hc7ns1a3/jellyfin'
rust = home / 'ovl-rust-music-artists-final/source/target/release/ovl-server'
report_path = root / 'paired-ready-retry.json'
report = {'scope': 'native authenticated warm loopback, synthetic local fixture; shared N100',
          'repetitions': 5, 'concurrency': 10, 'requests_per_trial': 300,
          'page_size': 50, 'items': 10000, 'arms': {}}

def cancel(signum, frame):
    raise SystemExit(128 + signum)

signal.signal(signal.SIGTERM, cancel)

def file_hash(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def read_db(path, query):
    with sqlite3.connect(f'file:{path}?mode=ro', uri=True, timeout=5) as db:
        return db.execute(query).fetchone()

def response(conn, path, headers, data=None, statuses=(200,)):
    conn.request('GET' if data is None else 'POST', path,
                 body=None if data is None else json.dumps(data),
                 headers=headers if data is None else {**headers, 'Content-Type': 'application/json'})
    res = conn.getresponse()
    raw = res.read(500001)
    if res.status not in statuses or len(raw) > 500000:
        raise ValueError(f'HTTP {res.status}, response length {len(raw)}')
    return json.loads(raw), len(raw)

def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

def server_resources(pid):
    stat = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    status = pathlib.Path(f'/proc/{pid}/status').read_text().splitlines()
    rss = next(int(line.split()[1]) for line in status if line.startswith('VmRSS:'))
    cpu = (int(stat[11]) + int(stat[12])) / os.sysconf('SC_CLK_TCK')
    return cpu, rss

def run_arm(arm):
    env = os.environ.copy()
    if arm == 'jellyfin':
        stage = root / 'jellyfin'
        binary = jelly / 'jellyfin'
        port = int(ET.parse(stage/'config/network.xml').findtext('InternalHttpPort'))
        db_path = stage/'data/data/jellyfin.db'
        assert read_db(db_path, "select count(distinct PresentationUniqueKey),count(*) from BaseItems where Type like '%.Movie' and Name like 'Fixture %'") == (10000, 10000)
        token, user = read_db(db_path, "select AccessToken,UserId from Devices where DeviceId='ovlbench'")
        headers = {'Authorization': f'MediaBrowser Client="OVL benchmark", Device="N100", DeviceId="ovlbench", Version="1.0", Token="{token}"'}
        command = [str(binary), '--datadir', str(stage/'data'), '--configdir', str(stage/'config'),
                   '--cachedir', str(stage/'cache'), '--logdir', str(stage/'logs'),
                   '--webdir', str(jelly/'jellyfin-web'), '--ffmpeg', '/usr/bin/ffmpeg', '--nonetchange']
    else:
        stage = pathlib.Path(tempfile.mkdtemp(prefix='paired-rust-', dir=root))
        binary = rust
        port = free_port()
        headers = {'X-OVL-Request': '1'}
        env.update(OVL_DATA=str(stage/'data'), OVL_LISTEN=f'127.0.0.1:{port}', OVL_ACCELERATOR='cpu')
        command = [str(binary)]
    with socket.socket() as probe:
        if probe.connect_ex(('127.0.0.1', port)) == 0:
            raise RuntimeError(f'{arm} test port already occupied')
    log_path = stage/'paired-measure-retry.log'
    result = {'binary_sha256': file_hash(binary), 'trials': []}
    report['arms'][arm] = result
    with log_path.open('xb') as log:
        process = subprocess.Popen(command, cwd=stage, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            with closing(http.client.HTTPConnection('127.0.0.1', port, timeout=10)) as conn:
                for _ in range(160):
                    if process.poll() is not None:
                        raise RuntimeError(f'{arm} exited {process.returncode}')
                    try:
                        response(conn, '/System/Info/Public' if arm == 'jellyfin' else '/api/v1/status', headers)
                        if arm == 'rust' or 'Core startup complete' in log_path.read_text(errors='replace'):
                            break
                    except (OSError, ValueError, http.client.HTTPException):
                        conn.close()
                    time.sleep(.25)
                else:
                    raise RuntimeError(f'{arm} unavailable')
                if arm == 'jellyfin':
                    folders, _ = response(conn, '/Library/VirtualFolders', headers)
                    library = next(row['ItemId'] for row in folders if row['Name'] == 'Fixture')
                    tasks, _ = response(conn, '/ScheduledTasks', headers)
                    assert next(row['State'] for row in tasks if row['Key'] == 'RefreshLibrary') == 'Idle'
                else:
                    credentials = {'Name': 'ovlbench', 'Password': secrets.token_urlsafe(24)}
                    response(conn, '/api/v1/setup', headers, credentials, (201,))
                    login, _ = response(conn, '/api/v1/login', headers, credentials)
                    headers['Authorization'] = 'Bearer '+login['token']
                    del credentials
                    folder, _ = response(conn, '/api/v1/libraries', headers, {
                        'Name': 'Fixture', 'Path': str(root/'media'), 'Kind': 'movie',
                    }, (201,))
                    library = folder['id']
                    response(conn, '/api/v1/scan', headers, {}, (202,))
                    for _ in range(90):
                        scan, _ = response(conn, '/api/v1/scan', headers)
                        if not scan['running']:
                            break
                        time.sleep(1)
                    else:
                        raise RuntimeError('Rust scan did not complete')

                def path(offset, query=''):
                    if arm == 'rust':
                        return '/api/v1/items?' + urllib.parse.urlencode(dict(
                            library=library, q=query, offset=offset, limit=50))
                    params = dict(UserId=user, ParentId=library, Recursive='true',
                                  IncludeItemTypes='Movie', SortBy='SortName', SortOrder='Ascending',
                                  StartIndex=offset, Limit=50, EnableTotalRecordCount='false')
                    if query:
                        params['SearchTerm'] = query
                    return '/Items?' + urllib.parse.urlencode(params)

                def titles(data):
                    return ([row['Name'] for row in data['Items']] if arm == 'jellyfin'
                            else [row['title'] for row in data])

                expected = [f'Fixture {i:05d}' for i in range(1, 10001)]
                for offset in range(0, 10001, 50):
                    data, _ = response(conn, path(offset), headers)
                    assert titles(data) == expected[offset:offset+50], (arm, offset)
                result['browse_validation'] = 'all 10000 ordered titles and empty tail'
                result['search_probe'] = []
                for offset in (0, 950, 999):
                    data, size = response(conn, path(offset, 'Fixture 00'), headers)
                    names = titles(data)
                    result['search_probe'].append(dict(offset=offset, rows=len(names), bytes=size,
                                                       first=names[:2], last=names[-2:],
                                                       matches=names==expected[:999][offset:offset+50]))
            for trial in range(1, 6):
                def worker(index):
                    output = []
                    with closing(http.client.HTTPConnection('127.0.0.1', port, timeout=15)) as conn:
                        for request in range(30):
                            offset = ((trial*30+request)*10+index) % 200*50
                            start = time.perf_counter()
                            try:
                                data, size = response(conn, path(offset), headers)
                                elapsed = (time.perf_counter()-start)*1000
                                if titles(data) != expected[offset:offset+50]:
                                    raise ValueError('incorrect titles')
                                output.append((elapsed, size, None))
                            except (OSError, ValueError, http.client.HTTPException) as error:
                                output.append((0, 0, str(error)))
                                conn.close()
                    return output
                cpu_before, rss_before = server_resources(process.pid)
                start = time.perf_counter()
                with concurrent.futures.ThreadPoolExecutor(max_workers=10) as pool:
                    rows = [row for work in pool.map(worker, range(10)) for row in work]
                elapsed = time.perf_counter()-start
                cpu_after, rss_after = server_resources(process.pid)
                latencies = sorted(latency for latency, _, err in rows if err is None)
                errors = [err for _, _, err in rows if err is not None]
                stats = dict(trial=trial, seconds=elapsed, requests=len(rows), errors=errors,
                             success_rps=len(latencies)/elapsed, response_bytes=sum(size for _,size,_ in rows),
                             core_cpu_seconds=cpu_after-cpu_before, rss_before_kib=rss_before, rss_after_kib=rss_after)
                if latencies:
                    for percentile in (.50, .95, .99):
                        stats[f'p{int(percentile*100)}_ms'] = latencies[int((len(latencies)-1)*percentile)]
                result['trials'].append(stats)
                print(arm, json.dumps(stats), flush=True)
            if any(trial['errors'] for trial in result['trials']):
                raise RuntimeError(f'{arm} browse requests failed')
            p95 = [trial['p95_ms'] for trial in result['trials']]
            result['p95_median_ms'] = statistics.median(p95) if p95 else None
            result['p95_sample_sd_ms'] = statistics.stdev(p95) if len(p95) > 1 else None
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=25)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            result['exit_code'] = process.returncode

report['fixture_sha256'] = file_hash(root/'fixture.mp4')
for arm in ('jellyfin', 'rust'):
    try:
        run_arm(arm)
    finally:
        report_path.write_text(json.dumps(report, indent=2)+'\n')
print(report_path, flush=True)
