import copy
import json
from pathlib import Path
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

    def save(self, name, mean=100):
        root = self.root / name
        raw = root / 'raw/0000/fst-lib/OPB-POINT-028/new'
        raw.mkdir(parents=True)
        case = dict(id='OPB-POINT-028', fixture='S', boundary='P', workload='point', parameters={'k': 1})
        job = dict(case_id=case['id'], configuration=0, raw='raw/0000',
                   input=dict(case=case, backend='fst-lib', artifact='/absent/waveform.fst', source_ids=['data'], times=[1]),
                   artifact_identity=dict(file='waveform.fst', size=1, sha256='payload'),
                   measurement=dict(status='measured', wall_seconds=1, log='measure.log', raw_files=[
                       str((raw / n).relative_to(root)) for n in ('sample.json', 'estimates.json', 'benchmark.json')]),
                   verification=dict(status='passed', records=1))
        data = dict(schema=1, workload_version='opb-core-2', source_revision=name,
                    fixtures_revision='fixtures', finished_at='2026-01-01T00:00:00+00:00',
                    source_files={'benches/profile.rs': 'workload', 'src/lib.rs': name},
                    criterion=dict(sample_size=2), configurations=[dict(backend='fst-lib', sdk_home='/absent/sdk')], jobs=[job])
        (root / 'measure.log').write_text('saved diagnostic\n')
        (root / 'run.json').write_text(json.dumps(data))
        (raw / 'sample.json').write_text(json.dumps(dict(times=[mean * 2, mean * 4], iters=[2, 4])))
        (raw / 'benchmark.json').write_text(json.dumps(dict(full_id='fst-lib/OPB-POINT-028')))
        (raw / 'estimates.json').write_text(json.dumps(dict(
            mean=dict(point_estimate=mean, confidence_interval=dict(lower_bound=mean * .9, upper_bound=mean * 1.1, confidence_level=.95)),
            std_dev=dict(point_estimate=mean * .1))))
        return root, data, raw

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
                  f"sys.argv=['tools.opb','view',{str(root)!r},'--output',{str(self.output)!r}]; "
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


if __name__ == '__main__':
    unittest.main()
