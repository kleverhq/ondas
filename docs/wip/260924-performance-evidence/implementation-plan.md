# Make Ondas 1.0 performance claims reproducible


This self-contained execution plan can be handed to an implementing agent without the earlier conversation or integration transcripts. Implement the milestones, not another plan. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current. Paths are relative to `ondas-lib` unless another repository is named.

## Purpose / Big Picture


Ondas is a read-only Rust waveform library. Wavepeek integration exposed expensive opening, hierarchy lookup, sampling and range traversal. The release contains fixes and substantial existing tests/benchmarks, but some first-use paths are not isolated and one reference test can now use the same optimization as its subject.

The required result is a reproducible internal benchmark and a relevant correctness regression for every improved scenario below. Reuse sufficient existing cases. Measure before/after performance on identical public inputs, distinguishing cold queries from retained-state reuse. Do not require a separate historical experiment or new fixture for every commit.

Start by making evidence trustworthy, not by optimizing production code again. Change production behavior only if a new test demonstrates a defect. Timings remain local evidence, not CI thresholds.

## Non-Goals


Do not add a public metrics API, benchmark DSL, custom result store, plugin interface, mock FSDB SDK, mandatory cross-format dataset, or Wavepeek expression engine. Do not duplicate existing edge-case coverage or turn every semantic edge case into a timing workload. Individual-commit attribution and systematic mutation testing are not required.

Restoring oracle recipes for `fsdb0019-typed-values` and `fsdb0020-nine-state-ranges` is explicitly outside this task. Their existing artifacts and sidecar observations may be reused unchanged. Do not make recipe restoration a completion dependency.

Do not import private recordings, metadata or derived evidence into public artifacts/reports. Do not weaken path identity, normalization, event, projection, callback or error contracts to obtain faster timings. Consumer-side Wavepeek costs and its overall release gate are outside the library acceptance criteria.

## Progress


- [x] (2026-09-24) Audit release snapshot `4b75d96`, public issues #18–#30, relevant PRs #5–#17, test/benchmark sources, and public provider/oracle contracts; incorporate adversarial review.
- [x] Revise scope under KISS/YAGNI: four milestones, scenario-family comparisons, existing fixtures first, one new performance artifact, conditional Picorv32 conversion, no blanket per-mechanism ablation.
- [ ] Milestone 1: repair references, close focused test gaps, and connect required tests to their gates. Chronological zero-start FSDB point reference, cold/warm counters, sparse-active loop-visit regression and exact/fallback lookup checks now run; conflict fixture/gate remains.
- [ ] Milestone 2: reuse public data and supply only missing discriminating artifacts/observations. Local provider 4.7.0 has the sparse/dense FST and diagnostic FSDB with validated bytes, native inspection and oracle; FST 88-entry generate/repeat passed. Publication and a matching locked installation are still outstanding (no remote push authorized).
- [x] Milestone 3: add minimal benchmark cases, smoke execution, and accurate coverage documentation. New hierarchy/metadata, sparse/dense, VCD replay, FSDB point/interval families passed focused Criterion `--test`; full public (196 cases) and vendor (256 cases) smoke passed against the local snapshot. Durable methodology/test mapping was updated.
- [ ] Milestone 4: run family-level before/after comparisons and complete the evidence ledger.

The unchecked items are implementation work. No new timing result is claimed by this plan.

## Context and Orientation


The analyzed branch is `release/1.0.0-rc` at `4b75d96`. At analysis time `origin/main` was `36659cd`, already containing performance PRs #13–#16; reviewing only `origin/main..HEAD` misses those earlier improvements. `fixtures.lock.toml` pins `kleverhq.ondas-fixtures = 4.6.1`. Recheck these anchors before implementing on a later tree.

`src/waveform.rs` owns opening and the public `read_metadata` API. `src/hierarchy.rs` owns exact lookup and its lazy path index. `src/query/engine.rs` owns selected state, completed-tick normalization, replay and checkpoints; `src/query/engine/streaming_tests.rs` contains narrow generated-reader probes. Backend adapters are `src/backends/{vcd,fst,fsdb}.rs`; `native/fsdb.cpp` bridges the real vendor SDK.

A **selection** is an ordered list of signal handles with reusable query state. Duplicate public entries may share internal storage but must keep separate output positions. A **base** is a whole signal history; a **projection** observes selected bits. A **completed tick** includes all records at that timestamp, so persistent values are final and events have their full count. A **checkpoint** retains bounded normalized state and a reader position, not the whole history.

A **cold selection** has no replay/checkpoint state. It is different from a fresh waveform, and neither implies cold filesystem caches. A **warm selection** was deliberately primed. An **independent oracle** supplies expected observations from authored source semantics or a separate verified reader, not Ondas output. Comparing two Ondas APIs is supplementary evidence, not an independent oracle.

Keep the existing Criterion targets: `benches/vcd.rs`, `benches/fst.rs` with `benches/fst/{hotpaths,composed}.rs`, and `benches/fsdb.rs` with `benches/fsdb/{controlled,typed,composed}.rs`. `tests/support/fixtures.rs` validates catalog/artifacts; `tests/conformance.rs` runs external observations; other existing `tests/support/` modules contain normalized-oracle and consumer workloads. Small diagnostic VCD tests and private memory-reader tests need no external provider.

Durable conclusions belong in `docs/testing.md`, `docs/api-coverage.md`, `docs/benchmarking.md` and, only where their contracts change, `docs/fixtures.md` or backend documentation. Workload parameters remain Rust code, not fixture sidecar instructions.

## Surprises & Discoveries


- Public `just ci` with the local snapshot passed self-contained checks, FST/VCD conformance and public smoke; `release-check` correctly refused uncommitted changes. Re-run after committing. Full FST pool observed 88 fixtures/176 mode cases passed; full FSDB pool observed 22 fixtures, 21 positive and one empty-oracle diagnostic skip; the focused diagnostic test passed separately.
- The existing public FSDB oracle-factory verify baseline already failed for changed metadata/hashes on 0000–0018 and absent 0019/0020 recipes. New 0021 now has an explicit hashed, empty-oracle diagnostic exclusion with a producer test; the broader producer gate is still not green. A plan command temporarily generated 0019/0020 recipes; those generated files were removed without committing them, as restoration is out of scope.
- Locally regenerated sparse/dense VCD/FST bytes are identical across two neutral workspaces; all 224 public fixture hashes/sizes are unique and valid. Full FST producer generate/repeat reported 84 positive and 4 negative over 88 entries. Focused FST parity/escaped-identity and FSDB diagnostic/metadata tests passed with a two-fixture development snapshot.
- On the locked 4.6.1 provider, `just conformance-fsdb` passed 10 selected integration tests (public 45/45, optional private pool completed; no private evidence recorded here); seven ignored FSDB library tests were selected and passed. The rewritten chronological point test executes and asserts point-query route isolation.
- Existing FSDB one-shot `wave.samples` benchmarks already create fresh selections. Prepared `Selection::samples` and late scans can instead become cache/checkpoint hits during Criterion warm-up. Do not classify all existing point benchmarks as missing cold coverage.
- `fsdb_cold_points_match_chronological_reference` constructs expected values through a fresh positive-start point scan. After `46ca36c`, that reference can also seek entering state. Reference independence must be repaired.
- Hierarchy clones share the index. The first exact attempt consumes the linear-lookup opportunity even on a miss; a single sliced `signal()` request can make an exact miss followed by a second lookup/index build.
- `646a536` reduces finalization work and skips irrelevant prefix callbacks, not all query costs. Active-list setup still scans slots once; scan delivery and composed-query previous-event bookkeeping have additional selection-wide loops.
- FSDB point seeking still loads selected SDK data. A stable projection may require a long backward walk to establish `changed_at`. Rust record counts do not measure SDK decompression or all cursor advances.
- The ignored `conflicting_scope_reports_path_and_kind` test uses `ONDAS_FSDB_CONFLICT_FIXTURE` and does not match the vendor gate's `--lib fsdb_ -- --ignored` filter.
- All public integration issues #18–#30 were closed when inspected. #18 was withdrawn without an Ondas fix. #29 requested better rejection diagnostics, not acceptance of incompatible scopes. Public #30 confirmation did not establish a passing overall Wavepeek performance gate.

## Decision Log


- Reuse existing benchmarks/tests before adding cases. Reason: most older mechanisms already have useful evidence; first-use boundaries are the main gap.
- Compare scenario families against suitable pre-fix revisions, then the final implementation. Reason: the goal is to protect improved operations, not assign a percentage to each commit. Isolate individual commits only for an inconclusive result or an explicit attribution claim.
- Publish one sparse/dense FST, not a mandatory VCD/FST pair. Keep VCD as its reproducible source and use small memory/VCD tests for common-engine semantics.
- Make Picorv32 FSDB conditional on demonstrated inadequacy of the existing history fixtures. Exact reproduction of the consumer's selector list is not itself a library requirement.
- Keep the scope-conflict fixture and focused regression; do not make a new oracle-factory mode an independent deliverable. Resolve a mandatory producer-gate incompatibility explicitly if it arises; never claim an unmodified pipeline handles it.
- Retain sensitivity experiments only for the repaired point reference, actual active-slot loop counter, and warm FSDB traversal counters. No blanket optimization-removal campaign.
- Keep FSDB 0019/0020 recipe restoration out of scope. Changes to public producer/oracle repositories and publication require explicit permission; changes to `ondas-private` or private providers are not authorized.

## Coverage Ledger


The public issue URL is `https://github.com/kleverhq/ondas/issues/<number>`. The release work is PR #17; earlier performance families are PR #13 (FST), #14 (common), #15 (VCD), #16 (FSDB). “Existing” below means source-reviewed, not executed during planning. Preserve every row's scenario coverage; a sufficient existing test/benchmark satisfies a row without new code.

| Scenario / commits / issues | Reuse | Missing evidence to add |
|---|---|---|
| Path index: `7b17877`, `96d7031`, `0ace618`; #21/#27 | `path_index_is_lazy_and_shared_between_hierarchy_clones`, `indexed_paths_match_linear_lookup_across_duplicate_scope_paths`; topology open/warm lookup cases | First and second lookup with genuinely fresh hierarchy state; keep explicit warm lookup and small/large open/drop controls |
| Metadata-only FSDB: `bb7edfd`; #26 | `fsdb_metadata_without_hierarchy`, metadata/error routing checks | `read_metadata` versus full open/drop on small/large topology; metadata success versus full-open rejection on conflict fixture |
| Shared normalization and duplicate slots: `9337a3d`, `a2e829a` | Completed-tick movement/state tests, duplicate/projection semantics, existing wide scan/composed cases | Only missing retained-slot distinctions or reset controls; do not duplicate broad semantic tests |
| Active-slot finalization: `646a536`; #28 | `zero_events_do_not_activate_duplicate_slots`, streaming probes, real Picorv32 bulk samples | Actual loop-visit counter and controlled sparse/dense selected activity at several cardinalities |
| VCD replay/checkpoint/token work: `0932f39`, `bf8ca5e`, `585713c` | Replay tests, `vcd_boundary_checkpoint_skips_prefix_and_is_bounded`, Swerv composed points, compact-wide fresh-selection cases | Matched fresh late scan versus a reset series of one cold plus three repeated scans |
| FST candidates: `3fd1157`, `2c44ed1` | SCR1 section windows, wide candidate/scan pairs and string-candidate regression `4a09677` | Reuse for same-artifact/same-window historical comparisons; no new section fixture |
| FST byte validation: `cffaa97`; #25 | Exhaustive 256-byte test in `src/value.rs`, wide sampled cases, realistic all-unique Picorv32 samples | Add a bulk-cardinality case only if the existing sampled workload misses the reported shape; candidate-only timing is insufficient |
| FSDB loading/reuse/buffers: `b9f1ab8`, `beb3997`, `ed7bd91`, `81919a4` | Controlled history/wide cases; checkpoint, replay and caller-storage tests | Explicit reset/warm boundaries and both point/stream counter deltas; ensure buffer cases actually decode |
| FSDB cold points: `5faf2e7`; #22 | Existing one-shot history batches for 1/4/64 signals and typed oracles | Repair reference; cold stable-slice adverse control and genuinely warm repeat |
| FSDB cold windows: `46ca36c`; #30 | Cold-window equivalence test, history fixtures and composed consumers | Fresh-selection early/late scan, traces and candidate times; independent entering-state observations and one short nonzero-window control |
| Declaration/diagnostic fixes: `8369f6c`, `ac2d471`, `f23bb46`, `fb4d3f2`, `9b97938`, `1fd4c00`, `4b75d96`; #19/#20/#23/#24/#29 | Enum/hidden-scope/range/spelling tests and public oracles | Only missing adapter assertions and gate selection; no separate timing matrix for correctness-only fixes |

The earlier leaf-name filter `f8c8c76` is covered by hierarchy scenarios. PRs #5–#7 and #10–#12 introduced benchmark/composed baselines, not additional speedups to invent. Retain consumer-shaped driver → condition → payload → early-stop checks already under `tests/support/`; do not import Wavepeek. #18 does not justify a zero-width parameter/event compatibility special case.

## Milestone 1 — Trustworthy regression tests


First repair references and the few operation-count gaps in existing modules. Keep semantic edge cases in tests rather than multiplying benchmark dimensions.

In `src/query/engine.rs::tests`, build the cold-point reference with an explicitly zero-start chronological scan through the target tick, retaining only terminal states. Verify that reference construction adds no `point_queries`, then that the cold sample does. Compare with independent fixture observations as well: the chronological reference still shares normalization code with production. Do not perform an unrelated rewrite of every debug-string comparison; change equality assertions only where identity/value distinctions make them inadequate.

For cold-window and warm-repeat tests, snapshot counters immediately around the subject operation. A warm repeated sample must increment neither `records_read` nor `point_queries`; the existing replay test's stream-only assertion is insufficient. `point_queries` counts successful native point calls, not failed attempts. Counter bounds describe records delivered into Rust, not total SDK work.

In `src/query/engine/streaming_tests.rs`, add a small sparse-active schedule with many selected initialized values and only a few touched per tick. Include a later touch to a previously quiet slot, duplicate entries/projections, a net-equal same-tick excursion and an event absent on the next tick. Assert current/previous values and exact event counts. Instrument actual comparison/commit loop body visits in `complete_tick`; counting `active_slots.len()` alone would not detect a regression to scanning every slot. Count the per-query active-list setup separately or explicitly exclude it. Reuse existing pointer-movement, bounded-state and injected source-failure tests, including `generated_source_failures_never_publish_an_unfinished_tick`; do not introduce a mock SDK.

Retain existing hierarchy hit/miss/ambiguity/clone-sharing tests and add only missing exact-attempt versus selector-fallback distinctions. Preserve VCD replay/checkpoint invalidation and byte-budget tests, callback stop/panic recovery, FST byte-validation checks, and FSDB projection/mixed-value fallback tests. Use `fsdb0017-typed-records` observations to make glitch/event route checks explicit where missing. Do not claim every edge case needs a new test if a discriminating existing assertion already covers it.

After milestone 2 supplies the conflict artifact, replace the ad hoc path test with `fsdb_conflicting_scope_diagnostic_and_metadata_bypass` in `tests/conformance.rs`, under the existing feature/ignored-test policy. Load it using `fixture_catalog::load_artifact(&provider(), fixture_id)`, which uses the locked provider and checks containment, size and hash. Assert `Error::Backend` with path `top` and both kinds for full open; assert independently evidenced header metadata from `read_metadata`. The test name must be selected by the normal FSDB gate. No native bypass counter is needed for this case.

Only three focused sensitivity checks are required: an incorrect cold-point result is rejected independently, a selection-wide finalization loop exceeds the work counter, and an unnecessary warm native read changes the traversal counters. Use a disposable worktree if modifying implementation temporarily; never ship the modification. Other rows use ordinary correctness tests and the historical benchmark control.

Acceptance: run `./dev just test` and, after fixture setup, `./dev just conformance-fsdb`. References use the intended routes, counter assertions observe real work, required fixture tests are selected, and existing edge-case checks remain passing. The conflict test may wait for milestone 2 without blocking unrelated unit-test work.

## Milestone 2 — Minimal public inputs and independent observations


Read the producer repositories' local instructions before changes. Audited commits were `b97828e354fefe08d8d143a19e3d7f723062e004` in `ondas-fixtures-public` and `600e5d4c2135e3895f52f9c37ad238673131a459` in `ondas-fixtures-oracle`. Obtain explicit permission for the relevant repository edits and separately for publication. Allocate fixture IDs only after checking the then-current catalog.

### Reuse inventory


| Public fixtures | Concrete use |
|---|---|
| `fsdb0010-history-short`, `fsdb0011-history-long` | `top.clock`, `top.word_00` through `top.word_63`; end ticks 4096 and 1048576. Reuse one-shot/prepared batches of 1/4/64 signals. Use equal-width windows `17..=48` and `end-31..=end`, plus projected/duplicate `4000..=4001` on the short file. Verify relevant SDK view-window support before attributing a gain to skipped native traversal. |
| `fsdb0012-topology-small`, `fsdb0013-topology-many-handles`, `fsdb0014-topology-many-times` | Small/many-handles metadata and opening; sparse/constant probes. The many-handles input has 16384 histories. Existing quiet point/window are `524288` and `524288..=524320`. |
| `fsdb0015-wide-compact-toggle`, `fsdb0016-topology-many-aliases` | Whole/LSB/MSB/stable-slice/scalar and duplicate-entry controls. Keep actual paths/times already in `benches/fsdb/controlled.rs`; distinguish unique bases from declaration/output counts. |
| `fsdb0017-typed-records` | Independent glitch, event and X/Z observations. `top.glitch` returns to `0` at tick 1024 without a net normalized change; `top.trigger` has two occurrences there. Derive normalized prior change time from the supplied history, not the raw final write. |
| `fsdb0018-native-real32`, `fsdb0019-typed-values`, `fsdb0020-nine-state-ranges` | Reuse existing typed observations unchanged. Recipe restoration for 0019/0020 is out of scope. |
| `fst0083-wide-compact-toggle`, `vcd0096-wide-compact-toggle` | Existing sampled wide/scalar/projection and VCD fresh/reused work. No new wide artifact. |
| `fst0084-topology-small`, `fst0085-topology-many-handles`, `fst0086-topology-many-times` | Existing topology/open/lookup controls; no new hierarchy pair. |
| `fst0012-picorv32-test-ez-vcd` | Existing realistic all-unique bulk sampling, scan and trace. |
| `fst0015-scr1-max-ahb-coremark`, `vcd0071-swerv1` | FST section boundaries `745342`, `3248312`, `5812392`, clock windows `boundary-20..=boundary+20`; VCD late repeated window `12000..=13000`. |

Extend existing in-scope recipes only for actual new benchmark points/windows whose correctness is not already independently evidenced. A window must include entering state and every raw observation inside its inclusive bounds, including same-tick groups/events. Use the test-only normalized-oracle code to derive normalized expectations; never serialize Ondas output as its own oracle. Sparse oracles are not exhaustive file certification. Do not export a million-tick history to justify a small late window.

### New performance artifact: one sparse/dense FST


Author deterministic VCD source and convert it to FST using reviewed public tools. Publish only the FST unless a concrete additional requirement for the source VCD artifact is demonstrated. Keep source/generator and conversion provenance reproducible in the approved source repository; generate in neutral temporary paths and inspect embedded paths before delivery.

Use two banks of 64 one-bit histories over ticks `0..=4096`, all explicitly initialized. In `sparse`, one driver toggles each tick and 63 values stay constant; in `dense`, all 64 toggle. Benchmark unique selection prefixes 1, 16 and 64 at tick 4096, plus a 256-versus-4096 depth control at one fixed cardinality. Initialization and parity supply an independent formula for point values; add bounded oracle windows rather than complete output histories. A small private memory/VCD test, not a second published format benchmark, checks shared-engine corner cases.

First check whether an existing public FST supports literal escaped-name/provenance assertions. If not, add a few separate escaped declarations and an ordinary alias to this FST using the source shape in `tests/vcd_native.rs::escaped_declaration_spelling_agrees_with_fst`. Keep them outside the measured banks. A focused installed-artifact test checks `name_was_escaped`, logical identity, aliases and file/bytes parity, replacing the ignored test's runtime converter requirement. The oracle schema need not grow to encode this adapter-specific flag.

### New diagnostic artifact: conflicting-scope FSDB


Use the public issue #29 reproduction, copied to a neutral workspace:

    $timescale 1ns $end
    $scope module top $end
    $var wire 1 ! a $end
    $upscope $end
    $scope task top $end
    $var wire 1 " b $end
    $upscope $end
    $enddefinitions $end
    #0
    0!
    0"

Convert with `vcd2fsdb conflict.vcd -o conflict.fsdb` in the existing approved vendor environment, recording tool/version/hash and inspecting embedded paths. Independently inspect native declarations for the two root kinds and header metadata. Preserve the source, command and input/output hashes as reproducible evidence. This artifact serves the focused diagnostic/metadata-bypass test, not a performance matrix.

Use the provider's supported empty `oracle` for this adapter-specific case and require the focused library test. Do not label it with the generic negative oracle `open: {result: error, kind: malformed}`: `tests/conformance.rs::checked_open` expects `Error::Malformed`, while the existing rejection is `Error::Backend`. The SDK can read the declarations; inventing a native extraction failure would be false evidence. Empty-oracle discovery is not semantic coverage; report the separately executed focused test.

Do not implement a general `adapter-only` factory mode merely for this plan. A recorded native declaration inspection plus the focused regression is the evidence requirement. There is a real integration constraint: the current `fsdb.py::corpus_action` enumerates every FSDB and expects a positive recipe, and `load_recipe` rejects negative recipes. Therefore adding this catalog-only artifact does not automatically make full-factory generate/verify/repeat pass. Before delivery, resolve this with the producer's actual policy: if its mandatory complete-corpus gate requires explicit adapter-evidence handling, obtain approval and add only that minimum handling, with tests and honest separate reporting. Do not bypass a required gate, manufacture a positive oracle, or claim full-corpus success. Record any approved scope exception explicitly; without one, an unresolved required producer gate remains a delivery blocker.

### Conditional addition: Picorv32 FSDB


Do not create it initially. First run cold early/late workloads on `fsdb0010/0011`. If those files exercise the relevant SDK path, independently validate the results, and distinguish the old prefix cost from the optimized path, they satisfy the cold-window requirement.

Escalate only if those inputs fail to represent the required traversal behavior or the comparison remains inconclusive after checking setup/noise. Record the specific deficiency before generating another artifact. The fallback public source is `fst0013-picorv32-test-vcd`, audited SHA-256 `0edf567e53b1c85d6de589cf05d2435052ce6c50b9d656e74d40a5e7e9e13adf`; convert via `fst2vcd` and `vcd2fsdb` with original license/provenance preserved. Issue #30 documents the real workload; if exact selectors are necessary, resolve Wavepeek `v3.0.1` to an immutable commit and use `bench/e2e/tests_fsdb.json`, case `change_picorv32_signals_100_window_2us_trigger_posedge_clk`. It reports 95 selectors/62 distinct signals. Store required workload parameters in Rust, never fetch them at runtime.

Its optional 1ps-tick position sweep is `0..=2000000`, `1225000000..=1227000000`, `2450000000..=2452000000`. Add independent entering-state/complete-window observations rather than relying on the factory's early-time default selection. Exact candidate sets may differ; required normalized change times must be included. No exact-selector import, extra artifact, or oracle expansion is required if the existing controlled inputs suffice.

### Generation, publication and acceptance


Use the oracle repository's own workflow, not the library's `./dev` wrapper. Known entry points are `generate.py <recipe.json>` for FST, `vcd.py <recipe.json>` for VCD, and `fsdb.py <recipe.json>` for FSDB. Corpus commands provide `generate`, `verify`, and `repeat`; FSDB corpus calls must explicitly select `--provider kleverhq.ondas-fixtures` because their default is private. Follow its mandatory build/test/delivery rules; source/recipe changes require regeneration before repeat verification. Do not rebuild vendor infrastructure or run private-corpus generation under this plan.

Validate the new/changed inputs and any broader scope required by producer policy. Do not turn this task into repair of unrelated corpus gaps: report pre-existing missing recipes for FSDB 0019/0020 separately without claiming full-corpus success. Preserve reproducible sources, input/output/tool hashes and reviewed expected observations; generated raw evidence/receipts stay ignored according to producer rules. Sidecars are updated explicitly from reviewed independent output, never as a hidden generator side effect.

In the public provider, each fixture directory contains only `fixture.json` and ignored `waveform.<format>`. Check size/hash, duplicate content, provenance/license and path hygiene. Commit metadata, not waveform bytes. New artifacts require a minor catalog version bump; tag that exact commit. With publication authorization, run `python3 release.py <tag> <explicit-fixture-directories> --dry-run` before upload. Assets are `<fixture>.<sha256>.<format>.gz` containing the raw waveform. Update the library lock only after the immutable release/assets are available.

Acceptance: the locked public provider installs without private credentials; every new required artifact is validated; the relevant independent observations and focused adapter tests execute. A matching-version snapshot under library `tmp/` is acceptable during development, not final release reproducibility. Expected scope is one new performance FST and one diagnostic FSDB; reuse may reduce it, and Picorv32 needs the recorded escalation reason.

## Milestone 3 — Minimal benchmark additions and regular execution


Use existing targets and IDs when their workloads remain unchanged. New IDs must identify format, provider/fixture, file/bytes mode, operation, state boundary, parameters and backend. Put new families under `.../regression/...` for filtering. Check hashes and resolve ordinary handles outside timing; consume results through `black_box` or a minimal callback count. Correctness assertions belong in tests, not timed closures.

### Required timing boundaries


For `fresh-open`, time create/drop of a waveform; metadata-only times create/drop of `read_metadata` output. Metadata selection is automatic because no explicit-backend metadata API exists; full-open controls use `open_with`.

For `fresh-selection`, open/resolve once outside timing, then construct selection, run one query, consume/drop output and drop selection inside every iteration. This isolates absence of selection-local cache, not cold disk or all SDK residency. A `warm-repeat` case explicitly primes a dedicated selection outside timing. A cold-then-repeated series resets its selection every iteration so Criterion warm-up cannot erase the cold first operation.

First/second hierarchy lookup uses fresh per-iteration setup, normally an untimed open. Do not perform preparatory lookups or use `Hierarchy::clone` as reset. Parse paths outside timing for mechanism cases. Prime exactly one exact attempt for the second-lookup case; explicitly build the index for warm lookup. Keep waveform destruction outside lookup-only timing using Criterion's existing batching facilities. Do not return a selection borrowing an owned waveform from a setup closure.

Use isolated costs as the default. Retain a few existing fresh-open/query consumer cases, but do not add a fresh-waveform variant to every workload. State what each timer includes; no need for the Cartesian product of formats, ownership modes, query variants and semantic edge cases.

### Minimal additions by family


| Family | Required comparison |
|---|---|
| Hierarchy / metadata | Reuse small/large topology open/drop and warm lookup. Add first exact hit and second exact lookup including index build. Keep miss/ambiguity/slice fallback in tests unless a concrete timing question remains. Add FSDB metadata-only versus full open on small/large topology. |
| Active slots | New FST sparse/dense point samples for 1/16/64 unique entries, plus one fixed-cardinality depth pair. Reuse realistic Picorv32 bulk and wide duplicate/projection cases. No second published VCD scaling benchmark. |
| VCD | Reuse fresh/reused point and compact-wide cases. Add the same Swerv late window scanned once on a fresh selection versus four scans on a newly created selection per iteration: first cold, three reusable. Report queries/scans as throughput, not guessed SDK work. |
| FST | Reuse actual wide sampled cases for byte validation and existing SCR1 candidate windows/string controls. Candidate-only work does not validate sampled-byte performance; conservative candidate extras are allowed. |
| FSDB point/reuse/buffers | Reuse existing cold one-shot history batches and prepared controls. Add cold one-shot whole/stable-projection/scalar cases only where missing. Retain one explicitly primed same-time repeat. Ensure buffer comparisons decode rather than merely return cached state. |
| FSDB intervals | Fresh-selection early/late scans of equal width on short/long history; bounded trace and standalone candidate cases for a representative late selection; one documented warm repeat. Include a short nonzero window to expose extra seek/setup overhead. Existing composed consumers cover condition/payload/early-stop semantics; extend only if the new path is otherwise untested. |

Events/mixed selections, zero/max time, missing versus unknown, same-tick collisions, backwards/incompatible reuse, panic/errors and declaration corner cases need correct tests, not dedicated timing cases by default. Stable projections remain a benchmark exception because backward change-time proof is a concrete cost limitation. SDK loading, unsupported view-window behavior and skipped-prefix validation limits must remain explicit: seeking does not promise constant time or sequential validation of skipped records.

Keep `just check-local` fixture-free. Add thin `bench-smoke` and `bench-smoke-fsdb` recipes around existing `cargo bench ... -- --test` commands; run them in the corresponding fixture-backed public/vendor gates after conformance. Vendor smoke does not enter public GitHub Actions. Retain at least smoke execution of every new family; measure practical duration and disclose any proposed scope adjustment, never silently omit a required case. No elapsed-time thresholds.

Update `docs/benchmarking.md` to describe real cold/warm boundaries and correct stale claims that all cold FSDB reads replay the prefix. Update `docs/testing.md` and `docs/api-coverage.md` with actual tests/counter limits; reuse existing documentation rather than adding another coverage system.

Acceptance: relevant correctness tests and Criterion `--test` runs pass; required missing inputs fail; every new family is exercised by its appropriate gate. An unchanged existing benchmark can satisfy a ledger row without being renamed or rewritten.

## Milestone 4 — Scenario-family comparisons and evidence


Select one suitable pre-fix revision per family and compare it with the final implementation using the same harness, artifact hashes, provider snapshot, compiler/SDK and timer boundaries. The following anchors identify pre-fix choices, not a command to benchmark every intermediate commit:

| Family | Useful historical control |
|---|---|
| FST candidate ownership/section filtering | `3fd1157^`, before the PR #13 changes |
| Common normalization/duplicate retention and indexed lookup | `9337a3d^`, before the PR #14 sequence; use `7b17877^` only if isolating indexing is necessary |
| VCD replay/checkpoint/token reuse | `0932f39^`, before the PR #15 sequence |
| FSDB loading/checkpoint/final-state/buffer work | `b9f1ab8^`, before the PR #16 sequence |
| Integration-specific opening, lookup, byte validation, active slots and cold seeks | `36659cd`, before the release-branch integration fixes; `646a536` is a narrower existing control for the cold-window fix if needed |

Choose workloads that expose each ledger scenario within these families. Grouping comparisons does not justify dropping a scenario or assigning a measured gain to an individual commit without isolation. Use an intermediate parent/commit pair only when interacting changes obscure the result, a regression needs attribution, or an individual-mechanism speed claim is intended.

Backport only test/benchmark harness and necessary test input compatibility to old revisions, not later production optimizations. If an API did not exist, compare equivalent old behavior: `read_metadata` is new, so the historical control is full-open metadata extraction. Run relevant correctness checks on the timed revisions; an old wrong result is not a legitimate fast baseline. Disclose incompatible cases instead of changing data silently. A temporary, disclosed removal of one optimization is a fallback for an unavailable honest historical control, not a required deliverable.

Use Criterion's baseline mechanism. Repeat the unchanged revision to assess environmental variation, then perform a controlled before/after comparison, confirming the result with reversed run order. Add more runs when noise or overlapping effects prevent a conclusion, not a fixed experimental matrix for every commit. Only one timing process should run at a time. Worktrees do not share Criterion results automatically; preserve/copy `target/criterion` explicitly while keeping incompatible SDK build outputs separate.

For each scenario leave a compact ledger entry: correctness test, optional relevant work counter, benchmark ID, fixture version/hash, baseline/final revision, timer state boundary, observed change and uncertainty, adverse/control result, supported claim and limitation. Reuse the table in this WIP plan during review; raw outputs remain ignored. Do not require a new counter for every mechanism or repeat existing full transcripts.

Acceptance: all performance scenarios have appropriate reproducible comparisons and relevant correctness evidence, including reused cases; correctness-only fixes have executed regressions. A warm cache hit, smoke pass, integrator approval, skipped test or unrelated unchanged benchmark is not proof of a cold-path gain. An inconclusive required comparison remains open; investigate or narrow the claim honestly rather than fabricate success. No universal percentage or millisecond gate is introduced.

## Concrete Commands and Environment


Keep Git, editors and credentials on the host; run library project commands through `./dev`. Before SDK/private-fixture work in a checkout, compare existing `.env`, `.devcontainer.local/devcontainer.json` and `fixtures.private.lock.toml` with canonical copies in neighboring `ondas-private`. Copy missing files or explicitly approved replacements, preserve local edits and ignored status, and never synchronize changes back. Do not expose their contents in reports.

Start from the library root:

    git status --short
    git rev-parse HEAD
    ./dev --install-hooks
    ./dev just --list

Install reviewed hooks in new comparison worktrees too, and start their containers before any commit. No hook bypass or destructive reset is needed. Install the locked provider explicitly; use a fresh destination when an existing checkout is pinned to a different version rather than resetting it.

Relevant validation commands already available are:

    ./dev just fixtures-install
    ./dev just test
    ./dev just conformance
    ./dev just conformance-fsdb
    ./dev cargo bench --locked --bench vcd -- --test
    ./dev cargo bench --locked --bench fst -- --test
    ./dev cargo bench --locked --features fsdb-lib --bench fsdb -- --test

FSDB commands require the actual configured SDK and public payloads. Public benchmark workloads must not consume the private provider even if installed. Record selected/passed/failed/skipped tests honestly; ordinary ignored tests do not establish external coverage.

After implementation, run the normal gates, with the new smoke recipes connected as described above:

    ./dev just ci
    ./dev just msrv
    ./dev cargo test --doc --locked
    ./dev just ci-fsdb

For a local matching-version provider snapshot under `tmp/performance-fixtures/kleverhq.ondas-fixtures`, use an explicit container-local override for every related test/benchmark instead of modifying canonical configuration:

    ./dev bash -c 'export ONDAS_FIXTURES="$PWD/tmp/performance-fixtures"; cargo test --locked --features fsdb-lib --test conformance full_fsdb_pool -- --ignored --nocapture'
    ./dev bash -c 'export ONDAS_FIXTURES="$PWD/tmp/performance-fixtures"; cargo bench --locked --features fsdb-lib --bench fsdb -- --test'

Run focused adapter tests as well; full-pool execution alone does not cover the empty-oracle conflict case. Final reproducibility still requires the published locked provider.

Example family comparison commands, first on the chosen control and then on the final revision with preserved Criterion output:

    ./dev cargo bench --locked --bench fst -- regression --save-baseline before
    ./dev cargo bench --locked --bench fst -- regression --baseline before
    ./dev cargo bench --locked --features fsdb-lib --bench fsdb -- regression --save-baseline before
    ./dev cargo bench --locked --features fsdb-lib --bench fsdb -- regression --baseline before

Use the analogous VCD target and actual existing-case filters where reusing old IDs. `regression` selects proposed new IDs, not every required historical workload.

## Idempotence, Dependencies and Delivery Gates


Keep measurements, scratch patches and provider snapshots in ignored `tmp/`. Use dedicated comparison worktrees; never revert the active working tree in place or delete another person's artifacts. Do not change fixture bytes under an existing immutable provider version. Generation, installation and publication are explicit, never hidden inside tests.

Use existing Criterion, fixture validation and private generated-reader facilities. Add no runtime dependency or public API. Test-only counters must not add release allocations or become a public metrics framework. Keep new helpers local unless there is a second real caller.

Permission for public provider/oracle edits, public redistribution/licensing, an actual SDK run and required producer gates are explicit dependencies. A required new artifact or SDK check cannot be replaced by private evidence or an unreported skip. Conversely, optional Picorv32, a second published format, per-commit attribution and out-of-scope 0019/0020 recipe repair are not completion gates.

## Outcomes & Retrospective


Planning is complete; implementation is not. The audit found substantial existing coverage and focused gaps in reference independence, test selection, first-use measurement and sparse selected-slot scaling. This KISS revision reduces required new data and historical experiments while retaining all scenario rows and correctness requirements. No production code, provider or oracle implementation was changed; no build, test or timing was executed for this revision.

At implementation completion, record actual delivered coverage, comparison conclusions and any unresolved required checks. Before merging into `main`, promote durable methods/coverage into their owning documents and remove this task's WIP directory; preserve `docs/wip/AGENTS.md` and unrelated tasks.

Revision note: replaced the six-stage plan with four milestones after the user-approved KISS/YAGNI review. Picorv32 is conditional, the sparse/dense fixture publishes only FST, comparisons default to scenario families, and blanket mutation tests and benchmark edge-case matrices are removed. Oracle-factory development is limited to any genuinely required producer-gate accommodation, not a standalone adapter-mode project. FSDB 0019/0020 recipe restoration remains excluded.
