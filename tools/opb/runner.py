"""Local OPB process orchestration. All timings finish before verification starts."""
import argparse
import concurrent.futures
import datetime
import hashlib
import json
import multiprocessing
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import tomllib

from . import bindings, oracle

ROOT = Path(__file__).resolve().parents[2]
SDK = {'fsdb-lib': 'VERDI_HOME', 'wlf-lib': 'QUESTA_HOME',
       'shm-lib': 'XCELIUM_HOME', 'wdb-lib': 'XILINX_VIVADO'}
_active_child = None
_protecting_child = False
_pending_signal = None

SETTINGS = dict(sample_size=10, warm_up_seconds=0.2, measurement_seconds=0.6,
                nresamples=10000, sampling_mode='Auto', confidence_level=0.95,
                significance_level=0.05, noise_threshold=0.01)


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def cpu_count():
    try:
        return max(1, len(os.sched_getaffinity(0)))
    except (AttributeError, OSError):
        return os.cpu_count() or 1


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_run(run):
    path = Path(run['directory']) / 'run.json'
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(run, indent=2) + '\n')
    temporary.replace(path)


def selected_cases(args):
    cases = bindings.CATALOG['cases']
    if args.case:
        known = {c['id']: c for c in cases}
        unknown = set(args.case) - known.keys()
        if unknown:
            raise ValueError(f'unknown case IDs: {sorted(unknown)}')
        cases = [c for c in cases if c['id'] in args.case]
    elif args.quick or (args.command == 'run' and not args.full):
        cases = [c for c in cases if c['id'] in bindings.QUICK]
    if args.command == 'run':
        missing = [c['id'] for c in cases if c['id'] not in bindings.IMPLEMENTED]
        if missing:
            raise ValueError(f'case handlers not implemented: {missing}')
    return cases


def sweep_variable(key):
    return 'XILINX_VIVADO_HOMES' if key == 'XILINX_VIVADO' else key + 'S'


def configurations(args):
    backends = bindings.BACKENDS if args.backend == 'all' else (args.backend,)
    if args.sweep and args.backend == 'all':
        raise ValueError('select one proprietary backend for an SDK sweep')
    result = []
    for backend in backends:
        key = SDK.get(backend)
        if args.sweep:
            if key is None:
                raise ValueError('SDK sweep requires a proprietary backend')
            homes = os.environ.get(sweep_variable(key), '').split() or [os.environ.get(key, '')]
        else:
            homes = [os.environ.get(key, '')] if key else ['']
        for index, home in enumerate(homes, 1):
            label = f'{backend}/sdk-{index}' if key else backend
            result.append(dict(backend=backend, label=label, sdk_variable=key, sdk_home=home, position=index))
    return result


def input_ready(binding):
    """Compare pinned metadata and bytes. Do not open a Waveform."""
    directory = ROOT / 'fixtures' / binding['fixture']
    sidecar = json.loads((directory / 'fixture.json').read_text())
    artifact = binding['artifact']
    if sidecar['artifact'] != artifact:
        raise ValueError(f"fixture metadata differs from pinned OPB input: {binding['fixture']}")
    path = directory / artifact['file']
    if 'files' in artifact:
        wanted = {c['file'] for c in artifact['files']}
        actual = {str(p.relative_to(path)) for p in path.rglob('*') if p.is_file()}
        if actual != wanted:
            raise ValueError('SHM component set differs')
        for component in artifact['files']:
            item = path / component['file']
            if item.stat().st_size != component['size'] or sha256(item) != component['sha256']:
                raise ValueError(f"SHM component mismatch: {component['file']}")
    elif path.stat().st_size != artifact['size'] or sha256(path) != artifact['sha256']:
        raise ValueError(f'waveform digest/size differs: {path.name}')
    return path


def environment(config):
    env = os.environ.copy()
    key = config['sdk_variable']
    target = Path(env.get('CARGO_TARGET_DIR') or ROOT / 'target').resolve()
    target = target / 'opb-build' / (config['backend'] if key else 'public')
    if key:
        home = Path(config['sdk_home'])
        if not config['sdk_home'] or not home.is_dir():
            raise ValueError(f"{key} must select an available SDK")
        env[key] = str(home)
        env.pop(sweep_variable(key), None)
        target /= str(config['position'])
        if config['backend'] == 'shm-lib':
            reader = home / 'tools/lib/64bit/libsst2.so'
            if not reader.is_file():
                raise ValueError('SHM SDK reader is absent')
            runtime = target / 'runtime'
            runtime.mkdir(parents=True, exist_ok=True)
            link = runtime / 'libsst2.so'
            link.unlink(missing_ok=True)
            link.symlink_to(reader.resolve())
            env['LD_LIBRARY_PATH'] = str(runtime) + ':' + env.get('LD_LIBRARY_PATH', '')
        if config['backend'] == 'wdb-lib':
            env['RDI_DATADIR'] = str(home / 'data')
            env['LD_LIBRARY_PATH'] = str(home / 'lib/lnx64.o') + ':' + env.get('LD_LIBRARY_PATH', '')
    env['CARGO_TARGET_DIR'] = str(target)
    return env


def stop_child(child):
    if child is None:
        return
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    # The leader can exit while another group member ignores SIGTERM.
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    child.wait()


def defer_signal(signum):
    global _pending_signal
    if _protecting_child:
        _pending_signal = signum
        return True
    return False


def interrupt(signum, frame):
    if not defer_signal(signum):
        raise KeyboardInterrupt


class WorkerStopped(BaseException):
    pass


def verifier_init():
    # Ctrl-C belongs to the controller. SIGTERM must also reap the native child.
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    def terminate(signum, frame):
        if defer_signal(signum):
            return
        if _active_child is not None:
            raise WorkerStopped
        os._exit(128 + signum)
    signal.signal(signal.SIGTERM, terminate)


def process(command, env, output, timeout=None):
    """Reap each process group before the next phase, including on interruption."""
    global _active_child, _protecting_child, _pending_signal
    child = None
    try:
        _protecting_child = True
        try:
            child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=output,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            _active_child = child
        finally:
            _protecting_child = False
        if _pending_signal is not None:
            pending, _pending_signal = _pending_signal, None
            signal.raise_signal(pending)
        status = child.wait(timeout=timeout)
    except WorkerStopped:
        _pending_signal = signal.SIGTERM
    finally:
        _protecting_child = True
        try:
            stop_child(child)
        finally:
            _active_child = None
            _protecting_child = False
        if _pending_signal is not None:
            pending, _pending_signal = _pending_signal, None
            signal.raise_signal(pending)
    if status:
        raise RuntimeError(f'{command[0]} exited {status}')


def verify_pool(arguments, workers):
    existing = {child.pid for child in multiprocessing.active_children()}
    pool = concurrent.futures.ProcessPoolExecutor(max_workers=workers,
             mp_context=multiprocessing.get_context('spawn'), initializer=verifier_init)
    try:
        yield from pool.map(verify_one, arguments)
    except BaseException:
        pool.shutdown(wait=False, cancel_futures=True)
        children = [child for child in multiprocessing.active_children() if child.pid not in existing]
        for child in children:
            child.terminate()
        for child in children:
            child.join(timeout=10)
            if child.is_alive():
                child.kill()
                child.join()
        raise
    else:
        pool.shutdown(wait=True)


def runtime_libraries(output):
    libraries = set()
    for line in output.splitlines():
        item = line.split('=>', 1)[-1].strip().rsplit(' (', 1)[0].strip()
        if item.startswith('/') and Path(item).is_file():
            libraries.add(str(Path(item).resolve()))
    return sorted(libraries)


def build(config, directory):
    env = environment(config)
    command = ['cargo', 'bench', '--locked', '--bench', 'profile', '--no-run', '--message-format=json']
    if config['sdk_variable']:
        command += ['--features', config['backend']]
    output = directory / f"build-{config['backend']}-{config['position']}.log"
    with output.open('w') as log:
        process(command, env, log)
    artifacts = []
    for line in output.read_text().splitlines():
        if line.startswith('{'):
            message = json.loads(line)
            if message.get('reason') == 'compiler-artifact' and message['target']['name'] == 'profile' and message.get('executable'):
                artifacts.append(message['executable'])
    if len(artifacts) != 1:
        raise ValueError('Cargo did not report exactly one profile executable')
    binary = artifacts[0]
    with output.open('a') as log:
        process([binary, 'ready'], env, log)
        process(['ldd', binary], env, log)
    config.update(binary=binary, binary_sha256=sha256(binary), build_log=output.name,
                  runtime_libraries=runtime_libraries(output.read_text()))
    # Keep exact local identities in this ignored run, never in the catalog.
    config['runtime_environment'] = {k: env[k] for k in ('LD_LIBRARY_PATH', 'RDI_DATADIR') if k in env}
    return env


def raw_files(job, directory):
    root = directory / job['raw']
    required = ('sample.json', 'estimates.json', 'benchmark.json')
    files = []
    for name in required:
        matches = list(root.glob(f'**/new/{name}'))
        if len(matches) != 1:
            raise ValueError(f'expected one fresh Criterion {name}, got {len(matches)}')
        files.append(str(matches[0].relative_to(directory)))
    sample = json.loads((directory / files[0]).read_text())
    if len(sample['times']) != SETTINGS['sample_size'] or any(t <= 0 for t in sample['times']):
        raise ValueError('Criterion samples are absent or incomplete')
    return files


def verify_one(arguments):
    run_path, index, env = arguments
    # A spawned process loads the independent model, then runs the same built workload.
    run = json.loads(Path(run_path).read_text())
    job = run['jobs'][index]
    config = run['configurations'][job['configuration']]
    result = dict(started_at=now())
    start = time.monotonic()
    try:
        with tempfile.TemporaryFile(mode='w+') as output:
            process([config['binary'], 'verify', run_path, str(index)], env, output)
            output.seek(0)
            rows = (json.loads(line[4:]) for line in output if line.startswith('OPB:'))
            result.update(oracle.check(job['input']['case'], job['input'], rows), status='passed')
    except Exception as error:
        result.update(status='failed', error=str(error))
    result.update(finished_at=now(), wall_seconds=time.monotonic() - start)
    return index, result


def source_identity():
    paths = sorted(p for folder in ('src', 'native', 'benches', 'tools/opb')
                   for p in (ROOT / folder).rglob('*') if p.is_file() and '__pycache__' not in p.parts)
    paths += [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'build.rs']
    paths += sorted((ROOT / 'tools/repo').glob('*_build.rs'))
    return {str(p.relative_to(ROOT)): sha256(p) for p in paths}


def verify_identity(data):
    error = None
    try:
        if source_identity() != data['source_files']:
            error = 'source/model changed during the series'
        elif any(sha256(config['binary']) != config['binary_sha256'] for config in data['configurations']):
            error = 'built workload changed during the series'
    except OSError as problem:
        error = f'source/build identity unavailable: {problem}'
    if error:
        for job in data.get('jobs', []):
            if job['verification']['status'] == 'passed':
                job['verification'].update(status='failed', error=error)
        raise ValueError(error)


def run(args, cases):
    destination = Path(args.output or ROOT / 'target/opb-runs' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')).resolve()
    if destination.is_relative_to(ROOT) and destination.relative_to(ROOT).parts[0] not in ('target', 'tmp'):
        raise ValueError('outputs inside the checkout must be under ignored target/ or tmp/')
    # Fresh paths prevent stale Criterion data from being mistaken for a new measurement.
    destination.mkdir(parents=True, exist_ok=False)
    configs = configurations(args)
    jobs = []
    for n, config in enumerate(configs):
        for case in cases:
            binding = bindings.fixture_binding(case['fixture'], config['backend'])
            artifact = ROOT / 'fixtures' / binding['fixture'] / binding['artifact']['file']
            jobs.append(dict(case_id=case['id'], configuration=n, input=bindings.inputs(case, config['backend'], artifact),
                             artifact_identity=binding['artifact'], raw=f'raw/{len(jobs):04d}',
                             measurement=dict(status='not-run'), verification=dict(status='not-run')))
    workers = args.verify_workers if args.verify_workers is not None else cpu_count()
    data = dict(schema=1, workload_version='opb-core-2', directory=str(destination), created_at=now(),
                source_revision=os.environ.get('ONDAS_SOURCE_REVISION'),
                fixtures_revision=bindings.QUALIFICATION['fixtures_revision'], source_files=source_identity(),
                criterion=dict(version=next(p['version'] for p in tomllib.loads((ROOT / 'Cargo.lock').read_text())['package'] if p['name'] == 'criterion'), **SETTINGS), configurations=configs, jobs=jobs,
                verification_workers=dict(requested=args.verify_workers, available_cpus=cpu_count(), resolved=workers, effective=0),
                measurement_timeout_seconds=args.timeout,
                phases=dict(readiness=dict(started_at=now())))
    write_run(data)
    print(f'Run: {destination / "run.json"}', flush=True)
    envs = []
    try:
        # Readiness is complete for the whole series before the first measurement.
        checked = set()
        for config in configs:
            for case in cases:
                key = (case['fixture'], config['backend'])
                if key not in checked:
                    input_ready(bindings.fixture_binding(*key))
                    checked.add(key)
            envs.append(build(config, destination))
            write_run(data)
        data['phases']['readiness']['finished_at'] = now()
        data['phases']['measurement'] = dict(started_at=now())
        write_run(data)
        for index, job in enumerate(jobs):
            print(f'Measure {index + 1}/{len(jobs)} {configs[job["configuration"]]["label"]} {job["case_id"]}', flush=True)
            state = job['measurement']
            state['started_at'] = now()
            start = time.monotonic()
            try:
                log_path = destination / f'measure-{index:04d}.log'
                state['log'] = log_path.name
                with log_path.open('w') as log:
                    process([configs[job['configuration']]['binary'], 'measure', str(destination / 'run.json'), str(index)], envs[job['configuration']], log, args.timeout)
                state.update(status='measured', raw_files=raw_files(job, destination))
            except Exception as error:
                state.update(status='failed', error=str(error))
                if isinstance(error, subprocess.TimeoutExpired):
                    state['reason'] = 'timeout'
            state.update(finished_at=now(), wall_seconds=time.monotonic() - start)
            write_run(data)
        data['phases']['measurement']['finished_at'] = now()
        write_run(data)
        verify_identity(data)
        # The only barrier: all child measurement processes have already exited and been reaped.
        eligible = [i for i, j in enumerate(jobs) if j['measurement']['status'] == 'measured']
        if args.no_verify:
            for i in eligible:
                jobs[i]['verification'] = dict(status='unchecked', reason='--no-verify')
        elif eligible:
            effective = min(workers, len(eligible))
            data['verification_workers']['effective'] = effective
            data['phases']['verification'] = dict(started_at=now())
            write_run(data)
            arguments = [(str(destination / 'run.json'), i, envs[jobs[i]['configuration']]) for i in eligible]
            for index, result in verify_pool(arguments, effective):
                jobs[index]['verification'] = result
                print(f'Verify {index + 1}/{len(jobs)} {result["status"]} {jobs[index]["case_id"]}', flush=True)
                write_run(data)
            verify_identity(data)
            data['phases']['verification']['finished_at'] = now()
    except (Exception, KeyboardInterrupt) as error:
        data['error'] = str(error) or type(error).__name__
        print(f'error: {data["error"]}', file=sys.stderr)
    finally:
        data['finished_at'] = now()
        write_run(data)
    measured = sum(j['measurement']['status'] == 'measured' for j in jobs)
    passed = sum(j['verification']['status'] == 'passed' for j in jobs)
    print(f'{measured}/{len(jobs)} measured; {passed}/{len(jobs)} passed', flush=True)
    return 0 if measured == len(jobs) and (args.no_verify or passed == len(jobs)) and 'error' not in data else 1


def positive(text):
    value = int(text)
    if value <= 0:
        raise argparse.ArgumentTypeError('must be positive')
    return value


def main(argv=None):
    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGINT, interrupt)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('list', 'run'):
        command = commands.add_parser(name)
        selection = command.add_mutually_exclusive_group()
        selection.add_argument('--quick', action='store_true', help='end-to-end smoke: OPB-POINT-028 (default for run)')
        selection.add_argument('--case', action='append', metavar='ID')
        selection.add_argument('--full', action='store_true', help='all 354 OPB cases')
        if name == 'run':
            command.add_argument('--backend', choices=(*bindings.BACKENDS, 'all'), required=True)
            command.add_argument('--sweep', action='store_true', help='two passes over SDK HOMES: all timings, then verification')
            command.add_argument('--verify-workers', type=positive, metavar='N', help='default: available CPU count')
            command.add_argument('--no-verify', action='store_true', help='record timings as unchecked')
            command.add_argument('--output', help='new ignored output directory')
            timeout = command.add_mutually_exclusive_group()
            timeout.add_argument('--timeout', type=positive, default=600, metavar='SECONDS', help='wall limit per case/backend measurement (default: 600 seconds); verification has no timeout')
            timeout.add_argument('--no-timeout', dest='timeout', action='store_const', const=None, help='disable the measurement wall limit')
    args = parser.parse_args(argv)
    try:
        cases = selected_cases(args)
        if args.command == 'list':
            for case in cases:
                state = 'implemented' if case['id'] in bindings.IMPLEMENTED else 'not-implemented'
                print(f"{case['id']} {state} {case['boundary']} {case['workload']} {case['fixture']}")
            return 0
        if not cases:
            raise ValueError('empty case selection')
        return run(args, cases)
    except (ValueError, OSError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    sys.exit(main())
