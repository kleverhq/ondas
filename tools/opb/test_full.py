"""Semantic regressions for the full catalog, without waveform payloads."""
import copy
import itertools
import unittest

from . import bindings, model, oracle, runner
from .test_model import case


def find(workload, **params):
    return copy.deepcopy(next(c for c in bindings.CATALOG['cases']
        if c['workload'] == workload and all(c['parameters'].get(k) == v for k,v in params.items())))


class FullTests(unittest.TestCase):
    def test_positions_windows_and_order_are_concrete(self):
        for pos,percent in (('t05',5),('t50',50),('t95',95)):
            c=find('point-owned',position=pos,k=1)
            self.assertEqual(bindings.inputs(c,'fst-lib','unused')['point'],1000000*percent)
        for name,bounds in (('early10',(0,9999999)),('middle10',(45000000,54999999)),('late10',(90000000,100000000)),('all',(0,100000000))):
            self.assertEqual(bindings.window(100000000,name),bounds)
        grids={order:bindings.inputs(find('point-session',order=order),'fst-lib','unused')['times']
               for order in ('ascending','descending','permuted','random-early','random-middle','random-late','ping-pong','random-global-switch')}
        self.assertEqual(grids['ascending'],sorted(grids['permuted']))
        self.assertEqual(grids['descending'],list(reversed(grids['ascending'])))
        self.assertEqual(len({tuple(t) for t in grids.values()}),8)
        self.assertTrue(all(t < 10000000 for t in grids['random-early']))
        self.assertTrue(all(45000000 <= t < 55000000 for t in grids['random-middle']))
        self.assertTrue(all(t >= 90000000 for t in grids['random-late']))
        c=find('explicit-times-order',order='duplicates')
        times=bindings.inputs(c,'fst-lib','unused')['times']
        self.assertEqual(len(times),100)
        self.assertEqual(len(set(times)),50)
        self.assertEqual((min(times),max(times)),(45000000,54999999))

    def test_sessions_switch_actual_sets_and_query_drivers(self):
        c=find('point-session',order='random-global-switch',k=100)
        i=bindings.inputs(c,'fst-lib','unused')
        self.assertEqual(len(i['session_choices']),100)
        self.assertEqual(len(i['session_paths']),3)
        self.assertNotEqual(i['session_paths'][0],i['session_paths'][1])
        rows=iter(oracle.expected(c,i))
        first=list(itertools.islice(rows,100))
        second=list(itertools.islice(rows,100))
        self.assertEqual(first[0]['bits'],model.value_at(c['fixture'],'regular64/0',i['times'][0]))
        self.assertEqual(second[0]['bits'],model.value_at(c['fixture'],'regular64/1',i['times'][1]))
        self.assertEqual(len(bindings.selected_sources(find('many-drivers'))),109)
        self.assertEqual(len(bindings.selected_sources(find('accept-every16-edge'))),102)

    def test_projected_changes_are_normalized(self):
        stable=find('computed-scan')
        stable['selection']='stable-high-slice'
        self.assertEqual(list(oracle.range_iter(stable,45000000,54999999)),[
            dict(kind='initial',slot=0,bits=oracle.projection(stable,44999999)[0],changed_at=None)])
        overlap=find('overlapping-slices-point')
        bits=model.value_at('M','payload/0',50000000)
        self.assertEqual(oracle.projection(overlap,50000000),[bits[-i-64:len(bits)-i] for i in range(100)])
        active=find('computed-point')
        active['selection']='active-bit'
        self.assertEqual(oracle.point(active,50000000)[0][1],50000000)

    def test_candidate_stable_slice_may_have_no_candidates(self):
        c=find('candidates-stable-slice')
        i=bindings.inputs(c,'fst-lib','unused')
        oracle.check(c,i,[dict(kind='done')])
        oracle.check(c,i,[dict(kind='candidate',time=i['start']),dict(kind='done')])
        with self.assertRaisesRegex(ValueError,'strictly increasing'):
            oracle.check(c,i,[dict(kind='candidate',time=i['start'])]*2+[dict(kind='done')])

    def test_scan_only_reorders_equal_tick_slots(self):
        rows=[dict(kind='change',slot=1,time=10,bits='0'),dict(kind='change',slot=0,time=10,bits='1')]
        self.assertEqual(list(oracle.canonical_scan(rows)),list(reversed(rows)))
        with self.assertRaisesRegex(ValueError,'unordered'):
            list(oracle.canonical_scan(rows+[dict(kind='change',slot=0,time=9,bits='0')]))
        c=find('materialize-traces',k=1)
        i=bindings.inputs(c,'fst-lib','unused')
        rows=list(oracle.expected(c,i))
        changes=[n for n,r in enumerate(rows) if r['kind']=='change']
        if len(changes)>1:
            a,b=changes[:2]
            rows[a],rows[b]=rows[b],rows[a]
            with self.assertRaises(ValueError):
                oracle.check(c,i,iter(rows+[dict(kind='done')]))

    def test_native_metadata_scopes_are_real_prefixes(self):
        c=find('metadata-scopes',k=100)
        i=bindings.inputs(c,'wdb-lib','unused')
        paths=[bindings.native_path(d['path'],'wdb-lib') for d in model.inventory(c['fixture'])]
        prefixes={tuple(p[:n]) for p in paths for n in range(1,len(p))}
        self.assertEqual(len(i['metadata_paths']),100)
        self.assertTrue(all(tuple(p) in prefixes for p in i['metadata_paths']))

    def test_point_changed_at_does_not_materialize_payload_history(self):
        c=find('driver-payload-base-sharing',shape='shared-base')
        value,changed=oracle.point(c,50000000)[0]
        self.assertEqual(value,model.value_at('M','drivers/0',50000000))
        self.assertEqual(changed,(50000//16)*16*1000)
        self.assertEqual(oracle.projection(c,50000000)[2],model.value_at('M','payload/0',50000000))

    def test_full_and_smoke_selectors(self):
        import argparse
        selected=runner.selected_cases(argparse.Namespace(command='run',case=None,quick=False,full=True))
        self.assertEqual(len(selected),354)
        self.assertEqual({c['id'] for c in selected},bindings.IMPLEMENTED)
        selected=runner.selected_cases(argparse.Namespace(command='run',case=None,quick=False,full=False))
        self.assertEqual([c['id'] for c in selected],['OPB-POINT-028'])

    def test_query_absolute_edges_and_repeated_reads(self):
        for work in ('reject-all','accept-all-edges','accept-every16-edge','first-accepted-after-rejects','many-drivers','current-previous-repeat','eager-every16-edge','driver-payload-base-sharing'):
            c=find(work)
            i=bindings.inputs(c,'fst-lib','unused')
            i['start'],i['end']=1000,512000
            accepted=set(oracle.query_ticks(c,i))
            drivers=c['parameters']['drivers']
            times=set()
            for dep in oracle.dependencies(c)[:drivers]:
                for tick,_ in oracle.slot_history(c['fixture'],dep):
                    if tick>i['end']:break
                    if tick>=i['start']:times.add(tick)
            times.add(1101)  # Legal extra candidate must not become an edge.
            rows=[oracle.sharing_selection(c)] if work=='driver-payload-base-sharing' else []
            for tick in sorted(times):
                rows.append(dict(kind='candidate',time=tick))
                if work=='current-previous-repeat':
                    for t,slots in ((tick-1,range(drivers)),(tick,range(drivers+1)),(tick-1,range(drivers))):
                        values=oracle.sample_rows(c,t)
                        rows.extend(values[s] for s in slots)
                if tick in accepted:
                    rows.append(dict(kind='accepted',time=tick))
                    rows.extend(oracle.sample_rows(c,tick)[drivers+1:])
                    if work=='first-accepted-after-rejects':break
            rows.extend([dict(kind='query-end',stopped=work=='first-accepted-after-rejects'),dict(kind='done')])
            with self.subTest(work=work):
                oracle.check(c,i,iter(rows))
                bad=copy.deepcopy(rows)
                bad.insert(1,dict(kind='accepted',time=1101))
                with self.assertRaises(ValueError):oracle.check(c,i,iter(bad))
                samples=[r for r in bad if r['kind']=='sample']
                if samples:
                    rows=copy.deepcopy(rows)
                    row=next(r for r in rows if r['kind']=='sample')
                    row['bits']='x'+row['bits'][1:]
                    with self.assertRaises(ValueError):oracle.check(c,i,iter(rows))

    def test_wdb_hierarchy_rejects_repeated_unknown_declarations(self):
        c=case('OPB-HIER-020')
        i=bindings.inputs(c,'wdb-lib','unused')
        q=bindings.fixture_binding('S','wdb-lib')
        def rows():
            for d in model.inventory('S'):
                yield dict(kind='variable',path=bindings.native_path(d['path'],'wdb-lib'),width=d['width'])
            for _ in range(q['unrecorded']):
                yield dict(kind='variable',path=['tb_opb','invented'],width=None)
            yield dict(kind='hierarchy',scopes=q['physical_scopes'],variables=q['recorded']+q['unrecorded'],scope_sha256=q['scope_sha256'])
            yield dict(kind='done')
        with self.assertRaisesRegex(ValueError,'declaration identity'):
            oracle.check(c,i,rows())

    def test_query_equal_answers_do_not_hide_wrong_base_sharing(self):
        for shape,bases in (('shared-base',2),('distinct-base',3)):
            c=find('driver-payload-base-sharing',shape=shape)
            i=bindings.inputs(c,'fst-lib','unused')
            wrong=dict(kind='selection',slots=3,bases=5-bases,widths=[1,64,1024],same_order=True)
            with self.assertRaisesRegex(ValueError,'observation 0'):
                oracle.check(c,i,[wrong,dict(kind='done')])
