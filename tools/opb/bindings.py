"""Concrete deterministic inputs for the finite OPB catalog."""
import functools
import json
import random
import re
from pathlib import Path

from . import layout, model

ROOT = Path(__file__).resolve().parent
CATALOG = json.loads((ROOT / 'cases.json').read_text())
QUALIFICATION = json.loads((ROOT / 'qualification.json').read_text())
BACKENDS = ('vcd-native', 'fst-lib', 'ghw-native', 'fsdb-lib', 'wlf-lib', 'shm-lib', 'wdb-lib')
IMPLEMENTED = frozenset(c['id'] for c in CATALOG['cases'])
QUICK = frozenset({'OPB-POINT-028'})


@functools.lru_cache(maxsize=5)
def corpus(profile):
    return layout.bindings(profile, list(model.inventory(profile)))


def native_path(path, backend):
    if backend == 'ghw-native':
        return [re.sub(r'^word\[([0-9]+)\]$', r'word(\1)', c) for c in path]
    if backend != 'wdb-lib':
        return path
    result = []
    i = 0
    while i < len(path):
        a = path[i]
        b = path[i + 1] if i + 1 < len(path) else None
        if ((a.startswith('word[') and b == 'data') or
            (a == 'branch' and b in ('left', 'right', 'child', 'side')) or
            (a, b) in (('endpoint', 'activity'), ('extra', 'tree'))):
            result.append(a + '.' + b)
            i += 2
        else:
            result.append(a)
            i += 1
    return result


def fixture_binding(profile, backend):
    fmt = backend.split('-')[0]
    return next(r for r in QUALIFICATION['bindings'] if r['format'] == fmt and r['profile'] == profile)


def selected_sources(case):
    if '_sources' in case:
        return case['_sources']
    p = case['parameters']
    selection = case['selection']
    definition = corpus(case['fixture'])['selections'][selection]
    sources = definition['sources']
    if selection == 'query-controls-payload' and p['drivers'] == 1:
        sources = definition['one_driver_sources']
    if selection == 'repeated64' or p.get('shape') == 'repeated':
        return sources[:1] * p['k']
    if selection in ('active-bit', 'stable-high-slice', 'concat8x64', 'nested16-result-bit',
                     'concat8-first', 'concat8-identical-repeat', 'nested16-first',
                     'overlapping64-of1024', 'query-shared-base', 'query-distinct-base', 'all-core'):
        return sources
    return sources[:p.get('k', len(sources))]


def window(t1, name):
    return {'early10': (0, t1 // 10 - 1), 'middle10': (t1 * 45 // 100, t1 * 55 // 100 - 1),
            'late10': (t1 * 90 // 100, t1), 'all': (0, t1)}[name]


def time_grid(profile, q, bounds, order):
    lo, hi = bounds
    rng = random.Random(model.config(profile)['seed'])
    times = [lo + (hi - lo) * j // (q - 1) for j in range(q)] if q > 1 else [lo]
    if order == 'descending':
        times.reverse()
    elif order == 'permuted':
        rng.shuffle(times)
    elif order == 'duplicates':
        unique = (q + 1) // 2
        grid = [lo + (hi - lo) * j // (unique - 1) for j in range(unique)] if unique > 1 else [lo]
        times = [grid[j // 2] for j in range(q)]
    elif order.startswith('random-'):
        bounds = window(model.config(profile)['t1'], {'random-early': 'early10', 'random-middle': 'middle10',
                        'random-late': 'late10', 'random-global-switch': 'all'}[order])
        times = [rng.randint(*bounds) for _ in range(q)]
    elif order == 'ping-pong':
        t1 = model.config(profile)['t1']
        times = [rng.randint(*window(t1, 'early10' if j % 2 == 0 else 'late10')) for j in range(q)]
    return times


def inputs(case, backend, artifact):
    profile = case['fixture']
    p = case['parameters']
    definition = corpus(profile)
    t1 = model.config(profile)['t1']
    point = t1 * {'t05': 5, 't50': 50, 't95': 95}.get(p.get('position'), 50) // 100
    lo, hi = window(t1, p.get('window', 'middle10'))
    work = case['workload']
    if work in ('first-changes', 'first-change-end-bound', 'after-prior-prefix', 'constant-answer-prefix',
                'inspect-window', 'break-then-independent-point'):
        lo, hi = point, t1
        if p.get('end') == 'middle10-end':
            hi = window(t1, 'middle10')[1]
    if work == 'point-session':
        lo, hi = 0, t1
    times = time_grid(profile, p['q'], (lo, hi), p.get('order', 'ascending')) if 'q' in p else []
    if work == 'explicit-single-time':
        times = [point]
    sources = selected_sources(case)
    inventory = model.by_id(profile)
    path = lambda s: native_path(inventory[s]['path'], backend)
    scope = definition['deep' if p.get('position') == 'deep' or work == 'browse-deep' else 'shallow']
    target = scope + ([] if work.endswith('scope') else ['local_value'])
    if work.startswith('missing-last-'):
        target[-1 if work.endswith('signal') else len(target):] = ['missing']
    scopes = list(dict.fromkeys(tuple(d['path'][:-1]) for d in inventory.values()))
    lookup_kind = 'scope' if work.endswith('scope') else 'signal'
    many = [native_path(list(s) if lookup_kind == 'scope' else d['path'], backend)
            for s, d in zip(scopes, inventory.values())][:p.get('k', 0)] if work.startswith('lookup-many-') else []
    variants = []
    if work == 'point-session' and p['order'] == 'random-global-switch':
        regular = definition['selections']['regular64']['sources']
        k = p['k']
        for j in range(3):
            variants.append([path(s) for s in (regular[j:] + regular[:j])[:k]])
    metadata = []
    if work.startswith('metadata-'):
        native_variables = [native_path(d['path'], backend) for d in inventory.values()]
        all_scopes = sorted({tuple(path[:n]) for path in native_variables for n in range(1, len(path))})
        metadata = [list(s) for s in all_scopes[:p['k']]] if work == 'metadata-scopes' else [path(s) for s in list(inventory)[:p['k']]]
    independent_ids = definition['independent_point'][:100] if work == 'break-then-independent-point' else []
    history = case['history']
    prior_ids = definition['background_prior'][:100] if history in ('distant-disjoint', 'background-10000') else sources if history in ('identical', 'neighbor-same', 'identical-point-drop-selection', 'identical-batch-drop-selection', 'identical-batch-retain-selection') else []
    prior_point = point - 1 if history == 'neighbor-same' else point if history.startswith('identical') else t1 // 20
    prior_start = point - (t1 // model.config(profile)['steps']) if history == 'neighbor-same' else 0 if history == 'background-10000' else t1 // 20 if history == 'distant-disjoint' else lo
    return dict(case=case, backend=backend, artifact=str(artifact), source_ids=sources,
                paths=[path(s) for s in sources], lookup_path=native_path(target, backend),
                scope_path=native_path(scope, backend), lookup_paths=many, metadata_paths=metadata,
                session_paths=variants, session_choices=[j % 3 for j in range(len(times))] if variants else [],
                independent_paths=[path(s) for s in independent_ids], independent_source_ids=independent_ids,
                prior_paths=[path(s) for s in prior_ids], prior_source_ids=prior_ids,
                prior_point=prior_point, prior_start=prior_start,
                prior_end=t1, times=times, start=lo, end=hi, point=point, late_point=t1 * 95 // 100,
                step_ticks=t1 // model.config(profile)['steps'])
