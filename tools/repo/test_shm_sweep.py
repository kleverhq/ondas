import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


WRAPPER = Path(__file__).with_name("shm-sweep")


class ShmSweepTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.log = self.root / "calls.jsonl"
        self.homes = [self.root / name for name in ("reader-a", "reader-b")]
        for home in self.homes:
            self.make_reader(home)
        self.command = self.root / "command"
        self.command.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "record = {key: os.environ.get(key) for key in "
            "('XCELIUM_HOME', 'XCELIUM_HOMES', 'CARGO_TARGET_DIR', 'LD_LIBRARY_PATH')}\n"
            "record['args'] = sys.argv[1:]\n"
            "with open(os.environ['CALL_LOG'], 'a') as log:\n"
            "    log.write(json.dumps(record) + '\\n')\n"
            "sys.exit(int(os.environ.get('FAIL_STATUS', '42')) "
            "if os.environ['XCELIUM_HOME'] == os.environ.get('FAIL_HOME') else 0)\n"
        )
        self.command.chmod(0o755)

    def make_reader(self, home):
        library = home / 'tools/lib/64bit/libsst2.so'
        library.parent.mkdir(parents=True)
        library.touch()

    def run_sweep(self, **settings):
        env = {key: value for key, value in os.environ.items()
               if key not in ('XCELIUM_HOME', 'XCELIUM_HOMES', 'CARGO_TARGET_DIR', 'LD_LIBRARY_PATH')}
        env.update(CALL_LOG=str(self.log), **settings)
        result = subprocess.run(
            ["bash", str(WRAPPER), str(self.command), "argument with spaces", "--test"],
            cwd=self.root, env=env, text=True, capture_output=True,
        )
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] \
            if self.log.exists() else []
        return result, calls

    def test_single_home_preserves_arguments_and_spaces(self):
        home = self.root / "reader with spaces"
        self.make_reader(home)
        result, calls = self.run_sweep(XCELIUM_HOME=str(home))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]['XCELIUM_HOME'], str(home))
        self.assertEqual(calls[0]['CARGO_TARGET_DIR'], 'target/shm-sdk/1')
        self.assertEqual(calls[0]['args'], ['argument with spaces', '--test'])
        runtime = self.root / 'target/shm-sdk/1/runtime'
        self.assertEqual(calls[0]['LD_LIBRARY_PATH'], str(runtime))
        self.assertEqual((runtime / 'libsst2.so').resolve(),
                         home / 'tools/lib/64bit/libsst2.so')

    def test_list_overrides_single_home_and_isolates_output(self):
        result, calls = self.run_sweep(
            XCELIUM_HOMES='\n'.join(map(str, self.homes)), XCELIUM_HOME='/sdk/unused',
            CARGO_TARGET_DIR='tmp/build', LD_LIBRARY_PATH='/existing/runtime',
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call['XCELIUM_HOME'] for call in calls], list(map(str, self.homes)))
        self.assertEqual([call['CARGO_TARGET_DIR'] for call in calls],
                         ['tmp/build/shm-sdk/1', 'tmp/build/shm-sdk/2'])
        self.assertTrue(all(call['XCELIUM_HOMES'] is None for call in calls))
        for index, call in enumerate(calls, start=1):
            runtime = self.root / f'tmp/build/shm-sdk/{index}/runtime'
            self.assertEqual(call['LD_LIBRARY_PATH'], f'{runtime}:/existing/runtime')
            self.assertEqual((runtime / 'libsst2.so').resolve(),
                             self.homes[index - 1] / 'tools/lib/64bit/libsst2.so')

    def test_failure_does_not_stop_later_sdks(self):
        result, calls = self.run_sweep(
            XCELIUM_HOMES=' '.join(map(str, self.homes)), FAIL_HOME=str(self.homes[0]),
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 2)
        self.assertIn('FAIL SDK 1', result.stderr)
        self.assertIn('PASS SDK 2', result.stdout)

    def test_missing_sdk_fails_and_continues(self):
        result, calls = self.run_sweep(
            XCELIUM_HOMES=f'{self.root}/missing {self.homes[1]}',
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 1)
        self.assertIn('SDK directory is absent', result.stderr)

    def test_missing_reader_fails_and_continues(self):
        (self.homes[0] / 'tools/lib/64bit/libsst2.so').unlink()
        result, calls = self.run_sweep(
            XCELIUM_HOMES=' '.join(map(str, self.homes)),
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual([call['XCELIUM_HOME'] for call in calls], [str(self.homes[1])])
        self.assertIn('SHM reader is absent', result.stderr)

    def test_empty_selection_fails(self):
        for settings in ({}, {'XCELIUM_HOMES': ' \t\n'}):
            with self.subTest(settings=settings):
                result, calls = self.run_sweep(**settings)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])

    def test_cancelled_command_stops_the_sweep(self):
        result, calls = self.run_sweep(
            XCELIUM_HOMES=' '.join(map(str, self.homes)),
            FAIL_HOME=str(self.homes[0]), FAIL_STATUS='130',
        )
        self.assertEqual(result.returncode, 130)
        self.assertEqual(len(calls), 1)


if __name__ == '__main__':
    unittest.main()
