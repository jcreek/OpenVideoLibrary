"""Regression tests for benchmark harness failure paths (no candidate servers or media needed)."""
import argparse, http.server, json, os, shutil, subprocess, sys, tempfile, threading, time, unittest
from pathlib import Path

import bench_suite
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

    def test_h264_vaapi_is_rejected_without_hardware_upload(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / 'bench_suite.py'), '--output', os.devnull,
                                 '--ffmpeg-encoder', 'h264_vaapi'], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 2)
        self.assertIn('invalid choice', result.stderr)


class PlaybackReferenceArgumentTests(unittest.TestCase):
    def test_zero_repetitions_is_rejected_before_startup(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / 'playback_reference.py'), '--output', os.devnull,
                                 '--repetitions', '0'], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 2)
        self.assertIn('at least 1', result.stderr)


class ReportN100Tests(unittest.TestCase):
    CLAIM = 'The 400-trial optimized matrix meets the 150 ms'

    def render(self, matched_rows=None):
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
            subprocess.run([sys.executable, '-I', str(root / 'scripts/report_n100.py')], check=True, timeout=30)
            return (folder / 'N100.md').read_text()

    @staticmethod
    def row(p95, candidate='rust', concurrency=100, errors=0):
        return {'mode': 'idle', 'candidate': candidate, 'items': 100000, 'concurrency': concurrency, 'requests': 2000,
                'errors': errors, 'child_errors': 0, 'background': {'errors': 0}, 'p95_ms': p95, 'p99_ms': p95,
                'successful_rps': 1000, 'core_peak_rss_bytes': 1 << 20, 'core_cpu_seconds': 1}

    def matrix(self, p95=10):
        return [self.row(p95, candidate, concurrency) for candidate in ('go', 'rust')
                for concurrency in (1, 10, 25, 50, 100) for _ in range(40)]

    def test_missing_dataset_omits_its_conclusion(self):
        self.assertNotIn(self.CLAIM, self.render())

    def test_incomplete_or_slow_dataset_omits_its_conclusion(self):
        self.assertNotIn(self.CLAIM, self.render([self.row(10)]))
        self.assertNotIn(self.CLAIM, self.render(self.matrix()[:-1] + [self.row(151)]))
        self.assertNotIn(self.CLAIM, self.render([self.row(10)] * 400))  # one candidate, one concurrency

    def test_supporting_dataset_keeps_its_conclusion(self):
        self.assertIn(self.CLAIM, self.render(self.matrix()))

    def test_trials_without_successful_requests_render_as_unavailable(self):
        report = self.render([self.row(None, errors=2000)])
        self.assertIn('| idle | rust | 100 | n/a | n/a |', report)
        self.assertNotIn(self.CLAIM, report)


if __name__ == '__main__':
    unittest.main()
