import copy
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from . import viewer


class ViewerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / 'report.html'

    def save(self, name, mean=100, backend='fst-lib', case_id='OPB-POINT-028'):
        root = self.root / name
        raw = root / 'raw/0000' / backend / case_id / 'new'
        raw.mkdir(parents=True)
        case = dict(id=case_id, fixture='S', boundary='P', workload='point', parameters={'k': 1})
        artifact = 'waveform.' + backend.split('-')[0]
        job = dict(case_id=case['id'], configuration=0, raw='raw/0000',
                   input=dict(case=case, backend=backend, artifact='/absent/' + artifact, source_ids=['data'], times=[1]),
                   artifact_identity=dict(file=artifact, size=1, sha256='payload'),
                   measurement=dict(status='measured', wall_seconds=1, log='measure.log', raw_files=[
                       str((raw / n).relative_to(root)) for n in ('sample.json', 'estimates.json', 'benchmark.json')]),
                   verification=dict(status='passed', records=1))
        data = dict(schema=1, workload_version='opb-core-2', source_revision=name,
                    fixtures_revision='fixtures', finished_at='2026-01-01T00:00:00+00:00',
                    source_files={'benches/profile.rs': 'workload', 'src/lib.rs': name},
                    criterion=dict(sample_size=2), configurations=[dict(backend=backend, sdk_home='/absent/sdk')], jobs=[job])
        (root / 'measure.log').write_text('saved diagnostic\n')
        (root / 'run.json').write_text(json.dumps(data))
        (raw / 'sample.json').write_text(json.dumps(dict(times=[mean * 2, mean * 4], iters=[2, 4])))
        (raw / 'benchmark.json').write_text(json.dumps(dict(full_id=backend + '/' + case_id)))
        (raw / 'estimates.json').write_text(json.dumps(dict(
            mean=dict(point_estimate=mean, confidence_interval=dict(lower_bound=mean * .9, upper_bound=mean * 1.1, confidence_level=.95)),
            std_dev=dict(point_estimate=mean * .1))))
        return root, data, raw

    def matrix(self, name, means):
        root = self.root / name
        root.mkdir()
        data = None
        for position, (backend, values) in enumerate(means.items()):
            for case_id, mean in values.items():
                _, part, raw = self.save(f'{name}-part-{position}-{case_id}', mean, backend, case_id)
                if data is None:
                    data = {**part, 'configurations': [], 'jobs': []}
                if len(data['configurations']) == position:
                    data['configurations'].append(part['configurations'][0])
                job = part['jobs'][0]
                prefix = f"raw/{len(data['jobs']):04d}"
                shutil.copytree(raw, root / prefix / backend / case_id / 'new')
                job['configuration'] = position
                job['raw'] = prefix
                job['measurement']['raw_files'] = [name.replace('raw/0000', prefix) for name in job['measurement']['raw_files']]
                data['jobs'].append(job)
        (root / 'measure.log').write_text('saved diagnostics\n')
        self.update(root, data)
        return root, data

    def read(self, root):
        return viewer.read_run(root, self.output)

    def cell(self, run):
        return run['rows'][0]['cells']['OPB-POINT-028']

    def update(self, root, data):
        (root / 'run.json').write_text(json.dumps(data))

    def test_portable_read_ignores_recorded_locations_and_has_correct_units(self):
        root, data, _ = self.save('baseline')
        data['directory'] = '/missing/original/location'
        self.update(root, data)
        result = self.read(root)
        cell = self.cell(result)
        self.assertEqual(cell['status'], 'passed')
        self.assertEqual(cell['timing']['samples'], [100, 100])
        self.assertEqual(cell['timing']['mean'], 100)
        self.assertEqual(cell['timing']['links']['sample'], 'baseline/raw/0000/fst-lib/OPB-POINT-028/new/sample.json')

    def test_failed_partial_raw_is_never_a_timing_or_ratio(self):
        root, data, _ = self.save('timeout')
        data['jobs'][0]['measurement'].update(status='failed', reason='timeout', wall_seconds=600)
        data['jobs'][0]['verification']['status'] = 'not-run'
        self.update(root, data)
        result = self.read(root)
        viewer.comparisons([result])
        cell = self.cell(result)
        self.assertEqual(cell['status'], 'timeout')
        self.assertIsNone(cell['timing'])
        self.assertNotIn('ratio', cell['comparisons'][0])

    def test_missing_corrupt_and_foreign_raw_remain_visible(self):
        for name in ('missing', 'negative', 'wrong-id'):
            with self.subTest(name=name):
                root, _, raw = self.save(name)
                if name == 'missing':
                    (raw / 'sample.json').unlink()
                elif name == 'negative':
                    (raw / 'sample.json').write_text(json.dumps(dict(times=[-1, 1], iters=[1, 1])))
                else:
                    (raw / 'benchmark.json').write_text(json.dumps(dict(full_id='fst-lib/ANOTHER-CASE')))
                cell = self.cell(self.read(root))
                self.assertEqual(cell['status'], 'invalid raw')
                self.assertIsNone(cell['timing'])
                self.assertTrue(cell['data_error'])

    def test_unchecked_unfinished_and_run_error_do_not_qualify(self):
        for name in ('unchecked', 'unfinished', 'run-error'):
            root, data, _ = self.save(name)
            if name == 'unchecked':
                data['jobs'][0]['verification']['status'] = 'unchecked'
            elif name == 'unfinished':
                del data['finished_at']
            else:
                data['error'] = 'source identity changed'
            self.update(root, data)
            result = self.read(root)
            viewer.comparisons([result])
            self.assertNotEqual(self.cell(result)['status'], 'passed')
            self.assertNotIn('ratio', self.cell(result)['comparisons'][0])

    def test_comparison_allows_changed_library_revision_and_reports_sdk_conditions(self):
        a, _, _ = self.save('baseline', 100)
        b, data, _ = self.save('candidate', 200)
        data['configurations'][0]['sdk_home'] = '/different/sdk'
        data['fixtures_revision'] = 'same-payload-other-pin'
        data['jobs'][0]['input']['artifact'] = '/moved/waveform.fst'
        self.update(b, data)
        runs = [self.read(a), self.read(b)]
        viewer.comparisons(runs)
        comparison = self.cell(runs[1])['comparisons'][0]
        self.assertEqual(comparison['ratio'], 2)
        self.assertFalse(comparison['reasons'])
        self.assertEqual(len(comparison['conditions']), 2)
        self.assertFalse(comparison['ci_overlap'])

    def test_comparison_blocks_changed_workload_inputs_payload_and_settings(self):
        baseline, _, _ = self.save('baseline')
        for name in ('boundary', 'inputs', 'payload', 'settings', 'workload-source', 'version'):
            with self.subTest(name=name):
                root, data, _ = self.save(name)
                if name == 'boundary':
                    data['jobs'][0]['input']['case']['boundary'] = 'R'
                elif name == 'inputs':
                    data['jobs'][0]['input']['times'] = [2]
                elif name == 'payload':
                    data['jobs'][0]['artifact_identity']['sha256'] = 'different'
                elif name == 'settings':
                    data['criterion']['noise_threshold'] = .2
                elif name == 'workload-source':
                    data['source_files']['benches/profile.rs'] = 'different'
                else:
                    data['workload_version'] = 'opb-quick-1'
                self.update(root, data)
                runs = [self.read(baseline), self.read(root)]
                viewer.comparisons(runs)
                result = self.cell(runs[1])['comparisons'][0]
                self.assertNotIn('ratio', result)
                self.assertTrue(result['reasons'])

    def test_paths_cannot_escape_run_and_duplicate_jobs_are_rejected(self):
        root, data, raw = self.save('paths')
        for relative in ('../outside', '/absolute', 'raw\\outside'):
            with self.assertRaises(ValueError):
                viewer.saved_file(root, relative)
        outside = self.root / 'outside.json'
        outside.write_text('{}')
        (raw / 'sample.json').unlink()
        (raw / 'sample.json').symlink_to(outside)
        self.assertEqual(self.cell(self.read(root))['status'], 'invalid raw')
        data['jobs'].append(copy.deepcopy(data['jobs'][0]))
        self.update(root, data)
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.read(root)

    def test_html_escapes_saved_data_and_cli_does_not_import_runner_or_bindings(self):
        root, data, _ = self.save('html')
        text = '</script><img src=x onerror=alert(1)> /*OPB_SCRIPT*/'
        data['jobs'][0]['input']['case']['parameters']['note'] = text
        self.update(root, data)
        viewer.render([root], self.output)
        html = self.output.read_text()
        self.assertNotIn(text, html)
        self.assertIn('\\u003c/script\\u003e', html)
        embedded = json.loads(html.split('<script id="opb-data" type="application/json">')[1].split('</script>')[0])
        self.assertTrue(any(c['parameters'].get('note') == text for c in embedded['definitions'].values()))
        script = ("import sys,runpy; sys.modules['tools.opb.runner']=None; "
                  "sys.modules['tools.opb.bindings']=None; "
                  f"sys.argv=['tools.opb','view',{str(root)!r},'--ideal-of',{str(root)!r},'--output',{str(self.output)!r}]; "
                  "runpy.run_module('tools.opb',run_name='__main__')")
        result = subprocess.run([sys.executable, '-B', '-c', script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('no measurements executed', result.stdout)

    def test_output_cannot_overwrite_saved_evidence_or_its_aliases(self):
        root, data, raw = self.save('protected')
        (root / 'context.json').write_text('{}')
        (root / 'build.log').write_text('build diagnostic')
        data['configurations'][0]['build_log'] = 'build.log'
        self.update(root, data)
        evidence = [root / 'run.json', root / 'context.json', root / 'build.log', root / 'measure.log',
                    *(raw / name for name in ('sample.json', 'estimates.json', 'benchmark.json'))]
        for source in evidence:
            for kind in ('direct', 'symlink', 'hardlink'):
                with self.subTest(source=source.name, kind=kind):
                    output = source if kind == 'direct' else self.root / f'alias-{kind}.html'
                    if kind == 'symlink':
                        output.symlink_to(source)
                    elif kind == 'hardlink':
                        output.hardlink_to(source)
                    before = source.read_bytes()
                    with self.assertRaisesRegex(ValueError, 'overwrite saved'):
                        viewer.render([root], output)
                    self.assertEqual(source.read_bytes(), before)
                    if output != source:
                        output.unlink()

    def test_report_can_be_regenerated_beside_or_inside_saved_run(self):
        root, _, _ = self.save('regeneration')
        for output in (self.output, root / 'report.html'):
            viewer.render([root], output)
            before = output.read_bytes()
            viewer.render([root], output)
            self.assertEqual(output.read_bytes(), before)

    def test_ideal_selects_each_case_and_keeps_winner_evidence_without_mutating_source(self):
        root, _ = self.matrix('ideal', {'fst-lib': {'OPB-POINT-028': 80, 'OPB-POINT-029': 40},
                                      'vcd-native': {'OPB-POINT-028': 20, 'OPB-POINT-029': 60}})
        source = self.read(root)
        before = copy.deepcopy(source)
        derived = viewer.ideal_run(source, 0)
        self.assertEqual(source, before)
        self.assertEqual(derived['rows'][:-1], source['rows'])
        ideal = derived['rows'][-1]
        for case_id, mean, backend, job in [('OPB-POINT-028', 20, 'vcd-native', 2),
                                           ('OPB-POINT-029', 40, 'fst-lib', 1)]:
            cell = ideal['cells'][case_id]
            winner = next(row for row in source['rows'] if row['backend'] == backend)['cells'][case_id]
            self.assertEqual(cell['status'], 'synthetic')
            self.assertEqual(cell['timing'], winner['timing'])
            self.assertEqual(cell['timing']['mean'], mean)
            self.assertEqual(cell['synthetic_source']['backend'], backend)
            self.assertEqual(cell['synthetic_source']['job'], job)
            self.assertEqual(cell['log'], winner['log'])
            self.assertEqual(cell['verification']['status'], 'source passed')
        viewer.comparisons([source, derived])
        self.assertEqual(ideal['cells']['OPB-POINT-028']['comparisons'][1]['ratio'], 1)
        self.assertNotIn('ratio', ideal['cells']['OPB-POINT-028']['comparisons'][0])

    def test_ideal_excludes_unqualified_timings_and_does_not_invent_missing_cases(self):
        for status in ('failed', 'unchecked', 'invalid raw'):
            root, data = self.matrix('exclude-' + status, {'fst-lib': {'OPB-POINT-028': 80},
                                                          'vcd-native': {'OPB-POINT-028': 20}})
            job = data['jobs'][1]
            if status == 'failed':
                job['measurement'].update(status='failed', reason='timeout')
                job['verification']['status'] = 'not-run'
            elif status == 'unchecked':
                job['verification']['status'] = 'unchecked'
            else:
                (root / job['measurement']['raw_files'][0]).unlink()
            self.update(root, data)
            ideal = viewer.ideal_run(self.read(root), 0)['rows'][-1]
            self.assertEqual(ideal['cells']['OPB-POINT-028']['timing']['mean'], 80)
        root, data = self.matrix('missing', {'fst-lib': {'OPB-POINT-028': 80, 'OPB-POINT-029': 40}})
        data['jobs'][0]['verification']['status'] = 'unchecked'
        self.update(root, data)
        self.assertEqual(set(viewer.ideal_run(self.read(root), 0)['rows'][-1]['cells']), {'OPB-POINT-029'})
        data['jobs'][1]['verification']['status'] = 'unchecked'
        self.update(root, data)
        with self.assertRaisesRegex(ValueError, 'verified timing'):
            viewer.ideal_run(self.read(root), 0)

    def test_ideal_ties_follow_source_order_and_different_case_definitions_are_rejected(self):
        root, data = self.matrix('ties', {'fst-lib': {'OPB-POINT-028': 20},
                                        'vcd-native': {'OPB-POINT-028': 20}})
        cell = viewer.ideal_run(self.read(root), 0)['rows'][-1]['cells']['OPB-POINT-028']
        self.assertEqual(cell['synthetic_source']['backend'], 'fst-lib')
        data['jobs'][1]['input']['case']['parameters']['k'] = 2
        self.update(root, data)
        with self.assertRaisesRegex(ValueError, 'different case definitions'):
            viewer.ideal_run(self.read(root), 0)

    def test_ideal_render_reuses_source_rows_and_requires_a_supplied_input(self):
        root, _ = self.matrix('render-ideal', {'fst-lib': {'OPB-POINT-028': 80},
                                              'vcd-native': {'OPB-POINT-028': 20}})
        before = (root / 'run.json').read_bytes()
        viewer.render([root], self.output, root / 'run.json')
        data = json.loads(self.output.read_text().split('<script id="opb-data" type="application/json">')[1].split('</script>')[0])
        source, derived = data['runs']
        self.assertEqual(derived['rows_from'], 0)
        self.assertEqual(len(derived['rows']), 1)
        self.assertEqual(len(source['rows'] + derived['rows']), 3)
        self.assertEqual(derived['rows'][0]['cells']['OPB-POINT-028']['timing']['mean'], 20)
        self.assertEqual((root / 'run.json').read_bytes(), before)
        with self.assertRaisesRegex(ValueError, 'exactly one input'):
            viewer.render([root], self.output, self.root / 'not-an-input')


if __name__ == '__main__':
    unittest.main()
