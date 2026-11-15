// Runs a Chromium client on this machine while an SSH worker owns the N100 app.
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { arch, platform, release, cpus, totalmem } from 'node:os';
import { createInterface } from 'node:readline';
import { parseArgs } from 'node:util';
import { playCohort } from './bench-sustained-playback.mjs';

const { values } = parseArgs({
  options: {
    host: { type: 'string' }, key: { type: 'string' },
    workspace: { type: 'string' }, listen: { type: 'string' },
    output: { type: 'string' }, unbuffered: { type: 'boolean' }, sustained: { type: 'string' }, background: { type: 'string' },
  },
});
if (!values.host || !/^[a-zA-Z0-9_.@-]+$/.test(values.host) || values.host.startsWith('-')
    || !values.key || !/^\/[a-zA-Z0-9_/-]+$/.test(values.workspace ?? '')
    || !/^\d{1,3}(\.\d{1,3}){3}$/.test(values.listen ?? '') || !values.output) {
  throw Error('Specify --host, --key, --workspace (remote source), --listen (IPv4) and --output');
}

if (values.sustained && (values.unbuffered || !['direct', 'remux', 'transcode'].includes(values.sustained))) {
  throw Error('Choose one sustained mode: direct, remux, or transcode');
}

if (values.background && (!values.sustained || !['idle', 'scan'].includes(values.background))) {
  throw Error('Choose --background idle or scan with a sustained mode');
}

const ssh = spawn('ssh', [
  '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes', '-o', 'IdentitiesOnly=yes',
  '-o', 'ConnectTimeout=5', '-i', values.key, values.host,
  `cd '${values.workspace}' && exec python3 scripts/lan_playback_worker.py --listen '${values.listen}'${values.unbuffered ? ' --unbuffered' : ''}${values.sustained ? ' --sustained' : ''}${values.background ? ' --background ' + values.background : ''}`,
], { stdio: ['pipe', 'pipe', 'pipe'] });
ssh.stdin.on('error', () => {}); // If the worker exits, the close result is authoritative.
let stderr = '';
ssh.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-8192); });
const closed = new Promise((resolve, reject) => {
  ssh.once('error', reject);
  ssh.once('close', (code, signal) => resolve({ code, signal }));
});
closed.catch(() => {}); // The first worker event also observes spawn failures.
const lines = createInterface({ input: ssh.stdout })[Symbol.asyncIterator]();

async function deadline(promise, ms, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(Error(label)), ms);
    })]);
  } finally {
    clearTimeout(timer);
  }
}
async function event(timeout = 60000) {
  const line = await deadline(lines.next(), timeout, 'worker response timed out');
  if (line.done) throw Error(`SSH worker stopped: ${stderr}`);
  return JSON.parse(line.value);
}
async function ready(origin) {
  const expires = Date.now() + 10000;
  while (Date.now() < expires) {
    try {
      const response = await fetch(origin + '/api/v1/status', { signal: AbortSignal.timeout(1000) });
      if (response.ok) return;
    } catch { /* Server may still be binding the test port. */ }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error('N100 server did not answer over the direct connection');
}
async function observeBackground(origin, token, library) {
  const startEpoch = performance.timeOrigin + performance.now();
  ssh.stdin.write('scan-snapshot\n');
  const before = await event();
  const start = performance.now();
  async function load(search) {
    const latencies = [];
    const errors = {};
    let nextAt = start;
    for (let i = 0; i < 100; i++) {
      const remaining = nextAt - performance.now();
      if (remaining > 0) await new Promise(resolve => setTimeout(resolve, Math.ceil(remaining)));
      const offset = i % 10 * 100;
      const url = new URL('/api/v1/items', origin);
      url.search = new URLSearchParams({ library, limit: 100, offset, ...(search ? { q: 'Scan' } : {}) });
      const began = performance.now();
      nextAt = began + 100;
      try {
        const response = await fetch(url, { headers: { Authorization: 'Bearer ' + token },
          signal: AbortSignal.timeout(1000) });
        if (!response.ok) {
          await response.body?.cancel();
          errors['HTTP' + response.status] = (errors['HTTP' + response.status] ?? 0) + 1;
          continue;
        }
        const rows = await response.json();
        if (!Array.isArray(rows) || rows.length !== 100 || rows.some((row, index) =>
          row.library_id !== library || row.title !== 'Scan ' + String(offset + index + 1).padStart(6, '0'))) {
          throw Error('wrong page');
        }
        latencies.push(performance.now() - began);
      } catch (error) {
        const kind = error.message === 'wrong page' ? 'WrongPage' : error.name;
        errors[kind] = (errors[kind] ?? 0) + 1;
      }
    }
    latencies.sort((a, b) => a - b);
    const pct = p => latencies.length ? latencies[Math.floor((latencies.length - 1) * p)] : null;
    return { requests: 100, errors, successful: latencies.length, elapsed_ms: performance.now() - start,
      p50_ms: pct(.5), p95_ms: pct(.95), p99_ms: pct(.99) };
  }
  const [browse, search] = await Promise.all([load(false), load(true)]);
  ssh.stdin.write('scan-snapshot\n');
  const after = await event();
  const completed = after.completed_ms.slice(before.completed_ms.length);
  if (before.incomplete || after.incomplete || (values.background === 'scan' ? completed.length < 1 : completed.length !== 0)) {
    throw Error('scheduled scan observations did not match the background workload');
  }
  return { start_epoch_ms: startEpoch, end_epoch_ms: performance.timeOrigin + performance.now(),
    completed_scan_ms: completed, browse, search };
}
const report = {
  recorded_utc: new Date().toISOString(),
  scope: values.sustained ? 'Two simultaneous Chromium playback windows over LAN; generated local-storage fixture; no SMB, TLS or hardware transcode'
    : 'Direct HTTP, generated local-storage H.264/AAC MP4; decoded browser startup and seek; no SMB, TLS, or sustained concurrency',
  client: { platform: platform(), arch: arch(), kernel: release(), cpu: cpus()[0]?.model, ram_bytes: totalmem() },
  client_sha256: createHash('sha256').update(await readFile(new URL(import.meta.url))).digest('hex'),
  complete: false, results: [],
};
let browser;
try {
  const { origin, token, item, server } = await event(120000);
  if (new URL(origin).hostname !== values.listen) throw Error('unexpected worker address');
  report.server = server;
  report.transport = values.listen.startsWith('127.') ? 'loopback' : 'direct LAN HTTP; SSH control only';
  await ready(origin);
  browser = await chromium.launch({ channel: 'chromium', timeout: 15000 });
  report.client.browser = browser.version();

  if (values.sustained) {
    report.mode = values.sustained;
    if (values.background) report.background = values.background;
    report.sustained_client_sha256 = createHash('sha256')
      .update(await readFile(new URL('./bench-sustained-playback.mjs', import.meta.url))).digest('hex');
    for (let repetition = 0; repetition < 5; repetition++) {
      if (values.background === 'scan') {
        ssh.stdin.write('scan-snapshot\n');
        const snapshot = await event();
        // Start the cohort around six seconds before the next supported tick.
        const delay = (120000 - snapshot.uptime_ms % 60000 - 6000) % 60000;
        await new Promise(resolve => setTimeout(resolve, delay));
      }
      const result = await playCohort({ browser, origin, token, item, mode: values.sustained, server, deadline,
        observeWindow: values.background ? () => observeBackground(origin, token, server.scan_library) : undefined });
      report.results.push({ repetition, ...result });
      await writeFile(values.output, JSON.stringify(report, null, 2) + '\n');
    }
  } else {
  for (let repetition = 0; repetition < 5; repetition++) {
    const context = await browser.newContext({ extraHTTPHeaders: { Authorization: 'Bearer ' + token } });
    try {
      const page = await context.newPage();
      const isMediaRange = r => new URL(r.url()).origin === origin
        && new URL(r.url()).pathname === `/api/v1/items/${item}/file` && r.status() === 206;
      const mediaRanges = [];
      page.on('response', r => {
        if (isMediaRange(r) && mediaRanges.length < 32) mediaRanges.push(r);
      });
      const range = page.waitForResponse(isMediaRange, { timeout: 15000 });
      range.catch(() => {});
      await page.goto(origin + "/", { timeout: 10000 });
      const timings = await deadline(page.evaluate(async ({ item, seekTarget, mustFetch }) => {
        const video = document.querySelector('video');
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        function frame(start, minMediaTime) {
          return new Promise((resolve, reject) => {
            let id;
            const timer = setTimeout(() => {
              video.cancelVideoFrameCallback(id);
              reject(Error('decoded frame timed out'));
            }, 10000);
            function inspect(now, metadata) {
              if (metadata.mediaTime < minMediaTime) {
                id = video.requestVideoFrameCallback(inspect);
                return;
              }
              clearTimeout(timer);
              ctx.drawImage(video, 0, 0, 1, 1);
              resolve({ milliseconds: now - start, media_seconds: metadata.mediaTime,
                pixel: Array.from(ctx.getImageData(0, 0, 1, 1).data), width: metadata.width, height: metadata.height });
            }
            id = video.requestVideoFrameCallback(inspect);
          });
        }
        const start = performance.now();
        const response = await fetch(`/api/v1/items/${item}/playback`, {
          method: 'POST', signal: AbortSignal.timeout(8000),
          headers: { 'Content-Type': 'application/json', 'X-OVL-Request': '1' },
          body: JSON.stringify({ Mode: 'direct' }),
        });
        if (!response.ok) throw Error('direct playback request failed: ' + response.status);
        const playback = await response.json();
        if (playback.mode !== 'direct' || playback.url !== `/api/v1/items/${item}/file`) throw Error('unexpected direct URL');
        const setup_ms = performance.now() - start;
        const initial = frame(start, 0);
        initial.catch(() => {});
        video.src = playback.url;
        await video.play();
        const startup = await initial;
        const seekBuffered = Array.from({ length: video.buffered.length }, (_, i) =>
          video.buffered.start(i) <= seekTarget && video.buffered.end(i) >= seekTarget).some(Boolean);
        if (mustFetch && seekBuffered) throw Error('target was already buffered');
        const seekStart = performance.now();
        const afterSeek = frame(seekStart, seekTarget - .05);
        afterSeek.catch(() => {});
        video.currentTime = seekTarget;
        const seek = await afterSeek;
        video.pause();
        return { setup_ms, startup, seek, seek_target_buffered: seekBuffered,
          seek_start_epoch_ms: performance.timeOrigin + seekStart };
      }, { item, seekTarget: server.seek_target_seconds, mustFetch: values.unbuffered }), 22000, "browser playback trial timed out");
      const mediaResponse = await range;
      const peer = await mediaResponse.serverAddr();
      const contentRange = mediaResponse.headers()['content-range'];
      if (!contentRange?.endsWith('/' + server.fixture_bytes)) throw Error('media response size does not match the fixture');
      if (peer?.ipAddress !== values.listen || peer.port !== Number(new URL(origin).port)) throw Error('media was not served by the expected direct peer');
      const { startup, seek } = timings;
      const [r0, g0, b0] = startup.pixel;
      const [r1, g1, b1] = seek.pixel;
      if (!(r0 > 180 && g0 < 50 && b0 < 50 && startup.media_seconds < .5
          && b1 > 180 && g1 < 50 && r1 < 50 && seek.media_seconds < server.seek_target_seconds + .5
          && [startup, seek].every(f => f.width === 640 && f.height === 360))) {
        throw Error('unexpected decoded frame, dimensions, or media time');
      }
      let seekRange;
      if (values.unbuffered) {
        for (const candidate of mediaRanges) {
          const contentRange = candidate.headers()['content-range'];
          const match = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(contentRange ?? '');
          const request = candidate.request();
          const { startTime: started, responseStart } = request.timing();
          const responseAt = started + responseStart;
          if (!match || Number(match[3]) !== server.fixture_bytes
              || Number(match[1]) < server.fixture_bytes * .75
              || Number(match[1]) > server.seek_keyframe.position
              || Number(match[2]) < server.seek_keyframe.position
              || Number(match[2]) >= server.fixture_bytes
              || !request.headers().range?.startsWith(`bytes=${match[1]}-`)
              || started < timings.seek_start_epoch_ms - 1
              || responseStart < 0 || responseAt > timings.seek_start_epoch_ms + timings.seek.milliseconds + 1) continue;
          const peer = await candidate.serverAddr();
          if (peer?.ipAddress !== values.listen || peer.port !== Number(new URL(origin).port)) continue;
          seekRange = { content_range: contentRange, request_range: request.headers().range,
            request_start_after_seek_ms: started - timings.seek_start_epoch_ms,
            response_start_after_seek_ms: responseAt - timings.seek_start_epoch_ms, peer_verified: true };
          break;
        }
        if (!seekRange) throw Error('no new verified media range for unbuffered seek');
      }
      report.results.push({ repetition, status: 206, content_range: contentRange, peer_verified: true, ...timings, ...(seekRange ? { seek_range: seekRange } : {}) });
      await writeFile(values.output, JSON.stringify(report, null, 2) + '\n');
    } finally {
      await context.close();
    }
  }
  }
  ssh.stdin.end('done\n');
  report.cleanup = await event();
  const exit = await deadline(closed, 20000, 'worker shutdown timed out');
  if (exit.code !== 0 || report.cleanup.shutdown_exit !== 0) throw Error('worker cleanup failed');
  report.complete = true;
  if (values.background && report.results.some(row =>
    [row.background.browse, row.background.search].some(workload => workload.successful !== workload.requests))) {
    process.exitCode = 1;
  }
} catch (error) {
  report.error = String(error.message).slice(0, 1024);
  process.exitCode = 1;
} finally {
  ssh.stdin.end();
  if (browser) {
    try {
      await deadline(browser.close(), 10000, 'browser shutdown timed out');
    } catch (error) {
      report.complete = false;
      report.error ??= String(error.message).slice(0, 1024);
      process.exitCode = 1;
    }
  }
  try {
    await deadline(closed, 20000, 'worker cleanup timed out');
  } catch {
    ssh.kill('SIGTERM');
    await deadline(closed, 5000, 'SSH process did not exit').catch(() => ssh.kill('SIGKILL'));
  }
  await writeFile(values.output, JSON.stringify(report, null, 2) + '\n');
}
