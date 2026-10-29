import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from . import bindings, build_catalog, model, oracle


def case(identifier):
    return copy.deepcopy(next(c for c in bindings.CATALOG['cases'] if c['id'] == identifier))


class ModelTests(unittest.TestCase):
    def test_original_catalog_is_unchanged(self):
        path = Path(bindings.ROOT, 'cases.json')
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), '2db56ad50e3d60fd82be16da566e1758078482011c69d0a07165099a3b6501be')
        self.assertEqual(build_catalog.cases, bindings.CATALOG['cases'])
        self.assertEqual(build_catalog.quick_ids, bindings.CATALOG['quick_ids'])
        self.assertEqual(len(build_catalog.cases), 354)
        self.assertEqual(len(bindings.IMPLEMENTED), 354)

    def test_exact_native_paths_and_no_wdb_collisions(self):
        total = 0
        for profile in ('S', 'M', 'L', 'MH', 'MT'):
            paths = [tuple(bindings.native_path(d['path'], 'wdb-lib')) for d in model.inventory(profile)]
            self.assertEqual(len(paths), len(set(paths)))
            total += len(paths)
        self.assertEqual(total, 241235)
        self.assertEqual(bindings.native_path(['tb_opb', 'regular64', 'word[0]', 'data', 'q'], 'ghw-native'), ['tb_opb', 'regular64', 'word(0)', 'data', 'q'])
        self.assertEqual(bindings.native_path(['tb_opb', 'extra', 'tree', 'branch', 'left', 'q'], 'wdb-lib'), ['tb_opb', 'extra.tree', 'branch.left', 'q'])

    def test_initial_strictly_before_inclusive_change(self):
        c = case('OPB-SCAN-006')
        c['selection'], c['parameters']['k'] = 'drivers', 1
        initials, changes = oracle.range_rows(c, 16000, 16000)
        self.assertEqual(initials, [dict(kind='initial', slot=0, bits='0', changed_at=None)])
        self.assertEqual(changes, [dict(kind='change', slot=0, time=16000, bits='1')])
        initials, changes = oracle.range_rows(c, 16001, 32000)
        self.assertEqual(initials[0]['changed_at'], 16000)
        self.assertEqual(changes[0]['time'], 32000)
        initials, changes = oracle.range_rows(c, 0, 0)
        self.assertEqual(initials, [])
        self.assertEqual(changes, [dict(kind='change', slot=0, time=0, bits='0')])

    def test_exact_four_states_and_wide_bit_order(self):
        self.assertEqual(model.value_at('S', 'mixed_xz64/0', 0), 'zx10' * 16)
        self.assertEqual(model.value_at('S', 'mixed_xz64/0', 10000), '0zx1' * 16)
        self.assertEqual(model.value_at('S', 'all_x64/0', 0), 'x' * 64)
        wide = model.value_at('S', 'wide1024/0', 0)
        self.assertEqual(len(wide), 1024)
        self.assertNotEqual(wide[:64], wide[-64:])

    def test_slice_last_change_is_projection_change(self):
        stable = case('OPB-COMP-005')
        active = case('OPB-COMP-001')
        nested = case('OPB-COMP-013')
        concat = case('OPB-COMP-009')
        self.assertIsNone(oracle.point(stable, 50000000)[0][1])
        self.assertEqual(oracle.point(active, 50000000)[0][1], 50000000)
        self.assertEqual(oracle.point(nested, 50000000)[0], (model.value_at('M', 'regular64/15', 50000000)[-1], 40016000))
        self.assertEqual(len(oracle.point(concat, 50000000)[0][0]), 512)

    def test_prefix_counts_changes_only_and_candidates_allow_extras(self):
        c = case('OPB-PREFIX-028')
        i = bindings.inputs(c, 'fst-lib', '/unused')
        initials, changes = oracle.range_rows(c, i['start'], i['end'])
        self.assertEqual(len(initials), 100)
        self.assertEqual(len(changes), 100)
        self.assertEqual(len({r['time'] for r in changes}), 100)
        c = case('OPB-CAND-005')
        i = bindings.inputs(c, 'fst-lib', '/unused')
        _, changes = oracle.range_rows(c, i['start'], i['end'])
        ticks = sorted({r['time'] for r in changes} | {i['start'] + 1})
        rows = [dict(kind='candidate', time=t) for t in ticks] + [dict(kind='done')]
        oracle.check(c, i, rows)
        with self.assertRaisesRegex(ValueError, 'strictly increasing'):
            oracle.check(c, i, rows[:1] + rows)
        with self.assertRaisesRegex(ValueError, 'missing candidate'):
            oracle.check(c, i, [dict(kind='done')])

    def test_repeated_slots_are_not_deduplicated(self):
        c = case('OPB-DUP-001')
        self.assertEqual(len(bindings.selected_sources(c)), 100)
        rows = oracle.sample_rows(c, 50000000)
        self.assertEqual([r['slot'] for r in rows], list(range(100)))
        i = bindings.inputs(c, 'fst-lib', '/unused')
        with self.assertRaises(ValueError):
            oracle.check(c, i, rows[:1] + [dict(kind='done')])

    def test_selection_identity_is_required_even_with_matching_counts(self):
        c = case('OPB-SELECT-002')
        i = bindings.inputs(c, 'fst-lib', '/unused')
        row = dict(kind='selection', slots=100, bases=100, widths=[64] * 100, same_order=True)
        oracle.check(c, i, [row, dict(kind='done')])
        row['same_order'] = False
        with self.assertRaisesRegex(ValueError, 'observation'):
            oracle.check(c, i, [row, dict(kind='done')])

    def test_grid_and_query_follow_absolute_schedule(self):
        c = case('OPB-SESSION-003')
        i = bindings.inputs(c, 'fst-lib', '/unused')
        self.assertEqual(len(set(i['times'])), 100)
        self.assertEqual((min(i['times']), max(i['times'])), (0, 100000000))
        self.assertNotEqual(i['times'], sorted(i['times']))
        c = case('OPB-QUERY-005')
        i = bindings.inputs(c, 'fst-lib', '/unused')
        rows = list(oracle.expected(c, i))
        self.assertEqual(len(bindings.selected_sources(c)), 102)
        self.assertGreater(len(rows), 0)
        for tick in {r['time'] for r in rows if r['kind'] == 'sample'}:
            self.assertEqual(model.value_at('M', 'drivers/0', tick - 1), '0')
            self.assertEqual(model.value_at('M', 'drivers/0', tick), '1')
            self.assertEqual(((tick // 1000 - 16) // 32 + 1) % 16, 0)

    def test_independently_reconstructed_scope_count(self):
        c = case('OPB-HIER-022')
        scopes = {tuple(d['path'][:n]) for d in model.inventory('M') for n in range(1, len(d['path']))}
        self.assertEqual(len(scopes), 54344)
        self.assertEqual(oracle.digest(scopes), bindings.fixture_binding('M', 'vcd-native')['scope_sha256'])
        self.assertEqual(bindings.fixture_binding('M', 'wdb-lib')['physical_scopes'], 142505)

    def test_unknown_timestamp_is_allowed_but_wrong_known_time_fails(self):
        c = case('OPB-POINT-028')
        i = bindings.inputs(c, 'fst-lib', '/unused')
        rows = list(oracle.expected(c, i))
        rows[0]['changed_at'] = None
        result = oracle.check(c, i, iter(rows + [dict(kind='done')]))
        self.assertGreater(result['changed_at_unknown'], 0)
        rows[0]['changed_at'] = 12345
        with self.assertRaisesRegex(ValueError, 'observation 0'):
            oracle.check(c, i, iter(rows + [dict(kind='done')]))

    def test_full_recorded_hierarchy_paths_and_widths(self):
        c = case('OPB-HIER-022')
        i = bindings.inputs(c, 'vcd-native', '/unused')
        q = bindings.fixture_binding('M', 'vcd-native')
        def rows():
            for d in model.inventory('M'):
                yield dict(kind='variable', path=d['path'], width=d['width'])
            yield dict(kind='hierarchy', variables=q['recorded'], scopes=q['physical_scopes'], scope_sha256=q['scope_sha256'])
            yield dict(kind='done')
        self.assertEqual(oracle.check(c, i, rows())['recorded_paths_checked'], 29216)
        wrong = rows()
        row = next(wrong)
        row['width'] = 63
        import itertools
        with self.assertRaisesRegex(ValueError, 'wrong width'):
            oracle.check(c, i, itertools.chain([row], wrong))
