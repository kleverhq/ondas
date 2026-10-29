"""Independent streaming comparisons from integer corpus schedules."""
import bisect
import functools
import hashlib
import heapq
import itertools
import json

from . import bindings, model


def digest(paths):
    return hashlib.sha256(json.dumps(sorted(paths), separators=(',', ':'), ensure_ascii=True).encode()).hexdigest()


@functools.lru_cache(maxsize=128)
def history(profile, logical):
    return tuple(model.expected_events(profile, model.by_id(profile)[logical]))


def dependencies(case):
    ids = bindings.selected_sources(case)
    shape = case['selection']
    if shape in ('active-bit', 'stable-high-slice'):
        return [[(ids[0], 0, 0) if shape == 'active-bit' else (ids[0], 127, 64)]]
    if shape in ('concat8x64', 'concat8-first', 'concat8-identical-repeat'):
        return [[(s, None, None) for s in ids]]
    if shape in ('nested16-result-bit', 'nested16-first'):
        return [[(s, None, None) for s in ids]] if shape == 'nested16-first' else [[(ids[-1], 0, 0)]]
    if shape == 'overlapping64-of1024':
        return [[(ids[0], i + 63, i)] for i in range(100)]
    if shape == 'query-shared-base':
        return [[(ids[0], 0, 0)], [(ids[1], None, None)], [(ids[2], None, None)]]
    return [[(s, None, None)] for s in ids]


def sliced(bits, msb, lsb):
    return bits if msb is None else bits[len(bits) - msb - 1:len(bits) - lsb]


def projection(case, tick):
    return [''.join(sliced(model.value_at(case['fixture'], s, tick), hi, lo) for s, hi, lo in deps)
            for deps in dependencies(case)]


def last_change(profile, logical, tick, msb, lsb):
    d = model.by_id(profile)[logical]
    p = model.config(profile)
    dt = p['t1'] // p['steps']
    steps = model.update_steps(p, d)
    n = bisect.bisect_right(steps, tick // dt)
    if d['kind'] == 'low' and msb is not None and lsb > 0:
        return None
    current = sliced(model.render(p, d, n, steps[n - 1] if n else 0), msb, lsb)
    while n:
        previous = sliced(model.render(p, d, n - 1, steps[n - 2] if n > 1 else 0), msb, lsb)
        if previous != current:
            return steps[n - 1] * dt
        n -= 1
    return None


def source_point(profile, deps, tick):
    bits = ''.join(sliced(model.value_at(profile,s,tick),hi,lo) for s,hi,lo in deps)
    changed = max((last_change(profile,s,tick,hi,lo) or 0 for s,hi,lo in deps),default=0) or None
    return bits,changed


def point(case, tick):
    return [source_point(case['fixture'],deps,tick) for deps in dependencies(case)]


def sample_row(profile, deps, slot, tick):
    bits,changed = source_point(profile,deps,tick)
    return dict(kind='sample',slot=slot,time=tick,bits=bits,changed_at=changed)


def sample_rows(case, tick):
    return [sample_row(case['fixture'],deps,slot,tick) for slot,deps in enumerate(dependencies(case))]


def slot_history(profile, deps):
    def stream(i, s, hi, lo):
        for tick, bits in model.expected_events(profile, model.by_id(profile)[s]):
            yield tick, i, sliced(bits, hi, lo)
    if all(model.by_id(profile)[logical]['kind'] == 'low' and hi is not None and lo > 0 for logical, hi, lo in deps):
        yield 0, ''.join(sliced(model.value_at(profile,s,0),hi,lo) for s,hi,lo in deps)
        return
    streams = [stream(i, *dep) for i, dep in enumerate(deps)]
    parts = [''] * len(deps)
    previous = None
    for tick, group in itertools.groupby(heapq.merge(*streams), key=lambda r: r[0]):
        for _, i, bits in group:
            parts[i] = bits
        value = ''.join(parts)
        if value != previous:
            yield tick, value
            previous = value


def range_iter(case, start, end, trace=False):
    if start:
        for i, (bits, changed) in enumerate(point(case, start - 1)):
            yield dict(kind='initial', slot=i, bits=bits, changed_at=changed)
    def stream(slot, deps):
        for tick, bits in slot_history(case['fixture'], deps):
            if tick > end:
                break
            if tick >= start:
                yield tick, slot, bits
    streams = [stream(i, deps) for i, deps in enumerate(dependencies(case))]
    changes = itertools.chain.from_iterable(streams) if trace else heapq.merge(*streams)
    n = case['parameters'].get('n', 100 if case['workload'] in ('inspect-window', 'break-then-independent-point') else 0)
    if n:
        changes = itertools.islice(changes, n)
    for tick, slot, bits in changes:
        yield dict(kind='change', slot=slot, time=tick, bits=bits)


def range_rows(case, start, end):
    rows = list(range_iter(case, start, end))
    return [r for r in rows if r['kind'] == 'initial'], [r for r in rows if r['kind'] == 'change']


def query_ticks(case, job_input):
    p = model.config(case['fixture'])
    work = case['workload']
    drivers = case['parameters']['drivers']
    dt = p['t1'] // p['steps']
    ticks = set()
    for i in range(drivers):
        period = 16 + i * 2
        for ordinal, step in enumerate(range(period, p['steps'] + 1, 2 * period), 1):
            if not job_input['start'] <= step * dt <= job_input['end']:
                continue
            if work == 'reject-all':
                continue
            if work not in ('accept-all-edges', 'current-previous-repeat', 'many-drivers') and ordinal % 16:
                continue
            ticks.add(step * dt)
    ordered = sorted(ticks)
    return ordered[:1] if work == 'first-accepted-after-rejects' else ordered


def expected(case, job_input):
    work = case['workload']
    if work in ('point-owned', 'computed-point', 'whole-base-reference-point', 'repeated-handle-point',
                'overlapping-slices-point', 'four-state-point', 'value-shape-point', 'point-borrowed',
                'constant-answer-point', 'after-prior-point', 'same-wave-new-selection-point'):
        yield from sample_rows(case, job_input['point'])
    elif work in ('explicit-times', 'explicit-times-order', 'explicit-single-time', 'same-grid-repeated-points',
                  'same-wave-repeat-batch', 'constant-answer-batch', 'point-session'):
        for j, tick in enumerate(job_input['times']):
            c = case
            if job_input.get('session_choices'):
                c = dict(case, selection='regular64')
                ids = bindings.corpus(case['fixture'])['selections']['regular64']['sources']
                shift = job_input['session_choices'][j]
                c['_sources'] = (ids[shift:] + ids[:shift])[:case['parameters']['k']]
            yield from sample_rows(c, tick)
    elif work in ('saved-trace-samples_at', 'saved-trace-sample'):
        for slot,deps in enumerate(dependencies(case)):
            for tick in job_input['times']:
                yield sample_row(case['fixture'],deps,slot,tick)
    elif work in ('full-window', 'full-corpus', 'first-changes', 'first-change-end-bound', 'materialize-traces',
                  'computed-scan', 'whole-base-reference-scan', 'repeated-handle-scan', 'overlapping-slices-scan',
                  'four-state-scan', 'value-shape-scan', 'constant-answer-prefix', 'after-prior-prefix'):
        yield from range_iter(case, job_input['start'], job_input['end'], work == 'materialize-traces')
    elif case['id'].startswith('OPB-QUERY-'):
        if work == 'driver-payload-base-sharing':
            yield sharing_selection(case)
        drivers = case['parameters']['drivers']
        for tick in query_ticks(case, job_input):
            yield dict(kind='accepted', time=tick)
            yield from (r for r in sample_rows(case, tick) if r['slot'] > drivers)
        yield dict(kind='query-end', stopped=work == 'first-accepted-after-rejects')
    elif work in ('inspect-window', 'break-then-independent-point'):
        if work == 'inspect-window':
            yield from sample_rows(case, job_input['point'])
        yield from range_iter(case, job_input['start'], job_input['end'])
        c = case
        if work == 'break-then-independent-point':
            c = dict(case, selection='prior')
        yield from sample_rows(c, job_input['late_point'])


def compare(actual, want):
    count, known, unknown = 0, 0, 0
    for index, (a, b) in enumerate(itertools.zip_longest(actual, want)):
        if b is not None and 'changed_at' in b and a is not None and a.get('changed_at', 'absent') is None:
            b['changed_at'] = None
        if a != b:
            raise ValueError(f'observation {index}: got {a}, expected {b}')
        count += 1
        if 'changed_at' in a:
            known += a['changed_at'] is not None
            unknown += a['changed_at'] is None
    return dict(records=count, changed_at_known=known, changed_at_unknown=unknown)


def without_done(rows):
    done = False
    for row in rows:
        if done:
            raise ValueError('observations after done')
        if row == {'kind': 'done'}:
            done = True
        else:
            yield row
    if not done:
        raise ValueError('incomplete verification stream')


def canonical_scan(rows):
    changes = False
    last = -1
    slots = -1
    bucket = []
    for row in rows:
        if row['kind'] == 'initial':
            if changes or row['slot'] <= slots:
                raise ValueError('Initials out of selection order')
            slots = row['slot']
            yield row
        elif row['kind'] == 'change':
            changes = True
            if row['time'] < last:
                raise ValueError('unordered Changes')
            if row['time'] != last:
                yield from sorted(bucket, key=lambda r: r['slot'])
                bucket = []
                last = row['time']
            bucket.append(row)
        else:
            raise ValueError('unexpected range record')
    yield from sorted(bucket, key=lambda r: r['slot'])


def check(case, job_input, rows):
    work = case['workload']
    if case['id'].startswith('OPB-QUERY-'):
        return check_query(case, job_input, rows)
    if work in ('local-items', 'children-ready', 'browse-deep'):
        return check_local(case, job_input, rows)
    if work.startswith('metadata-'):
        return check_metadata(case, job_input, rows)
    if work in ('walk-dfs', 'walk-ready', 'flat-scopes', 'flat-variables'):
        return check_hierarchy(case, job_input, rows)
    if work in ('candidate-union', 'candidates-stable-slice'):
        return check_candidates(case, job_input, rows)
    rows = without_done(rows)
    if work == 'full-corpus':
        mapping = next(rows)
        paths = {tuple(bindings.native_path(d['path'], job_input['backend'])): d['id'] for d in model.inventory(case['fixture'])}
        observed = [tuple(p) for p in mapping['paths']]
        if mapping['kind'] != 'sources' or len(observed) != len(paths) or set(observed) != paths.keys():
            raise ValueError('full corpus signal set differs')
        case = dict(case, _sources=[paths[p] for p in observed])
    if work == 'flat-signals':
        return check_signals(case,job_input,rows)
    if work == 'open-drop':
        want = [dict(kind='open', backend=job_input['backend'], first=0, last=model.config(case['fixture'])['t1'],
                     timescale=None if job_input['backend'] == 'ghw-native' else [1, 'femtosecond'])]
    elif work in ('lookup-signal', 'constant-answer-lookup', 'signal-lookup-after-prior', 'text-lookup-signal'):
        want = [dict(kind='lookup', path=job_input['lookup_path'], width=64)]
    elif work in ('lookup-scope', 'text-lookup-scope'):
        want = [dict(kind='scope', path=job_input['lookup_path'])]
    elif work.startswith('missing-last-'):
        want = [dict(kind='missing',missing=True)]
    elif work.startswith('lookup-many-'):
        want = (dict(kind='scope',path=p) if work.endswith('scope') else dict(kind='lookup',path=p,width=64) for p in job_input['lookup_paths'])
    elif work == 'create-composition':
        want = itertools.chain([dict(kind='definition',width=sum(model.by_id(case['fixture'])[s]['width'] for s in bindings.selected_sources(case)))], sample_rows(case,job_input['point']))
    elif work == 'select-drop':
        k = case['parameters']['k']
        want = [dict(kind='selection', slots=k, bases=case['parameters']['bases'], widths=[64] * k, same_order=True)]
    else:
        want = expected(case, job_input)
        if work in ('full-window', 'full-corpus', 'first-changes', 'first-change-end-bound', 'computed-scan',
                    'whole-base-reference-scan', 'repeated-handle-scan', 'overlapping-slices-scan',
                    'four-state-scan', 'value-shape-scan', 'constant-answer-prefix', 'after-prior-prefix'):
            rows, want = canonical_scan(rows), canonical_scan(want)
    return compare(rows, want)


def check_candidates(case, job_input, rows):
    streams = ((tick for tick, _ in slot_history(case['fixture'], deps) if job_input['start'] <= tick <= job_input['end'])
               for deps in dependencies(case))
    required = iter(t for t, _ in itertools.groupby(heapq.merge(*streams)))
    expected_tick = next(required, None)
    previous, count = None, 0
    for row in without_done(rows):
        if row.get('kind') != 'candidate' or set(row) != {'kind', 'time'}:
            raise ValueError('unexpected candidate record')
        tick = row['time']
        if previous is not None and previous >= tick:
            raise ValueError('candidates are not strictly increasing')
        if not job_input['start'] <= tick <= job_input['end']:
            raise ValueError('candidate outside inclusive window')
        if expected_tick is not None and expected_tick < tick:
            raise ValueError(f'missing candidate tick: {expected_tick}')
        if expected_tick == tick:
            expected_tick = next(required, None)
        previous, count = tick, count + 1
    if expected_tick is not None:
        raise ValueError(f'missing candidate tick: {expected_tick}')
    return dict(records=count, candidates=count)


def path_checksum(path):
    encoded = json.dumps(path,separators=(',', ':'),ensure_ascii=True).encode()
    return int.from_bytes(hashlib.sha256(encoded).digest(),'big')


def checksum_hex(value):
    return f'{value % (1 << 256):064x}'


def check_hierarchy(case, job_input, rows):
    backend = job_input['backend']
    qualified = bindings.fixture_binding(case['fixture'], backend)
    expected = {} if case['workload'] == 'flat-scopes' else {
        tuple(bindings.native_path(item['path'], backend)): item['width'] for item in model.inventory(case['fixture'])}
    variables, extras, summary = 0, 0, None
    all_checksum, extra_checksum = 0, 0
    for row in without_done(rows):
        if row.get('kind') == 'variable' and summary is None:
            variables += 1
            path = tuple(row['path'])
            width = expected.pop(path, None)
            if backend == 'wdb-lib':
                item_checksum = path_checksum(path)
                all_checksum += item_checksum
            if width is None:
                extras += 1
                if backend == 'wdb-lib':extra_checksum += item_checksum
                if backend != 'wdb-lib':
                    raise ValueError(f'unexpected variable path: {path}')
            elif row['width'] != width:
                raise ValueError(f'wrong width at {path}: {row["width"]}/{width}')
        elif row.get('kind') == 'hierarchy' and summary is None:
            summary = row
        else:
            raise ValueError('unexpected hierarchy record')
    scopes = 0 if case['workload'] == 'flat-variables' else qualified['physical_scopes']
    variable_count = 0 if case['workload'] == 'flat-scopes' else qualified['recorded'] + qualified['unrecorded']
    want = dict(kind='hierarchy', scopes=scopes, variables=variable_count,
                scope_sha256=digest([]) if not scopes else qualified['scope_sha256'])
    extra_count = 0 if case['workload'] == 'flat-scopes' else qualified['unrecorded']
    if summary != want or expected or extras != extra_count or variables != variable_count:
        raise ValueError(f'hierarchy differs: summary={summary}, expected={want}, missing recorded={len(expected)}, extra variables={extras}')
    if backend == 'wdb-lib' and variable_count and (checksum_hex(all_checksum) != qualified['variable_path_checksum'] or checksum_hex(extra_checksum) != qualified['unrecorded_path_checksum']):
        raise ValueError('WDB declaration identity differs')
    return dict(records=variables + scopes, recorded_paths_checked=qualified['recorded'] if variable_count else 0, extra_native_variables=extras)


def check_metadata(case, job_input, rows):
    variables = {tuple(bindings.native_path(d['path'],job_input['backend'])):d['width'] for d in model.inventory(case['fixture'])}
    is_scope = case['workload'] == 'metadata-scopes'
    keys = ('kind','object','path','name','parent') if is_scope else ('kind','object','path','name','parent','width')
    def actual():
        for row in without_done(rows):
            optional = {'native_kind'} if is_scope else {'native_kind','encoding','range','direction','signedness','logic_domain','has_enum'}
            if set(row) != set(keys) | optional:
                raise ValueError('metadata fields missing or extra')
            yield {k:row[k] for k in keys}
    def want():
        for path in job_input['metadata_paths']:
            r = dict(kind='metadata',object='scope' if is_scope else 'variable',path=path,name=path[-1],parent=path[:-1] or None)
            if not is_scope:
                r['width'] = variables[tuple(path)]
            yield r
    return compare(actual(),want())


def check_local(case, job_input, rows):
    backend = job_input['backend']
    variables = {tuple(bindings.native_path(d['path'],backend)):d['width'] for d in model.inventory(case['fixture'])}
    qualified = bindings.fixture_binding(case['fixture'],backend)
    scopes = {p[:n] for p in variables for n in range(1,len(p))}
    scopes.update(tuple(p) for p in qualified['empty_extra_scopes'])
    target = tuple(job_input['scope_path'])
    parents = [target] if case['workload'] != 'browse-deep' else [target[:n] for n in range(1,len(target)+1)]
    expected_variables = []
    expected_scopes = []
    for parent in parents:
        expected_variables += [(p,w) for p,w in variables.items() if p[:-1] == parent]
        expected_scopes += [p for p in scopes if p[:-1] == parent]
    seen = []
    summary = None
    for row in without_done(rows):
        if row['kind'] == 'variable' and summary is None:
            seen.append((tuple(row['path']),row['width']))
        elif row['kind'] == 'hierarchy' and summary is None:
            summary = row
        else:
            raise ValueError('unexpected local hierarchy observation')
    want = dict(kind='hierarchy',scopes=len(expected_scopes),variables=len(expected_variables),scope_sha256=digest(expected_scopes))
    if backend == 'wdb-lib':
        key = 'browse' if case['workload'] == 'browse-deep' else case['parameters']['position']
        local = qualified['local_children'][key]
        want.update(scopes=local['scopes'],variables=local['variables'],scope_sha256=local['scope_sha256'])
        known = dict(expected_variables)
        unknown = []
        for path,width in seen:
            if path in known:
                if known.pop(path) != width:raise ValueError('wrong local recorded width')
            else:
                unknown.append(path)
        if known or len(expected_variables) != local['recorded_variables'] or len(unknown) != local['unrecorded_variables'] or digest(unknown) != local['unrecorded_path_sha256'] or digest([p for p,_ in seen]) != local['variable_path_sha256']:
            raise ValueError('local WDB declarations differ')
    elif sorted(seen) != sorted(expected_variables):
        raise ValueError('local recorded declarations differ')
    if summary != want:
        raise ValueError(f'local hierarchy differs: {summary}/{want}')
    return dict(records=len(seen)+want['scopes'])


def sharing_selection(case):
    return dict(kind='selection',slots=3,bases=case['parameters']['bases'],widths=[1,64,1024],same_order=True)


def check_query(case, job_input, rows):
    rows = iter(without_done(rows))
    if case['workload'] == 'driver-payload-base-sharing':
        compare([next(rows,None)],[sharing_selection(case)])
    drivers = case['parameters']['drivers']
    deps = dependencies(case)
    def ticks(dep):
        for tick,_ in slot_history(case['fixture'],dep):
            if tick > job_input['end']:
                break
            if tick >= job_input['start']:
                yield tick
    required = iter(t for t,_ in itertools.groupby(heapq.merge(*(ticks(dep) for dep in deps[:drivers]))))
    required_tick = next(required,None)
    accepted = iter(query_ticks(case,job_input))
    accepted_tick = next(accepted,None)
    previous, callbacks, accepted_count, records = -1,0,0,0
    ended = False
    for row in rows:
        if row['kind'] == 'query-end':
            stopped = case['workload'] == 'first-accepted-after-rejects'
            if row != dict(kind='query-end',stopped=stopped) or accepted_tick is not None or (not stopped and required_tick is not None):
                raise ValueError('query stop or completeness differs')
            if stopped and (accepted_count != 1 or previous != query_ticks(case,job_input)[0]):
                raise ValueError('query did not stop at first accepted edge')
            ended = True
            if next(rows,None) is not None:
                raise ValueError('observations after query end')
            break
        if row['kind'] != 'candidate' or not job_input['start'] <= row['time'] <= job_input['end'] or row['time'] <= previous:
            raise ValueError('invalid query candidate')
        tick = row['time']
        if required_tick is not None and required_tick < tick:
            raise ValueError('missing query driver candidate')
        if required_tick == tick:
            required_tick = next(required,None)
        if accepted_tick is not None and accepted_tick < tick:
            raise ValueError('missing accepted edge')
        previous, callbacks = tick,callbacks+1
        if case['workload'] == 'current-previous-repeat':
            probe_case = dict(case, _sources=bindings.selected_sources(case)[:drivers+1])
            want = []
            for t, slots in ((tick-1,range(drivers)),(tick,range(drivers+1)),(tick-1,range(drivers))):
                values = sample_rows(probe_case,t)
                want.extend(values[s] for s in slots)
            compare((next(rows,None) for _ in want),want)
            records += len(want)
        if accepted_tick == tick:
            expected_accept = dict(kind='accepted',time=tick)
            if next(rows,None) != expected_accept:
                raise ValueError('missing accepted edge marker')
            want = sample_rows(case,tick)[drivers+1:]
            compare((next(rows,None) for _ in want),want)
            records += len(want)+1
            accepted_count += 1
            accepted_tick = next(accepted,None)
    if not ended:
        raise ValueError('incomplete query stream')
    return dict(records=records+callbacks+1,candidates=callbacks,accepted_ticks=accepted_count)


def check_signals(case, job_input, rows):
    expected = {tuple(bindings.native_path(d['path'],job_input['backend'])):d['width'] for d in model.inventory(case['fixture'])}
    count = len(expected)
    qualified = bindings.fixture_binding(case['fixture'],job_input['backend'])
    if job_input['backend'] == 'wdb-lib':
        count = qualified['recorded']+qualified['unrecorded']
    seen,summary,total = 0,None,0
    for row in rows:
        if row['kind']=='signal' and summary is None:
            path=tuple(row['path'])
            if path in expected:
                if expected.pop(path)!=row['width']:raise ValueError('wrong signal width')
            elif job_input['backend']!='wdb-lib':raise ValueError('unexpected signal')
            if job_input['backend']=='wdb-lib':
                total+=path_checksum(path)
            seen+=1
        elif row['kind']=='signals' and summary is None:
            summary=row
        else:raise ValueError('unexpected signal inventory record')
    if expected or seen!=count or summary is None or set(summary)!={'kind','count','unique','same_set'} or not 0 < summary['count'] <= seen or not summary['unique'] or not summary['same_set']:
        raise ValueError(f'signal inventory differs: {summary}; missing={len(expected)}, declarations={seen}/{count}')
    if job_input['backend']=='wdb-lib' and checksum_hex(total)!=qualified['variable_path_checksum']:
        raise ValueError('signal declaration identity differs')
    return dict(records=seen,signals=summary['count'])
