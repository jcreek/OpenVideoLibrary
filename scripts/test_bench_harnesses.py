"""Regression tests for benchmark harness failure paths (no candidate servers or media needed)."""
import argparse, contextlib, http.server, io, itertools, json, os, shutil, socket, subprocess, sys, tempfile, threading, time, types, unittest
from pathlib import Path

import bench_suite
import lan_playback_worker
import remote_bench

SCRIPTS = Path(__file__).resolve().parent


class FakeProcess:
    def __init__(self, fd):
        self.stdout = os.fdopen(fd, 'rb', buffering=0)


class ReadEventTests(unittest.TestCase):
    def setUp(self):
        read, self.write = os.pipe()
        self.process = FakeProcess(read)
        self.addCleanup(self.process.stdout.close)

    def test_partial_line_then_stall_times_out(self):
        os.write(self.write, b'{"ready": tr')
        self.addCleanup(os.close, self.write)
        start = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, 'timed out'):
            remote_bench.read_event(self.process, timeout=.3)
        self.assertLess(time.monotonic() - start, 2)

    def test_events_in_one_chunk_are_returned_in_order(self):
        os.write(self.write, b'{"a": 1}\n{"b": 2}\n')
        os.close(self.write)
        self.assertEqual(remote_bench.read_event(self.process, timeout=1), {'a': 1})
        self.assertEqual(remote_bench.read_event(self.process, timeout=1), {'b': 2})
        with self.assertRaisesRegex(RuntimeError, 'exited without response'):
            remote_bench.read_event(self.process, timeout=1)


class ItemsServer:
    """Serves 100 rows whose titles ignore the requested query."""

    def __init__(self, title):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(handler):
                body = json.dumps([{'id': i, 'title': f'{title} {i:03}'} for i in range(100)]).encode()
                handler.send_response(200)
                handler.send_header('Content-Type', 'application/json')
                handler.send_header('Content-Length', str(len(body)))
                handler.end_headers()
                handler.wfile.write(body)

            def log_message(handler, *args):
                pass
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class QueryValidationTests(unittest.TestCase):
    def test_remote_bench_rejects_rows_outside_the_query_prefix(self):
        server = ItemsServer('Movie 00001')
        self.addCleanup(server.close)
        self.assertEqual(remote_bench.load(3, 1, 'Movie 00009', server.port)['errors'], 3)
        result = remote_bench.load(3, 1, 'movie 00001', server.port)
        self.assertEqual(result['errors'], 0)
        self.assertIsNotNone(result['p95_ms'])

    def test_remote_bench_reports_null_p95_when_every_request_fails(self):
        server = ItemsServer('Other')
        self.addCleanup(server.close)
        self.assertIsNone(remote_bench.load(2, 1, 'Movie', server.port)['p95_ms'])


class BenchSuiteCleanupTests(unittest.TestCase):
    def test_candidate_is_reaped_when_an_encoder_cannot_start(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pidfile = root / 'pid'
            candidate = root / 'candidate'
            candidate.write_text(f'#!/bin/sh\necho $$ > "{pidfile}"\nexec /bin/sleep 60\n')
            candidate.chmod(0o755)
            media = root / 'sample.mp4'
            media.write_bytes(b'not media')
            (root / 'empty-path').mkdir()
            args = argparse.Namespace(media=str(media), scan_root=None, ffmpeg_count=1,
                                      ffmpeg_mode='transcode', ffmpeg_encoder='libx264')
            saved = os.environ['PATH']
            os.environ['PATH'] = str(root / 'empty-path')  # no ffmpeg
            try:
                with self.assertRaises(FileNotFoundError):
                    bench_suite.run(candidate, root / 'db', args, 1, '')
            finally:
                os.environ['PATH'] = saved
            deadline = time.monotonic() + 5
            while not pidfile.exists() and time.monotonic() < deadline:
                time.sleep(.05)
            pid = int(pidfile.read_text()) if pidfile.exists() else None
            if pid is None:
                return  # terminated before the shell recorded its pid: nothing left running
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            os.kill(pid, 9)
            self.fail('candidate process was left running')

    def fake_candidate(self, root, media_seconds):
        with socket.socket() as sock:
            if sock.connect_ex(('127.0.0.1', 8097)) == 0:
                self.skipTest('port 8097 is in use')
        server = root / 'server.py'
        server.write_text(f"""import http.server, json, time
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *a): pass
    def do_GET(self):
        if self.path.startswith('/media'):
            open({str(root / 'media-requested')!r}, 'w').close()
            time.sleep({media_seconds}); status, body = 206, b'x'
        elif self.path.startswith('/info'):
            status, body = 200, b'{{}}'
        else:
            status, body = 200, json.dumps([{{'id': i, 'title': 'Movie %d' % i}} for i in range(100)]).encode()
        self.send_response(status); self.send_header('Content-Length', str(len(body))); self.end_headers()
        self.wfile.write(body)
import os
open({str(root / 'pid')!r}, 'w').write(str(os.getpid()))
http.server.ThreadingHTTPServer(('127.0.0.1', 8097), H).serve_forever()
""")
        candidate = root / 'candidate'
        # A shell wrapper keeps an interpreter path containing spaces out of the shebang.
        candidate.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{server}"\n')
        candidate.chmod(0o755)
        media = root / 'sample.mp4'
        media.write_bytes(b'x')
        return candidate, argparse.Namespace(media=str(media), scan_root=None, ffmpeg_count=0, requests=1,
                                             ffmpeg_mode='transcode', ffmpeg_encoder='libx264')

    def test_in_flight_background_request_finishes_before_the_candidate_stops(self):
        with tempfile.TemporaryDirectory() as tmp:
            # A media range takes a second, so stopping the run almost always finds one in flight.
            candidate, args = self.fake_candidate(Path(tmp), 1)
            result = bench_suite.run(candidate, Path(tmp) / 'db', args, 1, '')
        self.assertEqual(result['errors'], 0)
        self.assertEqual(result['background']['errors'], 0)

    def test_signal_while_waiting_for_background_requests_still_reaps_the_candidate(self):
        pidfile, live_at_wait = [None], []

        class SignalledThread(threading.Thread):
            def __init__(self, *a, **kw):
                super().__init__(*a, **kw)
                # Thread.run() deletes _args when the target returns, so record the kind now.
                self.stream = kw.get('args') == ('stream',)

            def join(self, timeout=None):
                if self.stream:
                    # Only the success path waits while the candidate is still running.
                    try:
                        os.kill(int(pidfile[0].read_text()), 0)
                        live_at_wait.append(True)
                    except (OSError, ValueError):
                        live_at_wait.append(False)
                    raise SystemExit(143)  # SIGTERM arriving during the success-path wait
                return super().join(timeout)
        with tempfile.TemporaryDirectory() as tmp:
            candidate, args = self.fake_candidate(Path(tmp), 1)
            pidfile[0] = Path(tmp) / 'pid'
            saved = bench_suite.threading.Thread
            bench_suite.threading.Thread = SignalledThread
            try:
                with self.assertRaises(SystemExit):
                    bench_suite.run(candidate, Path(tmp) / 'db', args, 1, '')
            finally:
                bench_suite.threading.Thread = saved
            pid = int((Path(tmp) / 'pid').read_text())
        self.assertEqual(live_at_wait, [True])
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        os.kill(pid, 9)
        self.fail('candidate process was left running')

    def test_failed_run_stops_the_candidate_before_waiting_for_background_requests(self):
        with tempfile.TemporaryDirectory() as tmp:
            candidate, args = self.fake_candidate(Path(tmp), 20)
            real_load, calls = bench_suite.load, []

            def failing_load(*a):
                calls.append(1)
                if len(calls) == 2:
                    time.sleep(.5)  # let the stream worker start its slow request
                    raise RuntimeError('measured load failed')
                return real_load(*a)
            bench_suite.load = failing_load
            started = time.monotonic()
            try:
                with self.assertRaises(RuntimeError):
                    bench_suite.run(candidate, Path(tmp) / 'db', args, 1, '')
            finally:
                bench_suite.load = real_load
            # The slow request was in flight, so waiting on it first would take far longer.
            self.assertTrue((Path(tmp) / 'media-requested').exists())
        self.assertLess(time.monotonic() - started, 10)

    def test_h264_vaapi_is_rejected_without_hardware_upload(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / 'bench_suite.py'), '--output', os.devnull,
                                 '--ffmpeg-encoder', 'h264_vaapi'], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 2)
        self.assertIn('invalid choice', result.stderr)


class LanWorkerWaitTests(unittest.TestCase):
    def wait(self, commands, background='idle', step=50):
        chunks = iter(command.encode() + b'\n' for command in commands)

        def read(fd, n):
            # Running out of commands must not end the wait: only the deadline may.
            chunk = next(chunks, None)
            if chunk is None:
                raise AssertionError('read past the scripted commands')
            return chunk
        clock = itertools.count(step=step)
        worker = lan_playback_worker
        saved = worker.select, worker.os, worker.sys, worker.time, worker.observe_scans
        # Input is ready while commands remain; afterwards select reports a timeout.
        pending = list(commands)
        worker.select = types.SimpleNamespace(
            select=lambda r, w, x, timeout: (r, [], []) if pending and pending.pop(0) is not None else ([], [], []))
        worker.os = types.SimpleNamespace(read=read)
        worker.sys = types.SimpleNamespace(stdin=types.SimpleNamespace(fileno=lambda: 0))
        worker.time = types.SimpleNamespace(monotonic=lambda: next(clock))
        worker.observe_scans = lambda path, offset: {}
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                return worker.await_client(None, 0, background, 0)
        finally:
            worker.select, worker.os, worker.sys, worker.time, worker.observe_scans = saved

    def test_each_command_restarts_the_wait(self):
        # Ten snapshots spread over far more than one 240 s budget still reach done.
        self.assertTrue(self.wait(['scan-snapshot'] * 10 + ['done']))

    def test_a_silent_client_still_times_out(self):
        self.assertFalse(self.wait([], step=241))

    def test_sustained_run_without_background_has_one_budget_for_the_client_limits(self):
        # A 300 s gap before done exceeds the old shared 240 s budget but not the client limits.
        self.assertTrue(self.wait(['done'], background=None, step=300))
        self.assertFalse(self.wait([], background=None, step=421))

    def test_a_client_that_stops_after_a_snapshot_times_out(self):
        self.assertFalse(self.wait(['scan-snapshot'], step=100))

    def test_snapshots_beyond_the_cap_are_rejected(self):
        self.assertFalse(self.wait(['scan-snapshot'] * 11 + ['done']))


class PlaybackReferenceArgumentTests(unittest.TestCase):
    def test_zero_repetitions_is_rejected_before_startup(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / 'playback_reference.py'), '--output', os.devnull,
                                 '--repetitions', '0'], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 2)
        self.assertIn('at least 1', result.stderr)


class ReportN100Tests(unittest.TestCase):
    CLAIM = 'The 400-trial optimized matrix meets the 150 ms'

    NATIVE = 'In the native SQLite comparison, Rust had lower paired p95 latency'

    def render(self, matched_rows=None, native_rows=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'scripts').mkdir()
            shutil.copy(SCRIPTS / 'report_n100.py', root / 'scripts')
            folder = root / 'docs/benchmarks'
            folder.mkdir(parents=True)
            playback = [{'mode': m, 'adaptive': a, 'first_decoded_frame_ms': 1000}
                        for m, a in [('direct', False), ('transcode', False), ('transcode', True)]]
            (folder / 'n100-playback.json').write_text(json.dumps({'results': playback}))
            if matched_rows is not None:
                (folder / 'n100-matched-optimized.json').write_text(
                    json.dumps({'complete': True, 'results': matched_rows}))
            if native_rows is not None:
                (folder / 'n100-native-sqlite.json').write_text(
                    json.dumps({'complete': True, 'results': native_rows}))
            subprocess.run([sys.executable, '-I', str(root / 'scripts/report_n100.py')], check=True, timeout=30)
            return (folder / 'N100.md').read_text()

    @staticmethod
    def row(p95, candidate='rust', concurrency=100, errors=0, mode='idle', query='', repetition=0, cost=1):
        return {'mode': mode, 'candidate': candidate, 'items': 100000, 'concurrency': concurrency, 'requests': 2000,
                'errors': errors, 'child_errors': 0, 'background': {'errors': 0}, 'p95_ms': p95, 'p99_ms': p95,
                'query': query, 'repetition': repetition, 'successful_rps': 1000,
                'core_peak_rss_bytes': cost << 20, 'core_cpu_seconds': cost}

    def matrix(self, p95=10):
        return [self.row(p95, candidate, concurrency, mode=mode, query=query, repetition=repetition)
                for candidate in ('go', 'rust') for concurrency in (1, 10, 25, 50, 100)
                for mode in ('idle', 'mixed-vaapi') for query in ('', 'Movie 0009') for repetition in range(10)]

    def native(self, rust_p95=80):
        rows = []
        for mode in ('idle', 'mixed-vaapi'):
            for repetition in range(5):
                rows.append(self.row(190 if mode == 'idle' else 120, 'go-cgo', 200, mode=mode,
                                     repetition=repetition, cost=2))
                rows.append(self.row(rust_p95, 'rust', 200, mode=mode, repetition=repetition))
        return rows

    def test_missing_dataset_omits_its_conclusion(self):
        self.assertNotIn(self.CLAIM, self.render())

    def test_incomplete_or_slow_dataset_omits_its_conclusion(self):
        self.assertNotIn(self.CLAIM, self.render([self.row(10)]))
        self.assertNotIn(self.CLAIM, self.render(self.matrix()[:-1] + [self.row(151)]))
        self.assertNotIn(self.CLAIM, self.render([self.row(10)] * 400))  # one candidate, one concurrency

    def test_a_candidate_missing_one_concurrency_omits_its_conclusion(self):
        rows = self.matrix()
        for row in rows:
            if row['candidate'] == 'go' and row['concurrency'] == 100:
                row['candidate'] = 'rust'
        self.assertNotIn(self.CLAIM, self.render(rows))

    def test_a_candidate_missing_search_omits_its_conclusion(self):
        rows = self.matrix()
        for row in rows:
            if row['candidate'] == 'go' and row['query']:
                row['candidate'] = 'rust'
        self.assertNotIn(self.CLAIM, self.render(rows))

    def test_native_comparison_needs_supporting_measurements(self):
        self.assertIn(self.NATIVE, self.render(native_rows=self.native()))
        self.assertNotIn(self.NATIVE, self.render(native_rows=self.native(rust_p95=200)))
        unpaired = [row for row in self.native() if not (row['candidate'] == 'rust' and row['repetition'] == 4)]
        self.assertNotIn(self.NATIVE, self.render(native_rows=unpaired))
        # A second library size where Rust is slower must not be hidden behind the first.
        # Rust stays under 150 ms there, so only the pairing can reject it.
        slower = [dict(row, items=10000, p95_ms=140 if row['candidate'] == 'rust' else 120)
                  for row in self.native()]
        self.assertNotIn(self.NATIVE, self.render(native_rows=slower + self.native()))


    def test_supporting_dataset_keeps_its_conclusion(self):
        self.assertIn(self.CLAIM, self.render(self.matrix()))

    def test_trials_without_successful_requests_render_as_unavailable(self):
        report = self.render([self.row(None, errors=2000)])
        self.assertIn('| idle | rust | 100 | n/a | n/a |', report)
        self.assertNotIn(self.CLAIM, report)


if __name__ == '__main__':
    unittest.main()
