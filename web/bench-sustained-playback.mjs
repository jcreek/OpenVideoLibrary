// Two concurrent, separately authenticated browser contexts on the LAN client.
export async function playCohort({ browser, origin, token, item, mode, server, deadline }) {
  const contexts = [];
  try {
    for (let i = 0; i < 2; i++) {
      contexts.push(await browser.newContext({ extraHTTPHeaders: { Authorization: 'Bearer ' + token } }));
    }
    const prepared = await Promise.allSettled(contexts.map(async context => {
      const page = await context.newPage();
      await page.goto(origin + '/', { timeout: 10000 });
      if (mode !== 'direct') {
        await deadline(page.addScriptTag({ url: origin + '/bench-hls.js' }), 10000, 'HLS client load timed out');
      }
      return page;
    }));
    const failedPage = prepared.find(p => p.status === 'rejected');
    if (failedPage) throw failedPage.reason;

    const completed = await Promise.allSettled(prepared.map(async ({ value: page }) => {
      const received = [];
      page.on('response', response => {
        const url = new URL(response.url());
        if (url.origin === origin && received.length < 32
            && (response.status() === 206 && url.pathname === `/api/v1/items/${item}/file`
                || response.status() === 200 && /^\/api\/v1\/playback\/[^/]+\/v0-\d+\.ts$/.test(url.pathname))) {
          received.push(response);
        }
      });
      const metrics = await deadline(page.evaluate(async ({ item, mode }) => {
        const video = document.querySelector('video');
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        let hls, playback, stop, timer, callback;
        let waitingSince = null, waitingEvents = 0, waitingMs = 0;
        let startFrame, lastFrame, frameCount = 0, maxGap = 0, gapExcess = 0;
        let rejectPlayback;
        const failed = new Promise((_, reject) => { rejectPlayback = reject; });
        failed.catch(() => {});
        video.addEventListener('error', () => rejectPlayback(Error('video decode failed')));
        video.addEventListener('waiting', () => {
          if (startFrame && waitingSince === null) {
            waitingSince = performance.now();
            waitingEvents++;
          }
        });
        video.addEventListener('playing', () => {
          if (waitingSince !== null) {
            waitingMs += performance.now() - waitingSince;
            waitingSince = null;
          }
        });
        const started = performance.now();
        try {
          const response = await fetch(`/api/v1/items/${item}/playback`, {
            method: 'POST', signal: AbortSignal.timeout(8000),
            headers: { 'Content-Type': 'application/json', 'X-OVL-Request': '1' },
            body: JSON.stringify({ Mode: mode }),
          });
          if (!response.ok) throw Error('playback setup failed: ' + response.status);
          playback = await response.json();
          if (playback.mode !== mode || (mode === 'direct'
            ? playback.url !== `/api/v1/items/${item}/file`
            : !/^[A-Za-z0-9_-]+$/.test(playback.id ?? '') ||
              playback.url !== `/api/v1/playback/${playback.id}/index.m3u8`)) {
            throw Error('unexpected playback route');
          }
          const setupMs = performance.now() - started;
          const watch = new Promise(resolve => { stop = resolve; });
          const first = new Promise(resolve => {
            function frame(now, metadata) {
              const row = { now, media_seconds: metadata.mediaTime, width: metadata.width, height: metadata.height };
              if (!startFrame) {
                ctx.drawImage(video, 0, 0, 1, 1);
                startFrame = { ...row, pixel: Array.from(ctx.getImageData(0, 0, 1, 1).data) };
                timer = setTimeout(stop, 20000);
                resolve();
              }
              if (lastFrame) {
                const gap = now - lastFrame.now;
                maxGap = Math.max(maxGap, gap);
                gapExcess += Math.max(0, gap - 250);
              }
              lastFrame = row;
              frameCount++;
              callback = video.requestVideoFrameCallback(frame);
            }
            callback = video.requestVideoFrameCallback(frame);
          });
          if (mode === 'direct') {
            video.src = playback.url;
          } else {
            if (!window.Hls?.isSupported()) throw Error('HLS MSE unavailable');
            hls = new window.Hls({ maxBufferLength: 12 });
            hls.on(window.Hls.Events.ERROR, (_, data) => {
              if (data.fatal) rejectPlayback(Error('HLS failed: ' + data.details));
            });
            hls.attachMedia(video);
            hls.loadSource(playback.url);
          }
          await Promise.race([Promise.all([video.play(), first]), failed]);
          await Promise.race([watch, failed]);
          const ended = performance.now();
          if (waitingSince !== null) waitingMs += ended - waitingSince;
          return {
            setup_ms: setupMs, first_frame_ms: startFrame.now - started,
            first_frame: startFrame, last_frame: lastFrame,
            first_frame_epoch_ms: performance.timeOrigin + startFrame.now,
            window_end_epoch_ms: performance.timeOrigin + ended,
            window_ms: ended - startFrame.now,
            media_advance_seconds: lastFrame.media_seconds - startFrame.media_seconds,
            frames: frameCount, waiting_events: waitingEvents, waiting_ms: waitingMs,
            max_frame_gap_ms: Math.max(maxGap, ended - lastFrame.now),
            frame_gap_excess_ms: gapExcess + Math.max(0, ended - lastFrame.now - 250),
          };
        } finally {
          clearTimeout(timer);
          if (callback !== undefined) video.cancelVideoFrameCallback(callback);
          video.pause();
          hls?.destroy();
          video.removeAttribute('src');
          video.load();
          if (/^[A-Za-z0-9_-]+$/.test(playback?.id ?? '')) {
            const response = await fetch('/api/v1/playback/' + playback.id, {
              method: 'DELETE', signal: AbortSignal.timeout(10000), headers: { 'X-OVL-Request': '1' },
            });
            if (!response.ok) throw Error('playback cleanup failed: ' + response.status);
          }
        }
      }, { item, mode }), 45000, 'sustained playback timed out');

      const peer = await received[0]?.serverAddr();
      const endpoint = new URL(origin);
      if (peer?.ipAddress !== endpoint.hostname || peer.port !== Number(endpoint.port)) {
        throw Error('did not observe direct LAN media delivery');
      }
      const [r, g, b] = metrics.first_frame.pixel;
      if (!(r > 180 && g < 50 && b < 50 && metrics.first_frame.media_seconds < .5
          && [metrics.first_frame, metrics.last_frame].every(f =>
            f.width === server.fixture_dimensions[0] && f.height === server.fixture_dimensions[1])
          && metrics.frames >= 432 && metrics.media_advance_seconds >= 18)) {
        throw Error('unexpected video output or insufficient decoded progress');
      }
      return { ...metrics, peer_verified: true };
    }));
    const failure = completed.find(r => r.status === 'rejected');
    if (failure) throw failure.reason;
    const players = completed.map(r => r.value);
    const overlapMs = Math.min(...players.map(r => r.window_end_epoch_ms))
      - Math.max(...players.map(r => r.first_frame_epoch_ms));
    if (overlapMs < 18000) throw Error('playback windows did not overlap for 18 seconds');
    return { players, overlap_ms: overlapMs };
  } finally {
    await Promise.allSettled(contexts.map(context => context.close()));
  }
}
