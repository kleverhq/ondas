"""Regenerate the original finite catalog. Implemented IDs are selected by the runner."""
import json
from collections import Counter
from itertools import product
from pathlib import Path

ROOT = Path(__file__).resolve().parent
cases = []
counts = Counter()

def add(group, workload, fixture="M", boundary="P", *, api, selection="regular64", history="none", **params):
    counts[group] += 1
    case_id = f"OPB-{group}-{counts[group]:03d}"
    tags = ["tier:core", f"op:{group.lower()}", f"fixture:{fixture.lower()}",
            f"boundary:{boundary.lower()}", "input:path", "value:bits",
            f"selection:{selection}", f"history:{history}"]
    for key in ("k", "q", "n", "position", "window", "shape", "order", "ownership"):
        if key in params:
            tags.append(f"{key}:{params[key]}")
    cases.append(dict(id=case_id, workload=workload, fixture=fixture, boundary=boundary,
                      api=api, selection=selection, history=history, parameters=params, tags=tags))

for f in ("S", "M", "L"):
    add("OPEN", "open-drop", f, "R", api="open_with", selection="none")

for pos, f, mode in product(("shallow", "deep"), ("S", "M", "L"), ("R", "P")):
    add("HIER", "local-items", f, mode, api="Scope::children; consume both Item kinds", selection="none", position=pos)
for kind, k in product(("scopes", "variables"), (1, 100, 10000)):
    add("HIER", f"metadata-{kind}", api="Scope / Variable getters including path", selection="none", k=k)
for f, mode in product(("S", "M", "L"), ("R", "P")):
    add("HIER", "walk-dfs", f, mode, api="Hierarchy::roots + Scope::children", selection="none")

for kind, pos, f, mode in product(("scope", "signal"), ("shallow", "deep"), ("S", "M", "L"), ("R", "P")):
    add("LOOK", f"lookup-{kind}", f, mode, api=f"Hierarchy::{kind}_path", selection="none", position=pos, path_form="parsed")
for kind, f in product(("scope", "signal"), ("S", "M", "L")):
    add("LOOK", f"missing-last-{kind}", f, api=f"Hierarchy::{kind}_path", selection="none", position="deep", path_form="parsed")
for kind, k in product(("scope", "signal"), (100, 10000)):
    add("LOOK", f"lookup-many-{kind}", api=f"Hierarchy::{kind}_path", selection="none", k=k, path_form="parsed")

def point(group, f, mode, k, pos, **extra):
    add(group, "point-owned", f, mode, api="Waveform::sample" if k == 1 else "Waveform::samples", k=k, position=pos, ownership="owned", **extra)

for k, pos, f, mode in product((1, 100), ("t05", "t50", "t95"), ("S", "M", "L"), ("R", "P")):
    point("POINT", f, mode, k, pos)
for pos, mode in product(("t05", "t50", "t95"), ("R", "P")):
    point("POINT", "M", mode, 10000, pos)

for window, f, mode in product(("early10", "middle10", "late10"), ("S", "M", "L"), ("R", "P")):
    add("BATCH", "explicit-times", f, mode, api="Waveform::samples_at", k=100, q=100, window=window, order="ascending", ownership="owned")
for (k, q), mode in product(((1, 100), (10000, 100), (100, 10000)), ("R", "P")):
    add("BATCH", "explicit-times", boundary=mode, api="Waveform::samples_at", k=k, q=q, window="middle10", order="ascending", ownership="owned")
for mode in ("R", "P"):
    add("BATCH", "explicit-times", boundary=mode, api="Waveform::samples_at", k=100, q=100, window="all", order="ascending", ownership="owned")

for k, pos, f, mode in product((1, 100), ("t05", "t50", "t95"), ("S", "M", "L"), ("R", "P")):
    add("PREFIX", "first-changes", f, mode, api="Selection::scan_each", selection="staggered64", k=k, n=100, position=pos, stop="N changes; initials excluded")
for (k, n), mode in product(((10000, 100), (100, 1), (100, 10000)), ("R", "P")):
    add("PREFIX", "first-changes", boundary=mode, api="Selection::scan_each", selection="staggered64", k=k, n=n, position="t50", stop="N changes; initials excluded")

for k, window, mode in product((1, 100), ("middle10", "all"), ("R", "P")):
    add("SCAN", "full-window", boundary=mode, api="Selection::scan_each", k=k, window=window)
for window, f in product(("middle10", "all"), ("S", "L")):
    add("SCAN", "full-window", f, api="Selection::scan_each", k=100, window=window)
for f in ("S", "M"):
    add("SCAN", "full-corpus", f, api="Hierarchy::signals + Selection::scan_each", selection="all-core", k="all", window="all")

orders = ("ascending", "descending", "permuted", "random-early", "random-middle", "random-late", "ping-pong", "random-global-switch")
for order in orders:
    add("SESSION", "point-session", api="Selection::samples; new selection only for switch", k=100, q=100, order=order)
for k in (1, 10000):
    add("SESSION", "point-session", api="Waveform::samples per request", k=k, q=100, order="random-global-switch")
for order, f in product(("random-global-switch", "ping-pong"), ("S", "M", "L")):
    add("SESSION", "point-session", f, "R", api="Selection::samples; new selection only for switch", k=100, q=100, order=order)
for order, f in product(("random-global-switch", "ping-pong"), ("S", "L")):
    add("SESSION", "point-session", f, api="Selection::samples; new selection only for switch", k=100, q=100, order=order)

for op, prior, f in product(("point", "prefix"), ("identical", "neighbor-same", "distant-disjoint", "background-10000"), ("M", "L")):
    mode = "S" if prior in ("identical", "neighbor-same") else "P"
    add("REUSE", f"after-prior-{op}", f, mode, api="Selection::samples" if op == "point" else "Selection::scan_each", selection="regular64" if op == "point" else "staggered64", history=prior, k=100, position="t50", **({"n":100} if op == "prefix" else {}))
add("REUSE", "same-wave-new-selection-point", api="Waveform::samples", history="identical-point-drop-selection", k=100, position="t50", ownership="owned")
for mode in ("P", "S"):
    add("REUSE", "same-wave-repeat-batch", boundary=mode, api="Waveform::samples_at" if mode=="P" else "Selection::samples_at", history="identical-batch-drop-selection" if mode=="P" else "identical-batch-retain-selection", k=100, q=100, window="middle10", order="ascending", ownership="owned")

for shape, op in product(("scalar", "wide1024", "constant64", "sparse64", "dense64"), ("point", "scan")):
    add("VALUE", f"value-shape-{op}", api="Waveform::samples" if op == "point" else "Selection::scan_each", selection=shape, k=100, **({"position":"t50"} if op == "point" else {"window":"middle10"}))

for scenario, f in product(("browse-deep", "inspect-window"), ("M", "L")):
    add("FLOW", scenario, f, "R", api="public API scenario", selection="none" if scenario == "browse-deep" else "staggered64", k=100)
for f in ("M", "L"):
    add("FLOW", "break-then-independent-point", f, api="Selection::scan_each Break; drop Selection; Waveform::samples", selection="staggered64", k=100, n=100, position="t50-then-t95")

for op, f, mode in product(("lookup", "point", "batch", "prefix"), ("MH", "MT"), ("R", "P")):
    api = {"lookup":"Hierarchy::signal_path", "point":"Waveform::samples", "batch":"Waveform::samples_at", "prefix":"Selection::scan_each"}[op]
    p = {"lookup":dict(position="shallow", path_form="parsed"), "point":dict(k=100,position="t50",ownership="owned"), "batch":dict(k=100,q=100,window="middle10",order="ascending",ownership="owned"), "prefix":dict(k=100,n=100,position="t50",stop="N changes; initials excluded")}[op]
    add("DIAG", f"constant-answer-{op}", f, mode, api=api, selection="none" if op=="lookup" else "staggered64" if op=="prefix" else "regular64", **p)

for k, shape in ((1,"unique"),(100,"unique"),(10000,"unique"),(100,"repeated"),(10000,"repeated")):
    add("SELECT", "select-drop", api="Waveform::select", k=k, shape=shape, bases=k if shape=="unique" else 1)

for pos in ("shallow", "deep"):
    add("CACHE", "children-ready", api="Scope::children", selection="none", position=pos, history="one-children-call")
add("CACHE", "walk-ready", api="Hierarchy::roots + Scope::children", selection="none", history="one-complete-walk")
for pos, prior in product(("shallow", "deep"), (1,2)):
    add("CACHE", "signal-lookup-after-prior", api="Hierarchy::signal_path", selection="none", position=pos, history=f"{prior}-variable-lookups", path_form="parsed")
for kind, pos in product(("scope", "signal"), ("shallow", "deep")):
    add("CACHE", f"text-lookup-{kind}", api=f"Hierarchy::{kind}", selection="none", position=pos, path_form="text")
for kind in ("scopes", "variables", "signals"):
    add("CACHE", f"flat-{kind}", api=f"Hierarchy::{kind}", selection="none")

for k, shape in ((1,"regular64"),(100,"regular64"),(10000,"regular64"),(1,"wide1024"),(100,"wide1024")):
    add("VISIT", "point-borrowed", api="Waveform::select + Selection::visit_samples", selection=shape, k=k, position="t50", ownership="borrowed")

for order in ("descending", "permuted", "duplicates"):
    add("ORDER", "explicit-times-order", api="Waveform::samples_at", k=100, q=100, window="middle10", order=order, ownership="owned")
add("ORDER", "explicit-single-time", api="Waveform::samples_at", k=100, q=1, position="t50", ownership="owned")
add("ORDER", "same-grid-repeated-points", api="Waveform::samples in a loop", k=100, q=100, window="middle10", order="ascending", ownership="owned")

for k, f, mode in product((1,100), ("M","L"), ("R","P")):
    add("TRACE", "materialize-traces", f, mode, api="Waveform::trace" if k==1 else "Waveform::traces", k=k, window="middle10", ownership="owned")
for k, method in product((1,100),("sample", "samples_at")):
    add("SAVED", f"saved-trace-{method}", boundary="T", api=f"Trace::{method}", k=k, q=100, window="middle10", order="permuted")

for k, window in product((1,100),("early10","middle10","late10")):
    add("CAND", "candidate-union", api="Selection::scan_candidate_times", k=k, window=window, oracle="ordered superset")
add("CAND", "candidates-stable-slice", api="Selection::scan_candidate_times", selection="stable-high-slice", k=1, window="middle10", oracle="ordered superset")

for variant, f in product(("reject-all", "accept-all-edges", "accept-every16-edge", "first-accepted-after-rejects", "many-drivers", "current-previous-repeat", "eager-every16-edge"),("M","L")):
    add("QUERY", variant, f, api="Selection::query + QueryContext::visit_samples", selection="query-controls-payload", window="middle10", k=109 if variant=="many-drivers" else 102, drivers=8 if variant=="many-drivers" else 1, payload_slots=100, stop="first accepted edge" if variant=="first-accepted-after-rejects" else "end")
for shape in ("shared-base", "distinct-base"):
    add("QUERY", "driver-payload-base-sharing", api="Selection::query + QueryContext::visit_samples", selection=f"query-{shape}", shape=shape, k=3, bases=2 if shape=="shared-base" else 3, drivers=1, payload_slots=1, window="middle10", predicate="every16-real-rising-edge", stop="end")

for shape, op, f in product(("active-bit", "stable-high-slice", "concat8x64", "nested16-result-bit"),("point","scan"),("M","L")):
    add("COMP", f"computed-{op}", f, api="Waveform::sample" if op=="point" else "Selection::scan_each", selection=shape, k=1, **({"position":"t50"} if op=="point" else {"window":"middle10"}))
for op in ("point","scan"):
    add("COMP", f"whole-base-reference-{op}", api="Waveform::sample" if op=="point" else "Selection::scan_each", selection="whole-low-active1024", k=1, **({"position":"t50"} if op=="point" else {"window":"middle10"}))
for k, op in product((100,10000),("point","scan")):
    add("DUP", f"repeated-handle-{op}", api="Waveform::samples" if op=="point" else "Selection::scan_each", selection="repeated64", k=k, bases=1, **({"position":"t50"} if op=="point" else {"window":"middle10"}))
for op in ("point","scan"):
    add("DUP", f"overlapping-slices-{op}", api="Waveform::samples" if op=="point" else "Selection::scan_each", selection="overlapping64-of1024", k=100, bases=1, **({"position":"t50"} if op=="point" else {"window":"middle10"}))
for shape in ("concat8-first", "concat8-identical-repeat", "nested16-first"):
    add("DEFINE", "create-composition", api="Waveform::concat", selection=shape, history="same-definition" if shape=="concat8-identical-repeat" else "none")
for shape, op in product(("mixed-xz64", "all-x64"),("point","scan")):
    add("LOGIC", f"four-state-{op}", api="Waveform::samples" if op=="point" else "Selection::scan_each", selection=shape, k=100, **({"position":"t50"} if op=="point" else {"window":"middle10"}))
for f, end in product(("M","L"),("middle10-end","eof")):
    if f == "M" and end == "eof":
        continue  # Existing OPB-PREFIX-040 is the identical baseline.
    add("HORIZON", "first-change-end-bound", f, api="Selection::scan_each", selection="staggered64", k=100, n=1, position="t50", end=end, stop="N changes; initials excluded")

expected_counts = {"OPEN":3,"HIER":24,"LOOK":34,"POINT":42,"BATCH":26,"PREFIX":42,"SCAN":14,"SESSION":20,"REUSE":19,"VALUE":10,"FLOW":6,"DIAG":16,"SELECT":5,"CACHE":14,"VISIT":5,"ORDER":5,"TRACE":8,"SAVED":4,"CAND":7,"QUERY":16,"COMP":18,"DUP":6,"DEFINE":3,"LOGIC":4,"HORIZON":3}
assert dict(counts) == expected_counts, (counts, expected_counts)
assert len(cases) == 354
assert len({c["id"] for c in cases}) == len(cases)
semantic_keys = [json.dumps({k:v for k,v in c.items() if k not in ("id", "tags")}, sort_keys=True) for c in cases]
assert len(set(semantic_keys)) == len(semantic_keys)
assert all(c["fixture"] in ("S","M","L","MH","MT") for c in cases)
assert all(c["boundary"] in ("R","P","S","T") for c in cases)
assert all(not (c["fixture"]=="S" and c["parameters"].get("k")==10000) for c in cases)
assert all(c["parameters"].get("k", 1)*c["parameters"].get("q",1) <= 1000000 for c in cases if isinstance(c["parameters"].get("k",1), int))

# A small, fixed subset of the same IDs. It creates no extra benchmarks.
quick_workloads = {"open-drop","walk-dfs","lookup-signal","point-owned","explicit-times","first-changes","full-window","point-session","materialize-traces","saved-trace-samples_at","candidate-union","accept-every16-edge","computed-point","repeated-handle-point","four-state-point","select-drop"}
quick_ids = []
for c in cases:
    p = c["parameters"]
    if c["fixture"] != "M" or c["workload"] not in quick_workloads:
        continue
    if c["boundary"] not in ("P","T") and c["workload"] != "open-drop":
        continue
    if p.get("k",100) != 100 and c["workload"] not in ("computed-point", "accept-every16-edge"):
        continue
    if p.get("q",100) != 100 or p.get("n",100) != 100:
        continue
    if p.get("position","t50") not in ("t50","shallow") or p.get("window","middle10") != "middle10":
        continue
    if p.get("order","ascending") not in ("ascending","permuted"):
        continue
    quick_ids.append(c["id"])

payload = dict(schema=1, status="proposal-not-executable", suite="Ondas Performance Benchmarks", suite_version="v1-draft",
               ondas_revision="b19cf2e8941b90bfd9abeb4487c41f7f47630228", corpus="portable-bits-v1-draft",
               count=len(cases), groups=dict(counts), quick_ids=quick_ids,
               rules_document="proposal.md", cases=cases)
if __name__ == "__main__":
    (ROOT / "cases.json").write_text(json.dumps(payload, ensure_ascii=False, indent=2)+"\n")
    print(f"Wrote {len(cases)} cases ({len(quick_ids)} quick)")
