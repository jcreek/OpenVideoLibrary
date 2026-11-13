"""Paired native playback-startup diagnostic v2: Jellyfin 12.4 and OVL on the N100.

Warm loopback, one request at a time, generated local fixture, shared host.
Changes from v1 (after independent review):

* both servers run together and arms are interleaved per round, alternating
  which goes first, after one discarded warm-up per scenario and arm;
* the live FFmpeg command line of each transcode is captured from /proc;
* server CPU (server process group plus reaped children) is sampled both when
  the first segment has been received and after the session stops;
* every first segment's per-stream packet span and bitrate are recorded, and
  its first decoded frame (bars encode the frame number) is checked.

Results are diagnostics, not LAN first-frame acceptance.
"""
import hashlib, http.client, json, os, pathlib, secrets, shutil, signal, socket, subprocess, tempfile, time, urllib.parse, uuid

HOME = pathlib.Path.home()
JELLY = HOME / 'ovl-jellyfin-12.4-hc7ns1a3/jellyfin'
RUST = HOME / 'ovl-rust-acceptance/source/target/release/ovl-server'
ROUNDS = 8
ROOT = pathlib.Path(tempfile.mkdtemp(prefix='ovl-playback-startup-', dir=HOME))
AUTH = 'MediaBrowser Client="OVL benchmark", Device="N100", DeviceId="ovlbench", Version="1.0"'
FIXTURE_NAME = 'Startup Fixture (2026).mp4'


def cancel(signum, frame):
    raise SystemExit(128 + signum)


# An SSH disconnect or Ctrl-C must still stop both trial servers.
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


def group_members(pid):
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
        except OSError:
            continue
        if int(fields[2]) == pid:
            yield entry, fields


def group_cpu(pid):
    """utime+stime of the server's process group plus the server's reaped children."""
    tick = os.sysconf('SC_CLK_TCK')
    total = 0
    for entry, fields in group_members(pid):
        total += int(fields[11]) + int(fields[12])
        if int(entry.name) == pid:
            total += int(fields[13]) + int(fields[14])
    return total / tick


def ffmpeg_command(pid):
    for entry, _ in group_members(pid):
        try:
            argv = (entry / 'cmdline').read_bytes().split(b'\0')
        except OSError:
            continue
        if argv and os.path.basename(argv[0].decode(errors='replace')) == 'ffmpeg':
            return [a.decode(errors='replace') for a in argv if a]
    return None


def wait_no_ffmpeg(pid, timeout=10):
    deadline = time.monotonic() + timeout
    while ffmpeg_command(pid) and time.monotonic() < deadline:
        time.sleep(.05)
    return ffmpeg_command(pid) is None


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
    probe = json.loads(subprocess.run(['ffprobe', '-v', 'error', '-show_entries', 'stream=index,codec_type,codec_name,height',
                                       '-of', 'json', str(segment)], capture_output=True, check=True, timeout=60).stdout)
    kinds = {(s['codec_type'], s['codec_name']) for s in probe['streams']}
    heights = {s.get('height') for s in probe['streams'] if s['codec_type'] == 'video'}
    assert ('video', 'h264') in kinds and ('audio', 'aac') in kinds and heights == {1080}, (kinds, heights)
    spans = {}
    for stream in probe['streams']:
        packets = json.loads(subprocess.run(['ffprobe', '-v', 'error', '-select_streams', str(stream['index']), '-show_entries',
                                             'packet=pts_time,duration_time,size', '-of', 'json', str(segment)],
                                            capture_output=True, check=True, timeout=60).stdout)['packets']
        pts = [float(p['pts_time']) for p in packets if p.get('pts_time') not in (None, 'N/A')]
        timed = [p for p in packets if p.get('pts_time') not in (None, 'N/A')]
        last = max(timed, key=lambda p: float(p['pts_time']))
        span = max(pts) - min(pts) + float(last.get('duration_time', 0) or 0)
        size = sum(int(p['size']) for p in packets)
        spans[stream['codec_type']] = {'seconds': round(span, 3), 'kbps': round(size * 8 / span / 1000, 1) if span else None}
    frame = frame_number(segment)
    assert abs(frame - start * 24) <= 2, ('first frame', frame, start)
    return frame, spans


def prepare_fixture():
    media = ROOT / 'media'
    media.mkdir()
    path = media / FIXTURE_NAME
    subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', f'color=black:size=1920x1080:rate=24,{BARS},format=yuv420p',
                    '-f', 'lavfi', '-i', "aevalsrc='0.3*lt(mod(t\\,1)\\,0.1)*sin(2*PI*800*t)':s=48000:c=stereo",
                    '-t', '120', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '20', '-g', '48', '-threads', '2',
                    '-c:a', 'aac', '-b:a', '192k', '-movflags', '+faststart', str(path)], check=True, timeout=1800)
    (media / 'Startup Fixture (2026).nfo').write_text('<movie><title>Startup Fixture</title></movie>')
    for p in media.iterdir():
        p.chmod(0o444)
    media.chmod(0o555)
    return media, path


def setup(arm, media, contexts):
    port = free_port()
    trial = ROOT / arm
    trial.mkdir()
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
    log = trial / 'process.log'
    proc = subprocess.Popen(command, env=env, cwd=trial, stdout=log.open('wb'), stderr=subprocess.STDOUT, start_new_session=True)
    ctx = {'arm': arm, 'port': port, 'proc': proc, 'trial': trial, 'media': media,
           'result': {'binary_sha256': digest(command[0]), 'scenarios': {}, 'ffmpeg_command': {}}}
    contexts.append(ctx)  # registered before any wait so an interrupt still stops it
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
        call(port, '/Startup/User', 'POST', {'Name': 'ovlbench', 'Password': password}, {'Authorization': AUTH})
        call(port, '/Startup/Complete', 'POST', headers={'Authorization': AUTH})
        auth = call(port, '/Users/AuthenticateByName', 'POST', {'Username': 'ovlbench', 'Pw': password}, {'Authorization': AUTH})
        headers = {'Authorization': AUTH + f', Token="{auth["AccessToken"]}"'}
        user = auth['User']['Id']
        encoding = call(port, '/System/Configuration/encoding', headers=headers)
        encoding.update(EncoderPreset='veryfast', EncodingThreadCount=2, EnableThrottling=False, EnableSegmentDeletion=False,
                        HardwareAccelerationType='none', EnableSubtitleExtraction=False)
        call(port, '/System/Configuration/encoding', 'POST', encoding, headers)
        ctx['result']['jellyfin_encoding'] = {k: call(port, '/System/Configuration/encoding', headers=headers).get(k) for k in
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
        ctx['item'] = items[0]['Id']
    else:
        headers = {'X-OVL-Request': '1'}
        call(port, '/api/v1/setup', 'POST', {'Name': 'ovlbench', 'Password': password}, headers)
        auth = call(port, '/api/v1/login', 'POST', {'Name': 'ovlbench', 'Password': password}, headers)
        headers['Authorization'] = 'Bearer ' + auth['token']
        library = call(port, '/api/v1/libraries', 'POST', {'Name': 'Fixture', 'Path': str(media), 'Kind': 'movie'}, headers)['id']
        call(port, '/api/v1/scan', 'POST', {}, headers)
        for _ in range(300):
            if not call(port, '/api/v1/scan', headers=headers)['running']:
                break
            time.sleep(1)
        else:
            raise RuntimeError('OVL scan incomplete')
        ctx['item'] = call(port, '/api/v1/items?' + urllib.parse.urlencode(dict(library=library)), headers=headers)[0]['id']
    ctx['headers'] = headers
    return ctx


def direct(ctx):
    port, item, headers = ctx['port'], ctx['item'], ctx['headers']
    path = (f'/Videos/{item}/stream?static=true&MediaSourceId={item}' if ctx['arm'] == 'jellyfin' else f'/api/v1/items/{item}/file')
    source = ctx['media'] / FIXTURE_NAME
    want = min(2097152, source.stat().st_size)
    body, requests = b'', 0
    while len(body) < want:  # a server may answer with fewer bytes than requested
        _, part = call(port, path, headers=headers, raw=True, statuses=(206,), extra={'Range': f'bytes={len(body)}-{want - 1}'})
        assert part, 'empty range response'
        body += part
        requests += 1
    with open(source, 'rb') as f:
        assert f.read(want) == body
    return {'range_requests': requests, 'range_bytes': want}


def transcode(ctx, start):
    port, item, headers, pid = ctx['port'], ctx['item'], ctx['headers'], ctx['proc'].pid
    if ctx['arm'] == 'rust':
        session = call(port, f'/api/v1/items/{item}/playback', 'POST', {
            'Mode': 'transcode', 'Start': start, 'Capabilities': {'containers': ['mp4', 'hls'], 'video_codecs': ['h264'],
                                                                   'audio_codecs': ['aac'], 'max_audio_channels': 2, 'hdr': False}}, headers)
        sid = session['id']
        for _ in range(1200):
            status, _ = call(port, session['url'], headers=headers, raw=True, statuses=(200, 404, 409, 425, 503))
            if status == 200:
                break
            time.sleep(.025)
        else:
            raise RuntimeError('OVL playlist not published')
        playlist = call(port, f'/api/v1/playback/{sid}/v0.m3u8', headers=headers, raw=True)[1].decode()
        first = next(line for line in playlist.splitlines() if line and not line.startswith('#'))
        command = ffmpeg_command(pid)
        data = call(port, f'/api/v1/playback/{sid}/{first}', headers=headers, raw=True)[1]
        stop = lambda: call(port, f'/api/v1/playback/{sid}', 'DELETE', headers=headers)
    else:
        play = uuid.uuid4().hex
        params = dict(MediaSourceId=item, PlaySessionId=play, DeviceId='ovlbench', VideoCodec='h264', AudioCodec='aac',
                      MaxWidth=1920, MaxHeight=1080, VideoBitrate=5_000_000, AudioBitrate=128_000, AudioChannels=2,
                      SegmentContainer='ts', SegmentLength=4, MinSegments=1, EnableAutoStreamCopy='false',
                      AllowVideoStreamCopy='false', AllowAudioStreamCopy='false', BreakOnNonKeyFrames='false')
        playlist = call(port, f'/Videos/{item}/main.m3u8?' + urllib.parse.urlencode(params), headers=headers, raw=True)[1].decode()
        uris = [line for line in playlist.splitlines() if line and not line.startswith('#')]
        data = call(port, f'/Videos/{item}/{uris[int(start // 4)]}', headers=headers, raw=True)[1]
        command = ffmpeg_command(pid)
        stop = lambda: call(port, '/Videos/ActiveEncodings?' + urllib.parse.urlencode(dict(DeviceId='ovlbench', PlaySessionId=play)),
                            'DELETE', headers=headers)
    return data, command, stop


def measure(ctx, name, start, keep):
    pid = ctx['proc'].pid
    cpu0 = group_cpu(pid)
    t0 = time.perf_counter()
    row = {}
    if start is None:
        row.update(direct(ctx))
        row['ms'] = round((time.perf_counter() - t0) * 1000, 1)
    else:
        data, command, stop = transcode(ctx, start)
        row['ms'] = round((time.perf_counter() - t0) * 1000, 1)
        row['cpu_at_first_segment_s'] = round(group_cpu(pid) - cpu0, 2)
        segment = ctx['trial'] / 'segment.ts'
        segment.write_bytes(data)
        row['first_frame'], row['streams'] = validate(segment, start)
        row['segment_bytes'] = len(data)
        stop()
        row['ffmpeg_reaped'] = wait_no_ffmpeg(pid)
        if keep and command and name not in ctx['result']['ffmpeg_command']:
            ctx['result']['ffmpeg_command'][name] = [a.replace(str(ROOT), '$ROOT') for a in command]
    row['session_cpu_s'] = round(group_cpu(pid) - cpu0, 2)
    row['rss_kib'] = rss_kib(pid)
    return row


def main():
    contexts = []
    try:
        media, fixture = prepare_fixture()
        report = {'scope': __doc__.strip().splitlines()[0], 'load_before': os.getloadavg(), 'fixture_sha256': digest(fixture),
                  'fixture_bytes': fixture.stat().st_size, 'rounds': ROUNDS,
                  'ffmpeg_version': subprocess.run(['ffmpeg', '-version'], capture_output=True, text=True).stdout.splitlines()[0],
                  'arms': {}}
        for arm in ('jellyfin', 'rust'):
            setup(arm, media, contexts)
        scenarios = [('direct', None), ('transcode-0', 0), ('transcode-60', 60)]
        for ctx in contexts:  # discarded warm-up per scenario
            for name, start in scenarios:
                measure(ctx, name, start, keep=False)
        for rnd in range(ROUNDS):
            order = contexts if rnd % 2 == 0 else contexts[::-1]
            for name, start in scenarios:
                for ctx in order:
                    row = measure(ctx, name, start, keep=True)
                    row['round'] = rnd + 1
                    ctx['result']['scenarios'].setdefault(name, []).append(row)
                    print(ctx['arm'], name, json.dumps(row), flush=True)
        for ctx in contexts:
            for name, rows in ctx['result']['scenarios'].items():
                ms = sorted(r['ms'] for r in rows)
                summary = {'median_ms': ms[len(ms) // 2], 'min_ms': ms[0], 'max_ms': ms[-1]}
                if 'cpu_at_first_segment_s' in rows[0]:
                    cpu = sorted(r['cpu_at_first_segment_s'] for r in rows)
                    summary['median_cpu_at_first_segment_s'] = cpu[len(cpu) // 2]
                ctx['result'].setdefault('summary', {})[name] = summary
            report['arms'][ctx['arm']] = ctx['result']
        report['load_after'] = os.getloadavg()
        stops = []
        for ctx in contexts:
            os.killpg(ctx['proc'].pid, signal.SIGTERM)
            stops.append(ctx)
        for ctx in stops:
            ctx['result']['exit_code'] = ctx['proc'].wait(timeout=30)
        (ROOT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        print('REPORT', ROOT / 'report.json', flush=True)
    finally:
        for ctx in contexts:
            if ctx['proc'].poll() is None:
                os.killpg(ctx['proc'].pid, signal.SIGTERM)
                try:
                    ctx['proc'].wait(timeout=30)
                except subprocess.TimeoutExpired:
                    os.killpg(ctx['proc'].pid, signal.SIGKILL)
                    ctx['proc'].wait()
        # Keep only the report; the fixture is read-only by design.
        media = ROOT / 'media'
        if media.exists():
            media.chmod(0o755)
            for p in media.iterdir():
                p.chmod(0o644)
        # Keep only the report; after a failure keep process logs as evidence.
        failed = not (ROOT / 'report.json').exists()
        for child in ROOT.iterdir():
            if child.name == 'report.json':
                continue
            if failed and child.is_dir() and child.name in ('jellyfin', 'rust'):
                for inner in child.iterdir():
                    if inner.name not in ('process.log', 'logs'):
                        shutil.rmtree(inner, ignore_errors=True) if inner.is_dir() else inner.unlink()
                continue
            shutil.rmtree(child, ignore_errors=True) if child.is_dir() else child.unlink()


if __name__ == '__main__':
    main()
