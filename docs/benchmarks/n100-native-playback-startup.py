"""Paired native playback-startup diagnostic: Jellyfin 12.4 and OVL on the N100.

Warm loopback, one client, generated local fixture, shared host. For each arm a
fresh server and isolated data directory is started, an account and a movie
library are created, and the scan is awaited. Three scenarios are then timed
with fresh sessions:

* direct: authenticated 2 MiB range read of the source file;
* transcode-0 / transcode-60: CPU H.264 1080p 5 Mb/s + AAC stereo 128 kb/s HLS
  starting at 0 s and 60 s, timed from the first request to the first media
  segment fully received. Jellyfin is configured for x264 veryfast with two
  threads, 4 s segments and no stream copy to match OVL's single rendition.

Every timed segment is validated: H.264 video at 1080 lines, AAC audio, and a
first decoded frame within two frames of the expected source position (the
fixture encodes its frame number as bars). Server CPU includes reaped FFmpeg
children. Results are diagnostics, not LAN first-frame acceptance.
"""
import hashlib, http.client, json, os, pathlib, secrets, shutil, signal, socket, sqlite3, subprocess, tempfile, time, urllib.parse, uuid

HOME = pathlib.Path.home()
JELLY = HOME / 'ovl-jellyfin-12.4-hc7ns1a3/jellyfin'
RUST = HOME / 'ovl-rust-acceptance/source/target/release/ovl-server'
REPS = 5
ROOT = pathlib.Path(tempfile.mkdtemp(prefix='ovl-playback-startup-', dir=HOME))
AUTH = 'MediaBrowser Client="OVL benchmark", Device="N100", DeviceId="benchuser", Version="1.0"'


def cancel(signum, frame):
    raise SystemExit(128 + signum)


# An SSH disconnect (SIGHUP) or Ctrl-C must still run the cleanup that stops
# both trial servers on the shared host.
for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
    signal.signal(sig, cancel)


def digest(path):
    with open(path, 'rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def call(port, path, method='GET', data=None, headers=None, statuses=(200, 201, 202, 204), raw=False, extra=None, limit=64 << 20):
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=120)
    try:
        body = None if data is None else json.dumps(data)
        h = dict(headers or {})
        if body is not None:
            h['Content-Type'] = 'application/json'
        h.update(extra or {})
        conn.request(method, path, body=body, headers=h)
        res = conn.getresponse()
        payload = res.read(limit + 1)
        if res.status not in statuses or len(payload) > limit:
            raise RuntimeError(f'{method} {path.split("?")[0]} -> HTTP {res.status} ({len(payload)} bytes)')
        if raw:
            return res.status, payload
        return json.loads(payload) if payload else None
    finally:
        conn.close()


def group_cpu(pid):
    """utime+stime+cutime+cstime of the server plus live members of its group."""
    tick = os.sysconf('SC_CLK_TCK')
    total = 0.0
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
        except OSError:
            continue
        if int(fields[2]) != pid:  # pgrp
            continue
        own = int(fields[11]) + int(fields[12])
        children = int(fields[13]) + int(fields[14]) if int(entry.name) == pid else 0
        total += (own + children) / tick
    return total


def rss_kib(pid):
    for line in pathlib.Path(f'/proc/{pid}/status').read_text().splitlines():
        if line.startswith('VmRSS:'):
            return int(line.split()[1])
    return 0


BARS = ','.join(
    f"drawbox=x={bit * 120}:y=0:w=120:h=1080:color=white:t=fill:enable='gt(bitand(n\\,{1 << bit})\\,0)'"
    for bit in range(16))


def frame_number(segment):
    gray = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(segment), '-frames:v', '1', '-vf', 'scale=1920:1080',
                           '-pix_fmt', 'gray', '-f', 'rawvideo', 'pipe:1'], capture_output=True, check=True, timeout=60).stdout
    assert len(gray) == 1920 * 1080, len(gray)
    return sum(1 << bit for bit in range(16) if gray[540 * 1920 + bit * 120 + 60] > 128)


def validate(segment, start):
    probe = json.loads(subprocess.run(['ffprobe', '-v', 'error', '-show_entries', 'stream=codec_type,codec_name,height',
                                       '-of', 'json', str(segment)], capture_output=True, check=True, timeout=60).stdout)
    kinds = {(s['codec_type'], s['codec_name']) for s in probe['streams']}
    heights = {s.get('height') for s in probe['streams'] if s['codec_type'] == 'video'}
    assert ('video', 'h264') in kinds and ('audio', 'aac') in kinds and heights == {1080}, (kinds, heights)
    frame = frame_number(segment)
    assert abs(frame - start * 24) <= 2, ('first frame', frame, start)
    return frame


def prepare_fixture():
    media = ROOT / 'media'
    media.mkdir()
    path = media / 'Startup Fixture (2026).mp4'
    subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', f'color=black:size=1920x1080:rate=24,{BARS},format=yuv420p',
                    '-f', 'lavfi', '-i', "aevalsrc='0.3*lt(mod(t\\,1)\\,0.1)*sin(2*PI*800*t)':s=48000:c=stereo",
                    '-t', '120', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '20', '-g', '48', '-threads', '2',
                    '-c:a', 'aac', '-b:a', '192k', '-movflags', '+faststart', str(path)], check=True, timeout=1800)
    (media / 'Startup Fixture (2026).nfo').write_text('<movie><title>Startup Fixture</title></movie>')
    for p in media.iterdir():
        p.chmod(0o444)
    media.chmod(0o555)
    return media, path


def start_server(arm, trial, port):
    if arm == 'jellyfin':
        conf = trial / 'config'
        conf.mkdir()
        (conf / 'network.xml').write_text(
            f'<NetworkConfiguration><InternalHttpPort>{port}</InternalHttpPort><EnableIPv6>false</EnableIPv6>'
            '<EnableRemoteAccess>false</EnableRemoteAccess><AutoDiscovery>false</AutoDiscovery><EnableUPnP>false</EnableUPnP>'
            '<LocalNetworkAddresses><string>127.0.0.1</string></LocalNetworkAddresses></NetworkConfiguration>')
        command = [str(JELLY / 'jellyfin'), '--datadir', str(trial / 'data'), '--configdir', str(conf), '--cachedir', str(trial / 'cache'),
                   '--logdir', str(trial / 'logs'), '--webdir', str(JELLY / 'jellyfin-web'), '--ffmpeg', '/usr/bin/ffmpeg', '--nonetchange']
        env = os.environ.copy()
    else:
        command = [str(RUST)]
        env = {**os.environ, 'OVL_LISTEN': f'127.0.0.1:{port}', 'OVL_DATA': str(trial / 'data'), 'OVL_ACCELERATOR': 'cpu'}
    log = (trial / 'process.log').open('wb')
    return subprocess.Popen(command, env=env, cwd=trial, stdout=log, stderr=subprocess.STDOUT, start_new_session=True), trial / 'process.log'


def run(arm, media):
    port = free_port()
    trial = ROOT / arm
    trial.mkdir()
    proc, log = start_server(arm, trial, port)
    result = {'binary_sha256': digest(JELLY / 'jellyfin' if arm == 'jellyfin' else RUST), 'scenarios': {}}
    try:
        for _ in range(240):
            if proc.poll() is not None:
                raise RuntimeError(f'{arm} exited {proc.returncode}')
            try:
                call(port, '/System/Info/Public' if arm == 'jellyfin' else '/api/v1/status')
                if arm == 'rust' or 'Core startup complete' in log.read_text(errors='replace'):
                    break
            except (OSError, RuntimeError, http.client.HTTPException):
                pass
            time.sleep(.25)
        else:
            raise RuntimeError(f'{arm} unavailable')
        password = secrets.token_urlsafe(24)
        if arm == 'jellyfin':
            call(port, '/Startup/User', headers={'Authorization': AUTH})
            call(port, '/Startup/User', 'POST', {'Name': 'benchuser', 'Password': password}, {'Authorization': AUTH})
            call(port, '/Startup/Complete', 'POST', headers={'Authorization': AUTH})
            auth = call(port, '/Users/AuthenticateByName', 'POST', {'Username': 'benchuser', 'Pw': password}, {'Authorization': AUTH})
            headers = {'Authorization': AUTH + f', Token="{auth["AccessToken"]}"'}
            user = auth['User']['Id']
            encoding = call(port, '/System/Configuration/encoding', headers=headers)
            encoding.update(EncoderPreset='veryfast', EncodingThreadCount=2, EnableThrottling=False, EnableSegmentDeletion=False,
                            HardwareAccelerationType='none', EnableSubtitleExtraction=False)
            call(port, '/System/Configuration/encoding', 'POST', encoding, headers)
            result['jellyfin_encoding'] = {k: call(port, '/System/Configuration/encoding', headers=headers).get(k) for k in
                                           ('EncoderPreset', 'EncodingThreadCount', 'EnableThrottling', 'HardwareAccelerationType', 'H264Crf')}
            query = urllib.parse.urlencode(dict(name='Fixture', collectionType='movies', paths=str(media), refreshLibrary='true'))
            options = dict(EnableRealtimeMonitor=False, EnableInternetProviders=False, SaveLocalMetadata=False, MetadataSavers=[],
                           EnableChapterImageExtraction=False, ExtractChapterImagesDuringLibraryScan=False, EnableTrickplayImageExtraction=False,
                           TypeOptions=[dict(Type='Movie', MetadataFetchers=[], ImageFetchers=[])])
            call(port, '/Library/VirtualFolders?' + query, 'POST', dict(LibraryOptions=options), headers)
            deadline = time.monotonic() + 900
            while time.monotonic() < deadline:
                tasks = call(port, '/ScheduledTasks', headers=headers)
                state = next(t['State'] for t in tasks if t['Key'] == 'RefreshLibrary')
                items = call(port, '/Items?' + urllib.parse.urlencode(dict(UserId=user, Recursive='true', IncludeItemTypes='Movie')), headers=headers)['Items']
                if items and state == 'Idle':
                    break
                time.sleep(5)
            else:
                raise RuntimeError('Jellyfin scan incomplete')
            item = items[0]['Id']
        else:
            headers = {'X-OVL-Request': '1'}
            call(port, '/api/v1/setup', 'POST', {'Name': 'benchuser', 'Password': password}, headers)
            auth = call(port, '/api/v1/login', 'POST', {'Name': 'benchuser', 'Password': password}, headers)
            headers['Authorization'] = 'Bearer ' + auth['token']
            library = call(port, '/api/v1/libraries', 'POST', {'Name': 'Fixture', 'Path': str(media), 'Kind': 'movie'}, headers)['id']
            call(port, '/api/v1/scan', 'POST', {}, headers)
            for _ in range(300):
                if not call(port, '/api/v1/scan', headers=headers)['running']:
                    break
                time.sleep(1)
            items = call(port, '/api/v1/items?' + urllib.parse.urlencode(dict(library=library)), headers=headers)
            item = items[0]['id']
        del password
        segment = trial / 'segment.ts'

        def direct():
            path = (f'/Videos/{item}/stream?static=true&MediaSourceId={item}' if arm == 'jellyfin' else f'/api/v1/items/{item}/file')
            # A server may answer a range with fewer bytes; continue like a client
            # until the first 2 MiB (or the whole, smaller, file) has arrived.
            source = media / 'Startup Fixture (2026).mp4'
            want = min(2097152, source.stat().st_size)
            body, requests = b'', 0
            while len(body) < want:
                _, part = call(port, path, headers=headers, raw=True, statuses=(206,),
                               extra={'Range': f'bytes={len(body)}-{want - 1}'})
                assert part, 'empty range response'
                body += part
                requests += 1
            assert len(body) == want, len(body)
            with open(source, 'rb') as f:
                assert f.read(want) == body
            return {'range_requests': requests, 'range_bytes': want}

        def transcode(start):
            if arm == 'rust':
                session = call(port, f'/api/v1/items/{item}/playback', 'POST', {
                    'Mode': 'transcode', 'Start': start, 'Capabilities': {'containers': ['mp4', 'hls'], 'video_codecs': ['h264'],
                                                                           'audio_codecs': ['aac'], 'max_audio_channels': 2, 'hdr': False}}, headers)
                sid = session['id']
                for _ in range(1200):
                    status, _ = call(port, session['url'], headers=headers, raw=True, statuses=(200, 404, 409, 425, 503))
                    if status == 200:
                        break
                    time.sleep(.025)
                playlist = call(port, f'/api/v1/playback/{sid}/v0.m3u8', headers=headers, raw=True)[1].decode()
                first = next(line for line in playlist.splitlines() if line and not line.startswith('#'))
                data = call(port, f'/api/v1/playback/{sid}/{first}', headers=headers, raw=True)[1]
                stop = lambda: call(port, f'/api/v1/playback/{sid}', 'DELETE', headers=headers)
            else:
                play = uuid.uuid4().hex
                params = dict(MediaSourceId=item, PlaySessionId=play, DeviceId='benchuser', VideoCodec='h264', AudioCodec='aac',
                              MaxWidth=1920, MaxHeight=1080, VideoBitrate=5_000_000, AudioBitrate=128_000, AudioChannels=2,
                              SegmentContainer='ts', SegmentLength=4, MinSegments=1, EnableAutoStreamCopy='false',
                              AllowVideoStreamCopy='false', AllowAudioStreamCopy='false', BreakOnNonKeyFrames='false')
                playlist = call(port, f'/Videos/{item}/main.m3u8?' + urllib.parse.urlencode(params), headers=headers, raw=True)[1].decode()
                uris = [line for line in playlist.splitlines() if line and not line.startswith('#')]
                index = int(start // 4)
                data = call(port, f'/Videos/{item}/{uris[index]}', headers=headers, raw=True)[1]
                stop = lambda: call(port, '/Videos/ActiveEncodings?' + urllib.parse.urlencode(dict(DeviceId='benchuser', PlaySessionId=play)),
                                    'DELETE', headers=headers)
            return {'bytes': len(data), 'data': data, 'stop': stop}

        scenarios = [('direct', direct, None), ('transcode-0', transcode, 0), ('transcode-60', transcode, 60)]
        for name, fn, start in scenarios:
            rows = []
            for rep in range(REPS):
                cpu0 = group_cpu(proc.pid)
                t0 = time.perf_counter()
                out = fn() if start is None else fn(start)
                elapsed = (time.perf_counter() - t0) * 1000
                row = {'rep': rep + 1, 'ms': round(elapsed, 1)}
                if start is None:
                    row.update(out)
                else:
                    segment.write_bytes(out['data'])
                    row['first_frame'] = validate(segment, start)
                    row['segment_bytes'] = out['bytes']
                    out['stop']()
                    time.sleep(2)  # let FFmpeg be killed and reaped so its CPU is attributed
                row['server_cpu_s'] = round(group_cpu(proc.pid) - cpu0, 2)
                row['rss_kib'] = rss_kib(proc.pid)
                rows.append(row)
                print(arm, name, json.dumps(row), flush=True)
            ms = sorted(r['ms'] for r in rows)
            result['scenarios'][name] = {'runs': rows, 'median_ms': ms[len(ms) // 2], 'min_ms': ms[0], 'max_ms': ms[-1]}
        return result
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
        result['exit_code'] = proc.returncode


def main():
    try:
        measure()
    finally:
        # Keep only the report; the fixture is read-only by design.
        for path in (ROOT / 'media', *(ROOT / 'media').glob('*')):
            if path.exists():
                path.chmod(0o755 if path.is_dir() else 0o644)
        for child in ROOT.iterdir():
            if child.name != 'report.json':
                shutil.rmtree(child, ignore_errors=True) if child.is_dir() else child.unlink()


def measure():
    media, fixture = prepare_fixture()
    report = {'scope': __doc__.strip().splitlines()[0], 'load_before': os.getloadavg(),
              'fixture_sha256': digest(fixture), 'repetitions': REPS, 'arms': {}}
    for arm in ('jellyfin', 'rust'):
        report['arms'][arm] = run(arm, media)
        report['load_after_' + arm] = os.getloadavg()
    (ROOT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print('REPORT', ROOT / 'report.json', flush=True)


if __name__ == '__main__':
    main()
