import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

script = Path(__file__).resolve().parent / 'summarize_browser.py'


def report(errors):
    test = {'projectName': 'chromium', 'annotations': [], 'status': 'expected',
            'results': [{'status': 'passed', 'duration': 5}]}
    return {'suites': [{'specs': [{'title': 'passes', 'tests': [test]}]}], 'errors': errors,
            'stats': {'expected': 1, 'unexpected': 0}}


class SummarizeBrowserTests(unittest.TestCase):
    def run_summary(self, data):
        with tempfile.TemporaryDirectory() as tmp:
            source, output = Path(tmp) / 'in.json', Path(tmp) / 'out.json'
            source.write_text(json.dumps(data))
            result = subprocess.run([sys.executable, str(script), '--input', str(source), '--output', str(output)])
            return result.returncode, json.loads(output.read_text())

    def test_passing_run_succeeds(self):
        code, summary = self.run_summary(report([]))
        self.assertEqual(code, 0)
        self.assertEqual(summary['errors'], [])

    def test_run_level_error_fails_even_when_tests_pass(self):
        code, summary = self.run_summary(report([{'message': 'Error: teardown failed', 'stack': '/tmp/x/global.ts:1'}]))
        self.assertEqual(code, 1)
        # Only the message is kept, so ephemeral paths in stacks are not saved as evidence.
        self.assertEqual(summary['errors'], ['Error: teardown failed'])


if __name__ == '__main__':
    unittest.main()
