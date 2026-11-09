"""Read saved OPB records and generate an offline heat map."""
import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import sys
from urllib.parse import quote


ROOT = Path(__file__).resolve().parents[2]


def fingerprint(value):
    encoded = json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True)
    return hashlib.sha256(encoded.encode()).hexdigest()


def saved_file(root, relative):
    path = Path(relative)
    if path.is_absolute() or '..' in path.parts or '\\' in str(relative):
        raise ValueError(f'unsafe saved path: {relative}')
    resolved = (root / path).resolve()
    if not resolved.is_relative_to(root):
        raise ValueError(f'saved path leaves run directory: {relative}')
    return resolved


def href(path, output):
    return quote(Path(os.path.relpath(path, output.parent)).as_posix(), safe='/..')


def number(value, zero=False):
    if (isinstance(value, bool) or not isinstance(value, (int, float))
            or not math.isfinite(value) or (value < 0 if zero else value <= 0)):
        raise ValueError('invalid timing or sample number')
    return value


def protect_evidence(root, manifest, data, output):
    output = Path(output).resolve()
    identity = output.stat() if output.exists() else None
    files = [manifest, root / 'context.json']
    relative = [c.get('build_log') for c in data['configurations']]
    for job in data['jobs']:
        for record in (job.get('measurement', {}), job.get('verification', {})):
            relative.append(record.get('log'))
            if isinstance(record.get('raw_files'), list):
                relative.extend(record['raw_files'])
        if job.get('raw'):
            try:
                raw = saved_file(root, job['raw'])
            except (ValueError, TypeError):
                continue
            if output.is_relative_to(raw):
                raise ValueError('output would overwrite saved raw evidence')
    for name in filter(None, relative):
        try:
            files.append(saved_file(root, name))
        except (ValueError, TypeError):
            continue
    for file in files:
        stat = file.stat() if identity and file.exists() else None
        if file == output or (stat and (stat.st_dev, stat.st_ino) == (identity.st_dev, identity.st_ino)):
            raise ValueError('output would overwrite saved evidence')


def timing(root, job, backend, settings, output):
    references = job['measurement'].get('raw_files', [])
    expected = {str(Path(job['raw']) / backend / job['case_id'] / 'new' / name)
                for name in ('sample.json', 'benchmark.json', 'estimates.json')}
    if len(references) != 3 or set(references) != expected:
        raise ValueError('raw references do not match case/backend')
    raw = {Path(ref).stem: json.loads(saved_file(root, ref).read_text()) for ref in references}
    if raw['benchmark']['full_id'] != backend + '/' + job['case_id']:
        raise ValueError('Criterion benchmark ID does not match case/backend')
    sample = raw['sample']
    count = settings['sample_size']
    if len(sample['times']) != count or len(sample['iters']) != count or count < 2:
        raise ValueError('incomplete Criterion samples')
    samples = [number(t) / number(i) for t, i in zip(sample['times'], sample['iters'])]
    estimate = raw['estimates']['mean']
    mean = number(estimate['point_estimate'])
    ci = estimate['confidence_interval']
    lower, upper = number(ci['lower_bound']), number(ci['upper_bound'])
    if lower > upper or not 0 < ci['confidence_level'] < 1:
        raise ValueError('invalid confidence interval')
    std = number(raw['estimates']['std_dev']['point_estimate'], zero=True)
    return dict(mean=mean, lower=lower, upper=upper, confidence=ci['confidence_level'],
                cv=std / mean, samples=samples,
                links={Path(ref).stem: href(saved_file(root, ref), output) for ref in references})


def read_run(path, output):
    path = Path(path).resolve()
    if path.is_dir():
        path /= 'run.json'
    root = path.parent
    data = json.loads(path.read_text())
    if not isinstance(data, dict) or data.get('schema') != 1:
        raise ValueError(f'{path}: unsupported saved-run schema')
    if not isinstance(data.get('configurations'), list) or not isinstance(data.get('jobs'), list):
        raise ValueError(f'{path}: configurations and jobs must be lists')
    protect_evidence(root, path, data, output)
    rows = [dict(backend=c['backend'], label=c.get('label', c['backend']), config=c, cells={})
            for c in data['configurations']]
    for index, job in enumerate(data['jobs']):
        position = job['configuration']
        if type(position) is not int or not 0 <= position < len(rows):
            raise ValueError(f'{path}: invalid configuration index')
        row = rows[position]
        case = job['input']['case']
        case_id = job['case_id']
        if case['id'] != case_id or job['input']['backend'] != row['backend']:
            raise ValueError(f'{path}: job ID/backend mismatch')
        if case_id in row['cells']:
            raise ValueError(f'{path}: duplicate case/configuration: {case_id}')
        m, v = job['measurement'], job['verification']
        concrete = dict(job['input'])
        concrete['artifact'] = Path(concrete['artifact']).name
        summary = {k: value for k, value in concrete.items() if not isinstance(value, (dict, list))}
        summary.update(sources=len(concrete.get('source_ids', [])),
                       unique_sources=len(set(concrete.get('source_ids', []))),
                       time_positions=len(concrete.get('times', [])))
        cell = dict(job=index, case=case, case_hash=fingerprint(case), input_hash=fingerprint(concrete),
                    artifact=job['artifact_identity'], input_summary=summary,
                    measurement=m, verification=v, status='not run', timing=None)
        # Keep diagnostic states and wall times, but embed no redundant raw-reference lists.
        cell['measurement'] = {k: val for k, val in m.items() if k not in ('raw_files', 'started_at', 'finished_at')}
        cell['verification'] = {k: val for k, val in v.items() if k not in ('started_at', 'finished_at')}
        if m['status'] == 'failed':
            cell['status'] = 'timeout' if m.get('reason') == 'timeout' else 'measurement failed'
        elif m['status'] == 'measured':
            cell['status'] = 'passed' if v['status'] == 'passed' else (
                'verification failed' if v['status'] == 'failed' else 'unchecked')
            try:
                cell['timing'] = timing(root, job, row['backend'], data['criterion'], output)
            except (OSError, ValueError, KeyError, TypeError) as error:
                cell.update(status='invalid raw', data_error=str(error))
            if cell['status'] == 'passed' and (data.get('error') or not data.get('finished_at')):
                cell['status'] = 'run error' if data.get('error') else 'provisional'
        for key, relative in [('log', m.get('log')), ('raw_directory', job.get('raw'))]:
            if relative:
                try:
                    file = saved_file(root, relative)
                    if file.exists():
                        cell[key] = href(file, output)
                except ValueError as error:
                    cell['link_error'] = str(error)
        row['cells'][case_id] = cell
    meta = {k: val for k, val in data.items() if k not in ('jobs', 'configurations', 'directory')}
    workload_files = {k: v for k, v in data.get('source_files', {}).items()
                      if k == 'benches/profile.rs' or k.startswith('benches/opb/')}
    context = root / 'context.json'
    return dict(name=root.name, href=href(path, output), rows=rows, meta=meta,
                workload_hash=fingerprint(workload_files) if workload_files else None,
                context=href(context, output) if context.is_file() else None)


def ideal_run(source, source_index):
    """Build a synthetic view from per-case minimum verified means."""
    if any(row['backend'] == 'ideal' for row in source['rows']):
        raise ValueError('ideal is reserved for the synthetic backend')
    cells = {}
    for case_id in dict.fromkeys(case for row in source['rows'] for case in row['cells']):
        candidates = [(row, row['cells'][case_id]) for row in source['rows']
                      if case_id in row['cells'] and row['cells'][case_id]['status'] == 'passed'
                      and row['cells'][case_id]['timing']]
        if not candidates:
            continue
        if len({cell['case_hash'] for _, cell in candidates}) != 1:
            raise ValueError(f'cannot synthesize different case definitions: {case_id}')
        row, winner = min(candidates, key=lambda pair: pair[1]['timing']['mean'])
        cell = copy.deepcopy(winner)
        cell.update(status='synthetic', measurement=dict(status='synthetic'),
                    verification=dict(status='source passed'),
                    synthetic_source=dict(backend=row['backend'], label=row['label'],
                                          job=winner['job'], manifest=source['href']))
        cells[case_id] = cell
    if not cells:
        raise ValueError('ideal requires at least one verified timing')
    meta = {key: copy.deepcopy(source['meta'][key]) for key in (
        'schema', 'workload_version', 'source_revision', 'fixtures_revision', 'source_files',
        'criterion', 'created_at', 'finished_at') if key in source['meta']}
    meta['derivation'] = dict(kind='minimum verified mean per case', source=source['name'],
                              measurements_executed=False,
                              note='Samples and CI belong to the winning source measurement.')
    rows = copy.deepcopy(source['rows'])
    rows.append(dict(backend='ideal', label='ideal · synthetic', synthetic=True,
                     config=dict(backend='ideal', synthetic=True), cells=cells))
    return dict(name=source['name'] + ' + ideal', href=source['href'], context=source['context'],
                meta=meta, workload_hash=source['workload_hash'], rows=rows, rows_from=source_index)


def comparisons(runs):
    """Explain which saved cells may be compared; library revisions may differ."""
    for current in runs:
        for row in current['rows']:
            for case_id, cell in row['cells'].items():
                cell['comparisons'] = []
                for previous in runs:
                    matches = [r for r in previous['rows'] if r['backend'] == row['backend']]
                    if len(matches) > 1:
                        matches = [r for r in matches if r['config'].get('sdk_home') == row['config'].get('sdk_home')]
                    other = matches[0]['cells'].get(case_id) if len(matches) == 1 else None
                    reasons, conditions = [], []
                    if (cell['status'] not in ('passed', 'synthetic') or not other
                            or other['status'] not in ('passed', 'synthetic')):
                        reasons.append('No verified measurement on one or both sides')
                    if other:
                        for label, a, b in [('Case parameters / timer boundary', cell['case_hash'], other['case_hash']),
                                            ('Exact inputs', cell['input_hash'], other['input_hash']),
                                            ('Waveform payload', cell['artifact'], other['artifact'])]:
                            if a != b:
                                reasons.append(label + ' differ')
                    for key, label in [('workload_version', 'Workload version'), ('criterion', 'Criterion settings')]:
                        if current['meta'].get(key) != previous['meta'].get(key):
                            reasons.append(label + ' differ')
                    if not current['workload_hash'] or current['workload_hash'] != previous['workload_hash']:
                        reasons.append('Measured workload source differs or is unknown')
                    if len(matches) == 1:
                        a, b = row['config'], matches[0]['config']
                        sdk_a = (a.get('sdk_home'), [p for p in a.get('runtime_libraries', [])
                                                   if not Path(p).name.startswith('libondas_')])
                        sdk_b = (b.get('sdk_home'), [p for p in b.get('runtime_libraries', [])
                                                   if not Path(p).name.startswith('libondas_')])
                        if sdk_a != sdk_b:
                            conditions.append('SDK selection / loaded libraries differ')
                    if current['meta'].get('fixtures_revision') != previous['meta'].get('fixtures_revision'):
                        conditions.append('Fixture revision differs; compare exact payload identities')
                    value = dict(reasons=reasons, conditions=conditions)
                    if not reasons:
                        value.update(ratio=cell['timing']['mean'] / other['timing']['mean'],
                                     reference={k: other['timing'][k] for k in ('mean', 'lower', 'upper', 'cv')},
                                     ci_overlap=max(cell['timing']['lower'], other['timing']['lower']) <=
                                     min(cell['timing']['upper'], other['timing']['upper']))
                    cell['comparisons'].append(value)


def render(paths, output, ideal_of=None):
    output = Path(output).resolve()
    if output.is_relative_to(ROOT) and output.relative_to(ROOT).parts[0] not in ('target', 'tmp'):
        raise ValueError('reports inside the checkout must be under ignored target/ or tmp/')
    paths = list(paths)
    runs = [read_run(path, output) for path in paths]
    if ideal_of is not None:
        def manifest(path):
            path = Path(path).resolve()
            return path / 'run.json' if path.is_dir() else path
        matches = [i for i, path in enumerate(paths) if manifest(path) == manifest(ideal_of)]
        if len(matches) != 1:
            raise ValueError('--ideal-of must identify exactly one input run')
        runs.append(ideal_run(runs[matches[0]], matches[0]))
    columns = list(dict.fromkeys(case_id for run in runs for row in run['rows'] for case_id in row['cells']))
    if not columns:
        raise ValueError('saved runs contain no cases')
    comparisons(runs)
    backends = list(dict.fromkeys(row['backend'] for run in runs for row in run['rows']))
    definitions = {}
    pools = {key: [] for key in ('artifacts', 'inputs', 'comparisons')}
    indexes = {key: {} for key in pools}

    def intern(kind, value):
        key = json.dumps(value, sort_keys=True, separators=(',', ':'))
        if key not in indexes[kind]:
            indexes[kind][key] = len(pools[kind])
            pools[kind].append(value)
        return indexes[kind][key]

    for run in runs:
        for row in run['rows']:
            for cell in row['cells'].values():
                definitions[cell['case_hash']] = cell.pop('case')
                cell['artifact'] = intern('artifacts', cell['artifact'])
                cell['input_summary'] = intern('inputs', cell['input_summary'])
                cell['comparisons'] = [intern('comparisons', value) for value in cell['comparisons']]
                if cell['timing']:
                    links = cell['timing'].pop('links')
                    cell['timing']['raw_prefix'] = str(Path(links['sample']).parent) + '/'
    saved_runs = []
    # Store inherited rows once in the embedded display data.
    for run in runs:
        if 'rows_from' in run:
            if run['rows'][:-1] != runs[run['rows_from']]['rows']:
                raise ValueError('synthetic source rows changed during rendering')
            saved_runs.append({**run, 'rows': run['rows'][-1:]})
        else:
            saved_runs.append(run)
    data = json.dumps(dict(runs=saved_runs, columns=columns, backends=backends, definitions=definitions, **pools),
                      separators=(',', ':'), allow_nan=False)
    # JSON is script data, never HTML. Escape characters that can terminate its element.
    data = data.replace('<', '\\u003c').replace('>', '\\u003e').replace('&', '\\u0026')
    template = Path(__file__).with_name('viewer.html').read_text()
    output.parent.mkdir(parents=True, exist_ok=True)
    script = Path(__file__).with_name('viewer.js').read_text()
    output.write_text(template.replace('/*OPB_SCRIPT*/', script, 1).replace('/*OPB_DATA*/null', data, 1))
    return runs


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('runs', nargs='+', help='saved run.json files or their directories')
    parser.add_argument('--output', required=True, help='offline HTML report (keep beside its run directories)')
    parser.add_argument('--ideal-of', metavar='RUN_JSON', help='append a synthetic run with per-case minimum verified means from this input run')
    args = parser.parse_args(argv)
    try:
        runs = render(args.runs, args.output, args.ideal_of)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f'error: {error}\n')
    print(f'{args.output}: {len(runs)} saved runs; no measurements executed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
