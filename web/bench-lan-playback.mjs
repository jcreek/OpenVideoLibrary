// Runs a Chromium client on this machine while an SSH worker owns the N100 app.
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { arch, platform, release, cpus, totalmem } from 'node:os';
import { createInterface } from 'node:readline';
import { parseArgs } from 'node:util';

const { values } = parseArgs({
  options: {
    host: { type: 'string' }, key: { type: 'string' },
    workspace: { type: 'string' }, listen: { type: 'string' },
    output: { type: 'string' },
  },
});
if (!values.host || !/^[a-zA-Z0-9_.@-]+$/.test(values.host) || values.host.startsWith('-')
    || !values.key || !/^\/[a-zA-Z0-9_/-]+$/.test(values.workspace ?? '')
    || !/^\d{1,3}(\.\d{1,3}){3}$/.test(values.listen ?? '') || !values.output) {
  throw Error('Specify --host, --key, --workspace (remote source), --listen (IPv4) and --output');
}

const ssh = spawn('ssh', [
  '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes', '-o', 'IdentitiesOnly=yes',
  '-o', 'ConnectTimeout=5', '-i', values.key, values.host,
  `cd '${values.workspace}' && exec python3 scripts/lan_playback_worker.py --listen '${values.listen}'`,
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
async function event() {
  const line = await deadline(lines.next(), 60000, 'worker response timed out');
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
const report = {
  recorded_utc: new Date().toISOString(),
  scope: 'Direct HTTP, generated local-storage H.264/AAC MP4; decoded browser startup and seek; no SMB, TLS, or sustained concurrency',
  client: { platform: platform(), arch: arch(), kernel: release(), cpu: cpus()[0]?.model, ram_bytes: totalmem() },
  client_sha256: createHash('sha256').update(await readFile(new URL(import.meta.url))).digest('hex'),
  complete: false, results: [],
};
let browser;
try {
  const { origin, token, item, server } = await event();
  if (new URL(origin).hostname !== values.listen) throw Error('unexpected worker address');
  report.server = server;
  report.transport = values.listen.startsWith('127.') ? 'loopback' : 'direct LAN HTTP; SSH control only';
  await ready(origin);
  browser = await chromium.launch({ channel: 'chromium', timeout: 15000 });
  report.client.browser = browser.version();

  for (let repetition = 0; repetition < 5; repetition++) {
    const context = await browser.newContext({ extraHTTPHeaders: { Authorization: 'Bearer ' + token } });
    try {
      const page = await context.newPage();
      const range = page.waitForResponse(r => new URL(r.url()).pathname === `/api/v1/items/${item}/file`
        && r.status() === 206, { timeout: 15000 });
      range.catch(() => {});
      await page.goto(origin + "/", { timeout: 10000 });
      const timings = await deadline(page.evaluate(async ({ item }) => {
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
          video.buffered.start(i) <= 3 && video.buffered.end(i) >= 3).some(Boolean);
        const afterSeek = frame(performance.now(), 2.95);
        afterSeek.catch(() => {});
        video.currentTime = 3;
        const seek = await afterSeek;
        video.pause();
        return { setup_ms, startup, seek, seek_target_buffered: seekBuffered };
      }, { item }), 22000, "browser playback trial timed out");
      const mediaResponse = await range;
      const peer = await mediaResponse.serverAddr();
      const contentRange = mediaResponse.headers()['content-range'];
      if (!contentRange?.endsWith('/' + server.fixture_bytes)) throw Error('media response size does not match the fixture');
      if (peer?.ipAddress !== values.listen || peer.port !== Number(new URL(origin).port)) throw Error('media was not served by the expected direct peer');
      const { startup, seek } = timings;
      const [r0, g0, b0] = startup.pixel;
      const [r1, g1, b1] = seek.pixel;
      if (!(r0 > 180 && g0 < 50 && b0 < 50 && startup.media_seconds < .5
          && b1 > 180 && g1 < 50 && r1 < 50 && seek.media_seconds < 3.5
          && [startup, seek].every(f => f.width === 640 && f.height === 360))) {
        throw Error('unexpected decoded frame, dimensions, or media time');
      }
      report.results.push({ repetition, status: 206, content_range: contentRange, peer_verified: true, ...timings });
      await writeFile(values.output, JSON.stringify(report, null, 2) + '\n');
    } finally {
      await context.close();
    }
  }
  ssh.stdin.end('done\n');
  report.cleanup = await event();
  const exit = await deadline(closed, 20000, 'worker shutdown timed out');
  if (exit.code !== 0 || report.cleanup.shutdown_exit !== 0) throw Error('worker cleanup failed');
  report.complete = true;
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
