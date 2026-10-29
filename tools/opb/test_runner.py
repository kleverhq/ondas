import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import signal
import sys
import time
import tempfile
import unittest
from unittest.mock import patch

from . import bindings, runner
from .test_model import case


class ImmediatePool:
    """Run mocked verification synchronously to inspect the controller barrier."""
    def __init__(self, **kwargs):
        self.workers = kwargs['max_workers']
    def __enter__(self):
        return self
    def __exit__(self, *args):
        pass
    def shutdown(self, **kwargs):
        pass
    def map(self, function, arguments):
        return map(function, arguments)


class RunnerTests(unittest.TestCase):
    def args(self, output, **extra):
        values = dict(command='run', output=str(output), backend='fsdb-lib', sweep=True,
                      quick=False, full=False, case=['OPB-POINT-028'], verify_workers=None, no_verify=False, timeout=600)
        values.update(extra)
        return argparse.Namespace(**values)

    def test_available_cpus_and_positive_override(self):
        with patch.object(os, 'sched_getaffinity', return_value={1, 2, 3}):
            self.assertEqual(runner.cpu_count(), 3)
        with patch.object(os, 'sched_getaffinity', side_effect=OSError()), patch.object(os, 'cpu_count', return_value=None):
            self.assertEqual(runner.cpu_count(), 1)
        for value in ('0', '-1'):
            with self.assertRaises(argparse.ArgumentTypeError):
                runner.positive(value)
        self.assertEqual(runner.positive('64'), 64)

    def test_cli_measurement_deadline_default_override_and_disable(self):
        for flags, expected in (([], 600), (['--timeout', '7'], 7), (['--no-timeout'], None)):
            with self.subTest(flags=flags), patch.object(runner, 'run', return_value=0) as run, patch.object(runner.signal, 'signal'):
                self.assertEqual(runner.main(['run', '--backend', 'fst-lib', *flags]), 0)
                self.assertEqual(run.call_args.args[0].timeout, expected)
        with patch.object(runner.signal, 'signal'), patch('sys.stderr', new_callable=io.StringIO):
            with self.assertRaises(SystemExit) as error:
                runner.main(['run', '--backend', 'fst-lib', '--timeout', '7', '--no-timeout'])
            self.assertEqual(error.exception.code, 2)

    def test_measurement_timeout_continues_and_does_not_limit_verification(self):
        for budget in (1, None):
            with self.subTest(budget=budget), tempfile.TemporaryDirectory() as d:
                path = Path(d) / 'run'
                def build(config, directory):
                    config['binary'] = 'unused'
                    return {}
                def process(command, env, output, timeout=None):
                    if command[1] == 'measure':
                        self.assertEqual(timeout, budget)
                        output.write('native diagnostic\n')
                        if command[-1] == '0' and budget is not None:
                            partial = path / 'raw/0000/partial.json'
                            partial.parent.mkdir(parents=True)
                            partial.write_text('{}')
                            raise subprocess.TimeoutExpired(command, timeout)
                    else:
                        self.assertEqual(command[1], 'verify')
                        self.assertIsNone(timeout)
                        saved = json.loads((path / 'run.json').read_text())
                        self.assertIn('finished_at', saved['phases']['measurement'])
                with patch.object(runner, 'input_ready'), patch.object(runner, 'source_identity', return_value={}), patch.object(runner, 'verify_identity'), patch.object(runner, 'build', side_effect=build), patch.object(runner, 'process', side_effect=process), patch.object(runner, 'raw_files', return_value=['unused']) as raw, patch.object(runner.oracle, 'check', return_value={}) as check, patch.object(runner.concurrent.futures, 'ProcessPoolExecutor', ImmediatePool), patch('sys.stdout', new_callable=io.StringIO):
                    result = runner.run(self.args(path, backend='fst-lib', sweep=False, timeout=budget), [case('OPB-POINT-027'), case('OPB-POINT-028')])
                saved = json.loads((path / 'run.json').read_text())
                self.assertEqual(saved['measurement_timeout_seconds'], budget)
                self.assertEqual(result, 0 if budget is None else 1)
                self.assertEqual(raw.call_count, 2 if budget is None else 1)
                self.assertEqual(check.call_count, 2 if budget is None else 1)
                first, second = saved['jobs']
                self.assertEqual(second['measurement']['status'], 'measured')
                self.assertEqual(second['verification']['status'], 'passed')
                if budget is not None:
                    self.assertEqual(first['measurement']['status'], 'failed')
                    self.assertEqual(first['measurement']['reason'], 'timeout')
                    self.assertNotIn('raw_files', first['measurement'])
                    self.assertEqual(first['verification']['status'], 'not-run')
                    self.assertEqual((path / first['measurement']['log']).read_text(), 'native diagnostic\n')
                    self.assertEqual((path / 'raw/0000/partial.json').read_text(), '{}')

    def test_sweep_barrier_statuses_and_no_verify(self):
        for no_verify in (False, True):
            with self.subTest(no_verify=no_verify), tempfile.TemporaryDirectory() as d:
                events = []
                def build(config, directory):
                    events.append(('ready', config['position']))
                    config['binary'] = 'unused'
                    return {'SDK_POSITION': str(config['position'])}
                def measure(command, env, output, timeout):
                    self.assertEqual(command[1], 'measure')
                    self.assertEqual(timeout, 600)
                    events.append(('measure', int(env['SDK_POSITION'])))
                def verify(arguments):
                    path, index, env = arguments
                    saved = json.loads(Path(path).read_text())
                    self.assertIn('finished_at', saved['phases']['measurement'])
                    self.assertTrue(all(j['measurement']['status'] == 'measured' for j in saved['jobs']))
                    events.append(('verify', int(env['SDK_POSITION'])))
                    return index, dict(status='passed')
                with patch.dict(os.environ, {'VERDI_HOMES': '/sdk/one /sdk/two'}), patch.object(runner, 'input_ready'), patch.object(runner, 'source_identity', return_value={}), patch.object(runner, 'verify_identity'), patch.object(runner, 'build', side_effect=build), patch.object(runner, 'process', side_effect=measure), patch.object(runner, 'raw_files', return_value=['unused']), patch.object(runner, 'verify_one', side_effect=verify), patch.object(runner, 'cpu_count', return_value=7), patch.object(runner.concurrent.futures, 'ProcessPoolExecutor', ImmediatePool), patch('sys.stdout', new_callable=io.StringIO):
                    path = Path(d) / 'run'
                    self.assertEqual(runner.run(self.args(path, no_verify=no_verify), [case('OPB-POINT-028')]), 0)
                self.assertEqual(events[:4], [('ready', 1), ('ready', 2), ('measure', 1), ('measure', 2)])
                self.assertEqual(events[4:], [] if no_verify else [('verify', 1), ('verify', 2)])
                saved = json.loads((path / 'run.json').read_text())
                self.assertEqual(saved['verification_workers']['resolved'], 7)
                self.assertEqual(saved['measurement_timeout_seconds'], 600)
                self.assertEqual(saved['verification_workers']['effective'], 0 if no_verify else 2)
                self.assertEqual([j['verification']['status'] for j in saved['jobs']], ['unchecked' if no_verify else 'passed'] * 2)

    def test_wdb_sweep_uses_shared_sdk_setting_and_isolates_children(self):
        with tempfile.TemporaryDirectory() as d:
            homes = [Path(d) / name for name in ('one', 'two')]
            for home in homes:
                home.mkdir()
            with patch.dict(os.environ, {'XILINX_VIVADO_HOMES': ' '.join(map(str, homes)),
                                         'XILINX_VIVADO': str(homes[1])}, clear=True), patch.object(runner, 'ROOT', Path(d)):
                args = self.args(d, backend='wdb-lib')
                configs = runner.configurations(args)
                self.assertEqual([c['sdk_home'] for c in configs], list(map(str, homes)))
                self.assertEqual([c['position'] for c in configs], [1, 2])
                environments = [runner.environment(c) for c in configs]
                for home, env in zip(homes, environments):
                    self.assertEqual(env['XILINX_VIVADO'], str(home))
                    self.assertNotIn('XILINX_VIVADO_HOMES', env)
                    self.assertEqual(env['RDI_DATADIR'], str(home / 'data'))
                self.assertNotEqual(environments[0]['CARGO_TARGET_DIR'], environments[1]['CARGO_TARGET_DIR'])
                os.environ['XILINX_VIVADO_HOMES'] = ''
                self.assertEqual([c['sdk_home'] for c in runner.configurations(args)], [str(homes[1])])

    def test_outer_sdk_build_directories_remain_separate(self):
        with tempfile.TemporaryDirectory() as d, patch.dict(os.environ, {}, clear=True), patch.object(runner, 'ROOT', Path(d)):
            root = Path(d)
            homes = [root / name for name in ('sdk-one', 'sdk-two')]
            targets = []
            for index, home in enumerate(homes, 1):
                home.mkdir()
                outer = root / 'fsdb-sdk' / str(index)
                os.environ.update(VERDI_HOME=str(home), CARGO_TARGET_DIR=str(outer))
                config = runner.configurations(self.args(d, sweep=False))[0]
                env = runner.environment(config)
                self.assertEqual(env['VERDI_HOME'], str(home))
                target = Path(env['CARGO_TARGET_DIR'])
                self.assertEqual(target, outer / 'opb-build/fsdb-lib/1')
                targets.append(target)
            self.assertNotEqual(*targets)
            os.environ.pop('CARGO_TARGET_DIR')
            env = runner.environment(config)
            self.assertEqual(Path(env['CARGO_TARGET_DIR']), root / 'target/opb-build/fsdb-lib/1')

    def test_failed_measurement_cannot_be_repaired_by_verification(self):
        with tempfile.TemporaryDirectory() as d:
            def build(config, directory):
                config['binary'] = 'unused'
                return {}
            with patch.object(runner, 'input_ready'), patch.object(runner, 'source_identity', return_value={}), patch.object(runner, 'verify_identity'), patch.object(runner, 'build', side_effect=build), patch.object(runner, 'process', side_effect=RuntimeError('failed timing')), patch.object(runner, 'verify_one') as verify, patch('sys.stdout', new_callable=io.StringIO):
                path = Path(d) / 'run'
                self.assertEqual(runner.run(self.args(path, backend='fst-lib', sweep=False), [case('OPB-POINT-028')]), 1)
            verify.assert_not_called()
            saved = json.loads((path / 'run.json').read_text())
            self.assertEqual(saved['jobs'][0]['measurement']['status'], 'failed')
            self.assertEqual(saved['jobs'][0]['verification']['status'], 'not-run')

    def test_unknown_never_succeeds_as_empty(self):
        for identifier in ('OPB-ABSENT-001',):
            with self.assertRaises(ValueError):
                runner.selected_cases(self.args('/unused', case=[identifier]))

    def test_quick_is_smoke_and_other_handlers_remain_selectable(self):
        for command, quick in (('run', False), ('run', True), ('list', True)):
            selected = runner.selected_cases(self.args('/unused', command=command, quick=quick, case=None))
            self.assertEqual([c['id'] for c in selected], ['OPB-POINT-028'])
        selected = runner.selected_cases(self.args('/unused', command='list', case=None))
        self.assertEqual(len(selected), 354)
        selected = runner.selected_cases(self.args('/unused', case=['OPB-SESSION-003', 'OPB-OPEN-002']))
        self.assertEqual([c['id'] for c in selected], ['OPB-OPEN-002', 'OPB-SESSION-003'])

    def test_readiness_checks_complete_shm_without_waveform_api(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            directory = root / 'fixtures' / 'shm' / 'example'
            (directory / 'waveform.shm').mkdir(parents=True)
            payload = directory / 'waveform.shm' / 'one.trn'
            payload.write_bytes(b'bytes')
            artifact = dict(file='waveform.shm', format='shm', files=[dict(file='one.trn', size=5, sha256=runner.sha256(payload))])
            (directory / 'fixture.json').write_text(json.dumps(dict(artifact=artifact)))
            binding = dict(fixture='shm/example', artifact=artifact)
            with patch.object(runner, 'ROOT', root):
                self.assertEqual(runner.input_ready(binding), payload.parent)
                (payload.parent / 'unexpected').write_bytes(b'')
                with self.assertRaisesRegex(ValueError, 'component set'):
                    runner.input_ready(binding)
                (payload.parent / 'unexpected').unlink()
                payload.write_bytes(b'wrong')
                with self.assertRaisesRegex(ValueError, 'component mismatch'):
                    runner.input_ready(binding)

    def test_missing_samples_cannot_be_claimed_measured(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, 'fresh Criterion'):
                runner.raw_files(dict(raw='raw'), Path(d))

    def test_runtime_libraries_include_absolute_needed_paths(self):
        with tempfile.TemporaryDirectory() as d:
            ordinary = Path(d) / 'ordinary.so'
            absolute = Path(d) / 'reader (with spaces).so'
            ordinary.touch()
            absolute.touch()
            output = f'libordinary.so => {ordinary} (0x1)\n\t{absolute} (0x2)\nlinux-vdso.so.1 (0x3)\nmissing.so => not found\n'
            self.assertEqual(runner.runtime_libraries(output), sorted([str(ordinary), str(absolute)]))

    def test_timeout_reaps_process_group(self):
        with tempfile.TemporaryFile() as output:
            with self.assertRaises(subprocess.TimeoutExpired):
                runner.process(['python3', '-c', 'import time; time.sleep(10)'], os.environ.copy(), output, 0.05)

    def test_stop_child_kills_members_after_group_leader_exits(self):
        for leader_exited in (False, True):
            with self.subTest(leader_exited=leader_exited):
                leader = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], process_group=0)
                member = None
                try:
                    member = subprocess.Popen([sys.executable, '-c', 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); print("ready",flush=True); time.sleep(30)'], process_group=leader.pid, stdout=subprocess.PIPE, text=True)
                    self.assertEqual(member.stdout.readline().strip(), 'ready')
                    if leader_exited:
                        leader.terminate()
                        leader.wait(timeout=5)
                    runner.stop_child(leader)
                    self.assertEqual(member.wait(timeout=5), -signal.SIGKILL)
                finally:
                    for child in (leader, member):
                        if child is not None:
                            if child.poll() is None:
                                child.kill()
                            child.wait(timeout=5)
                    if member is not None:
                        member.stdout.close()

    def test_parent_only_interrupt_reaps_native_verifier(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryFile() as output:
            root = Path(d)
            child = root / 'fake-workload'
            child.write_text('#!/usr/bin/env python3\nimport os,time\nfrom pathlib import Path\nPath(os.environ["PIDFILE"]).write_text(f"{os.getpid()} {os.getppid()}")\ntime.sleep(30)\n')
            child.chmod(0o755)
            path = root / 'run.json'
            path.write_text(json.dumps(dict(jobs=[dict(configuration=0, input=dict(case=case('OPB-OPEN-002')))], configurations=[dict(binary=str(child))])))
            env = os.environ.copy()
            env['PIDFILE'] = str(root / 'pid')
            code = f'from tools.opb.runner import verify_pool; import os; list(verify_pool([({str(path)!r},0,dict(os.environ))]*8,1))'
            controller = subprocess.Popen([sys.executable, '-c', code], stdout=output, stderr=output, env=env, start_new_session=True)
            try:
                limit = time.monotonic() + 10
                while not (root / 'pid').exists() and time.monotonic() < limit:
                    if controller.poll() is not None:
                        self.fail('controller exited before verifier started')
                    time.sleep(0.01)
                self.assertTrue((root / 'pid').exists(), 'verifier did not start')
                native_pid, worker_pid = map(int, (root / 'pid').read_text().split())
                os.kill(controller.pid, signal.SIGINT)  # Deliberately do not signal its group.
                self.assertNotEqual(controller.wait(timeout=10), 0)
                for pid in (native_pid, worker_pid):
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
            finally:
                if controller.poll() is None:
                    os.killpg(controller.pid, signal.SIGKILL)
                    controller.wait()

    def test_process_reaps_group_after_success_or_failed_exit(self):
        for status in (0, 1):
            with self.subTest(status=status), tempfile.TemporaryFile() as output:
                leader = subprocess.Popen([sys.executable, '-c', f'import sys; sys.stdin.read(1); sys.exit({status})'],
                                          process_group=0, stdin=subprocess.PIPE, text=True)
                member = None
                try:
                    member = subprocess.Popen([sys.executable, '-c', 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); print("ready",flush=True); time.sleep(30)'],
                                              process_group=leader.pid, stdout=subprocess.PIPE, text=True)
                    self.assertEqual(member.stdout.readline().strip(), 'ready')
                    leader.stdin.close()
                    with patch.object(runner.subprocess, 'Popen', return_value=leader):
                        if status:
                            with self.assertRaisesRegex(RuntimeError, 'exited 1'):
                                runner.process(['unused'], {}, output)
                        else:
                            runner.process(['unused'], {}, output)
                    self.assertEqual(member.wait(timeout=5), -signal.SIGKILL)
                finally:
                    leader.stdin.close()
                    for child in (leader, member):
                        if child is not None:
                            if child.poll() is None:
                                child.kill()
                            child.wait(timeout=5)
                    if member is not None:
                        member.stdout.close()

    def test_worker_termination_during_cleanup_reaps_native_child(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryFile() as output:
            marker = Path(d) / 'cleanup'
            native = 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)'
            code = f'''from pathlib import Path
import os,sys
from tools.opb import runner
original = runner.stop_child
def marked(child):
    marker = Path({str(marker)!r})
    marker.with_suffix('.tmp').write_text(str(child.pid))
    marker.with_suffix('.tmp').replace(marker)
    original(child)
runner.stop_child = marked
runner.verifier_init()
runner.process([sys.executable, '-c', {native!r}], dict(os.environ), sys.stdout, 0.5)
'''
            worker = subprocess.Popen([sys.executable, '-c', code], stdout=output, stderr=output, start_new_session=True)
            native_pid = None
            try:
                limit = time.monotonic() + 10
                while not marker.exists() and time.monotonic() < limit:
                    self.assertIsNone(worker.poll(), 'worker exited before cleanup')
                    time.sleep(0.01)
                self.assertTrue(marker.exists(), 'cleanup did not start')
                native_pid = int(marker.read_text())
                os.kill(worker.pid, signal.SIGTERM)
                self.assertEqual(worker.wait(timeout=10), 128 + signal.SIGTERM)
                with self.assertRaises(ProcessLookupError):
                    os.kill(native_pid, 0)
            finally:
                if native_pid is not None:
                    try:
                        os.killpg(native_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if worker.poll() is None:
                    worker.kill()
                worker.wait(timeout=5)

    def test_changed_source_cannot_receive_replay_validation(self):
        with patch.object(runner, 'source_identity', return_value={'model.py': 'changed'}):
            with self.assertRaisesRegex(ValueError, 'source/model changed'):
                runner.verify_identity(dict(source_files={'model.py': 'original'}, configurations=[]))

    def test_build_script_change_invalidates_completed_replay(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name in ('Cargo.toml', 'Cargo.lock', 'build.rs'):
                (root / name).write_text('original')
            with patch.object(runner, 'ROOT', root):
                data = dict(source_files=runner.source_identity(), configurations=[],
                            jobs=[dict(verification=dict(status='passed'))])
                (root / 'build.rs').write_text('changed')
                with self.assertRaisesRegex(ValueError, 'source/model changed'):
                    runner.verify_identity(data)
                self.assertEqual(data['jobs'][0]['verification']['status'], 'failed')

    def test_identity_io_error_invalidates_completed_replay(self):
        with patch.object(runner, 'source_identity', return_value={}):
            with tempfile.TemporaryDirectory() as d:
                data = dict(source_files={}, configurations=[dict(binary=str(Path(d) / 'missing'), binary_sha256='unused')],
                            jobs=[dict(verification=dict(status='passed'))])
                with self.assertRaisesRegex(ValueError, 'identity unavailable'):
                    runner.verify_identity(data)
                self.assertEqual(data['jobs'][0]['verification']['status'], 'failed')
