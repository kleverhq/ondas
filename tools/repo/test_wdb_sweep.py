import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


WRAPPER = Path(__file__).with_name("wdb-sweep")


class WdbSweepTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.log = self.root / "calls.jsonl"
        self.homes = [self.root / name for name in ("reader-a", "reader-b")]
        for home in self.homes:
            home.mkdir()
        self.command = self.root / "command"
        self.command.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "record = {key: os.environ.get(key) for key in "
            "('XILINX_VIVADO', 'XILINX_VIVADO_HOMES', 'CARGO_TARGET_DIR', 'RDI_DATADIR', 'LD_LIBRARY_PATH')}\n"
            "record['args'] = sys.argv[1:]\n"
            "with open(os.environ['CALL_LOG'], 'a') as log:\n"
            "    log.write(json.dumps(record) + '\\n')\n"
            "sys.exit(int(os.environ.get('FAIL_STATUS', '42')) "
            "if os.environ['XILINX_VIVADO'] == os.environ.get('FAIL_HOME') else 0)\n"
        )
        self.command.chmod(0o755)

    def run_sweep(self, **settings):
        env = {key: value for key, value in os.environ.items()
               if key not in ('XILINX_VIVADO', 'XILINX_VIVADO_HOMES', 'CARGO_TARGET_DIR', 'RDI_DATADIR', 'LD_LIBRARY_PATH')}
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
        home.mkdir()
        result, calls = self.run_sweep(XILINX_VIVADO=str(home))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]['XILINX_VIVADO'], str(home))
        self.assertEqual(calls[0]['CARGO_TARGET_DIR'], 'target/wdb-sdk/1')
        self.assertEqual(calls[0]['args'], ['argument with spaces', '--test'])
        self.assertEqual(calls[0]['RDI_DATADIR'], str(home / 'data'))
        self.assertEqual(calls[0]['LD_LIBRARY_PATH'], str(home / 'lib/lnx64.o'))

    def test_list_overrides_single_home_and_isolates_output(self):
        result, calls = self.run_sweep(
            XILINX_VIVADO_HOMES='\n'.join(map(str, self.homes)), XILINX_VIVADO='/sdk/unused',
            CARGO_TARGET_DIR='tmp/build', RDI_DATADIR='/unused/data',
            LD_LIBRARY_PATH='/existing/runtime',
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call['XILINX_VIVADO'] for call in calls], list(map(str, self.homes)))
        self.assertEqual([call['CARGO_TARGET_DIR'] for call in calls],
                         ['tmp/build/wdb-sdk/1', 'tmp/build/wdb-sdk/2'])
        self.assertTrue(all(call['XILINX_VIVADO_HOMES'] is None for call in calls))
        for home, call in zip(self.homes, calls):
            self.assertEqual(call['RDI_DATADIR'], str(home / 'data'))
            self.assertEqual(call['LD_LIBRARY_PATH'],
                             f'{home}/lib/lnx64.o:/existing/runtime')

    def test_relative_home_uses_absolute_runtime_paths(self):
        result, calls = self.run_sweep(XILINX_VIVADO=self.homes[0].name)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[0]['XILINX_VIVADO'], str(self.homes[0]))
        self.assertEqual(calls[0]['RDI_DATADIR'], str(self.homes[0] / 'data'))
        self.assertEqual(calls[0]['LD_LIBRARY_PATH'], str(self.homes[0] / 'lib/lnx64.o'))

    def test_command_is_required(self):
        result = subprocess.run(
            ['bash', str(WRAPPER)], text=True, capture_output=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn('usage:', result.stderr)

    def test_missing_command_fails_each_sdk(self):
        self.command.unlink()
        result, calls = self.run_sweep(XILINX_VIVADO_HOMES=' '.join(map(str, self.homes)))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(calls, [])
        self.assertIn('FAIL SDK 1 (exit 127)', result.stderr)
        self.assertIn('FAIL SDK 2 (exit 127)', result.stderr)

    def test_failure_does_not_stop_later_sdks(self):
        result, calls = self.run_sweep(
            XILINX_VIVADO_HOMES=' '.join(map(str, self.homes)), FAIL_HOME=str(self.homes[0]),
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 2)
        self.assertIn('FAIL SDK 1', result.stderr)
        self.assertIn('PASS SDK 2', result.stdout)

    def test_missing_sdk_fails_and_continues(self):
        result, calls = self.run_sweep(
            XILINX_VIVADO_HOMES=f'{self.root}/missing {self.homes[1]}',
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 1)
        self.assertIn('SDK directory is absent', result.stderr)

    def test_empty_selection_fails(self):
        for settings in ({}, {'XILINX_VIVADO_HOMES': ' \t\n'}):
            with self.subTest(settings=settings):
                result, calls = self.run_sweep(**settings)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])

    def test_cancelled_command_stops_the_sweep(self):
        for status in (130, 143):
            with self.subTest(status=status):
                self.log.unlink(missing_ok=True)
                result, calls = self.run_sweep(
                    XILINX_VIVADO_HOMES=' '.join(map(str, self.homes)),
                    FAIL_HOME=str(self.homes[0]), FAIL_STATUS=str(status),
                )
                self.assertEqual(result.returncode, status)
                self.assertEqual(len(calls), 1)


if __name__ == '__main__':
    unittest.main()
