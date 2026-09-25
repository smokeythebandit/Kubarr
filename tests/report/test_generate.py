import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('generate.py')
RUN_RUST = Path(__file__).with_name('run-rust.sh')


class ReportTests(unittest.TestCase):
    def test_playwright_expected_failure_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / 'browser.json'
            browser.write_text(json.dumps({'suites': [{'file': 'tests/browser/17-domains.spec.ts',
                'specs': [
                    {'title': 'expected failure', 'tests': [{'status': 'expected',
                        'expectedStatus': 'failed', 'results': [{'status': 'failed', 'duration': 4}]}]},
                    {'title': 'passing test', 'tests': [{'status': 'expected',
                        'expectedStatus': 'passed', 'results': [{'status': 'passed', 'duration': 2}]}]},
                    {'title': 'passing retry', 'tests': [{'status': 'flaky',
                        'expectedStatus': 'passed', 'results': [{'status': 'failed'}, {'status': 'passed'}]}]},
                ]}]}))
            output = root / 'out'
            subprocess.run([sys.executable, str(SCRIPT), '--playwright', f'browser={browser}',
                            '--output', str(output)], check=True)
            report = json.loads((output / 'results.json').read_text())
            observed = {r['scenario']: r['status'] for r in report['scenarios'] if r.get('kind') != 'requirement'}
            self.assertEqual(observed, {'expected failure': 'failed', 'passing test': 'passed',
                                        'passing retry': 'passed'})
            self.assertEqual(report['totals'], {'passed': 2, 'failed': 1, 'skipped': 0, 'not-run': 0})

    def test_frontend_harness_outcome_is_recorded_with_playwright_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            live = root / 'real-smoke.json'
            live.write_text(json.dumps({'suites': [{'file': 'tests/real/settings-accounts.spec.ts',
                'specs': [{'title': 'passing test', 'tests': [{'status': 'expected',
                    'expectedStatus': 'passed', 'results': [{'status': 'passed'}]}]}]}]}))
            for outcome in ('passed', 'failed'):
                with self.subTest(outcome=outcome):
                    output = root / outcome
                    subprocess.run([sys.executable, str(SCRIPT), '--playwright', f'live={live}',
                                    '--lane-result', f'live={outcome}', '--output', str(output)], check=True)
                    report = json.loads((output / 'results.json').read_text())
                    observed = [r for r in report['scenarios'] if r.get('kind') != 'requirement']
                    self.assertEqual([(r['lane'], r['section'], r['status']) for r in observed],
                                     [('live', 'users', 'passed'), ('live', 'acceptance', outcome)])
                    self.assertEqual(report['totals']['failed'], int(outcome == 'failed'))
            output = root / 'no-playwright'
            subprocess.run([sys.executable, str(SCRIPT), '--lane-result', 'live=failed',
                            '--output', str(output)], check=True)
            report = json.loads((output / 'results.json').read_text())
            self.assertEqual([(r['lane'], r['status']) for r in report['scenarios']
                              if r.get('kind') != 'requirement'], [('live', 'failed')])

    def test_missing_inputs_and_untrusted_titles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            browser = root / 'browser.json'
            browser.write_text(json.dumps({'suites': [{'file': 'tests/browser/17-domains.spec.ts',
                'specs': [{'title': '<script>alert(1)</script>', 'tests': [{'status': 'unexpected',
                    'results': [{'duration': 12, 'startTime': '2026-01-01T00:00:00Z',
                                 'attachments': [{'name': 'sanitized-failure'}]}]}]}]}]}))
            dest = root / 'out'
            subprocess.run([sys.executable, str(SCRIPT), '--playwright', f'browser={browser}',
                            '--playwright', f'live={root / "absent.json"}', '--output', str(dest)], check=True)
            data = json.loads((dest / 'results.json').read_text())
            self.assertEqual(data['totals']['failed'], 1)
            self.assertFalse(any(r['lane'] == 'live' and r['status'] == 'passed' for r in data['scenarios']))
            self.assertTrue(any(r['status'] == 'not-covered' for r in data['scenarios']))
            self.assertTrue(any(r['lane'] == 'live' and r['status'] == 'not-run' for r in data['scenarios']))
            page = (dest / 'index.html').read_text()
            self.assertNotIn('<script>alert(1)</script>', page)
            self.assertIn('&lt;script&gt;', page)

    def test_full_rust_log_failures_skips_and_duplicate_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'rust.log'
            log.write_text('Running unittests src/lib.rs (target/debug/deps/kubarr)\n'
                           'test settings::allowed ... ok\n'
                           'test settings::denied ... FAILED\n'
                           'test settings::external ... ignored\n'
                           'test result: FAILED. 1 passed; 1 failed; 1 ignored\n')
            Path(f'{log}.exit').write_text('101\n')
            output = root / 'out'
            args = [sys.executable, str(SCRIPT), '--rust', str(log), '--output', str(output)]
            subprocess.run(args, check=True)
            report = json.loads((output / 'results.json').read_text())
            self.assertEqual(report['totals'], {'passed': 1, 'failed': 1, 'skipped': 1, 'not-run': 0})
            self.assertEqual(report['commands'], [{'lane': 'api', 'log': str(log), 'exit_code': 101, 'status': 'failed'}])
            self.assertEqual({r['source'] for r in report['scenarios'] if r['lane'] == 'api' and r.get('kind') != 'requirement'}, {'src/lib.rs'})
            duplicate = subprocess.run([sys.executable, str(SCRIPT), '--rust', str(log),
                                        '--rust', str(log), '--output', str(output)], capture_output=True)
            self.assertNotEqual(duplicate.returncode, 0)
            self.assertIn(b'Duplicate input', duplicate.stderr)

    def test_interrupted_rust_and_playwright_discovery_are_not_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'rust.log'
            log.write_text('running 3 tests\ntest api::first ... ok\n')
            Path(f'{log}.exit').write_text('101\n')
            browser = root / 'browser.json'
            browser.write_text(json.dumps({'suites': [{'file': 'tests/browser/18-settings-access.spec.ts',
                'specs': [{'title': 'discovered only', 'tests': [{'status': 'skipped', 'results': []}]}]}],
                'errors': [], 'stats': {'skipped': 1}}))
            output = root / 'out'
            subprocess.run([sys.executable, str(SCRIPT), '--rust', str(log), '--playwright',
                            f'browser={browser}', '--output', str(output)], check=True)
            report = json.loads((output / 'results.json').read_text())
            self.assertEqual(report['totals']['failed'], 1)
            self.assertEqual(report['totals']['passed'], 1)
            self.assertEqual(report['totals']['not-run'], 1)

    def test_rust_compile_error_and_nonzero_after_passed_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            failed_compile = root / 'compile.log'
            failed_compile.write_text('error[E0425]: cannot find value\n')
            Path(f'{failed_compile}.exit').write_text('101\n')
            late_failure = root / 'late.log'
            late_failure.write_text('running 1 test\ntest good ... ok\n'
                                    'test result: ok. 1 passed; 0 failed; 0 ignored\nerror: post-test command failed\n')
            Path(f'{late_failure}.exit').write_text('2\n')
            output = root / 'out'
            subprocess.run([sys.executable, str(SCRIPT), '--rust', str(failed_compile), '--rust', str(late_failure),
                            '--rust', str(root / 'not-started.log'), '--output', str(output)], check=True)
            report = json.loads((output / 'results.json').read_text())
            self.assertEqual(report['totals'], {'passed': 1, 'failed': 2, 'skipped': 0, 'not-run': 1})
            self.assertEqual([c['status'] for c in report['commands']], ['failed', 'failed', 'not-run'])
            self.assertFalse(any(r['status'] == 'passed' and r['source'] == 'compile.log' for r in report['scenarios']))

    def test_runner_captures_pretest_exit_and_late_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, output, code in [('pretest', 'error: could not compile\n', 101),
                                       ('late', 'running 1 test\ntest x ... ok\ntest result: ok. 1 passed\n', 7)]:
                log = root / f'{name}.log'
                result = subprocess.run(['bash', str(RUN_RUST), str(log), sys.executable,
                                         '-c', f'import sys;sys.stdout.write({output!r});sys.exit({code})'],
                                        capture_output=True)
                self.assertEqual(result.returncode, code)
                self.assertEqual(Path(f'{log}.exit').read_text().strip(), str(code))
                self.assertEqual(log.read_text(), output)

    def test_passing_rows_without_command_exit_remain_unverified(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'no-exit.log'
            log.write_text('running 1 test\ntest first ... ok\ntest result: ok. 1 passed\n')
            output = root / 'out'
            subprocess.run([sys.executable, str(SCRIPT), '--rust', str(log), '--output', str(output)], check=True)
            report = json.loads((output / 'results.json').read_text())
            self.assertEqual(report['commands'][0]['status'], 'not-run')
            self.assertEqual(report['totals']['passed'], 1)
            self.assertEqual(report['totals']['not-run'], 1)


if __name__ == '__main__': unittest.main()
