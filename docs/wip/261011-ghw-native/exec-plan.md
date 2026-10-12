# Implement a native GHW backend for Ondas

This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective current. Follow the exec-plan skill when revising it. The implementation agent must update evidence and decisions as each milestone completes.


## Purpose / Big Picture


Implement a private native Rust reader named ghw-native. A user must be able to open a GHDL Waveform (GHW) recording from a file or shared bytes, inspect its declarations, and use all existing Ondas point, range, projection, candidate, and composed-query APIs. For example, a descending array and a generated instance that refers to its first bit must return the same source value at a chosen tick, rather than silently exchange bits.

The implementation must fit Ondas rather than import a viewer's data model. Its type information and scalar identifiers remain private. Opening validates the recording and determines its recorded time span. Queries keep current selected values, not complete histories of every source signal. Native Rust decoding must return an Ondas error for bad input; it must not invoke a C reader that can terminate the application.

“More complete than wellen” means satisfying the concrete type and regression acceptance below against wellen 0.25.6 at commit 34f40595c4330bcac364397679cdc3d5ae6b5c0c. It does not mean supporting every VHDL construct, repairing a broken producer, or counting a successful file open as value correctness. “Fast enough” means measured performance on the named realistic and controlled workloads, together with deterministic tests of bounded retained state, shared decoding, and early termination. For this implementation pass, benchmark execution is the acceptance gate. Ignore timing results and defer tuning to a separate pass.


## Non-Goals


Do not add a waveform writer, format conversion, a public VHDL type database, a public backend enum, a plugin interface, capability flags, arbitrary expressions, transactions, or a delta-cycle coordinate. A delta cycle is another simulator update at the same physical time; Ondas observes the final recorded state at that tick.

Do not use wellen, rs-ghw, ghw-sys, or libghw as the production GHW dependency. The existing wellen development dependency can remain a reference for compatible benchmark cases. Do not copy its compressed-history implementation or translate upstream source wholesale into the library.

The first delivered backend accepts uncompressed GHW. Gzip and bzip2 are outer compression containers, not additional GHW sections. Their decoding, shell decompressors, memory mapping, parallel body parsing, and a persistent global time index are outside this change. Existing unknown/unsupported errors must describe these limits without subprocesses or temporary-file conversion.

Do not synthesize source constants, generics, event occurrences, physical-unit APIs, signed/unsigned vector metadata, or text strings from names and observed values. VHDL character arrays remain readable arrays of enumerated leaves. This preserves their data without guessing a character code page or introducing a second aggregate representation.


## Progress


- [x] (2026-10-11) Create the feat/ghw-native worktree from main at 6aec34cd7b39ecd7ee44075fac5c04a941a1888b and install its host hooks.
- [x] (2026-10-11) Read the current API/model, fixture strategy, benchmark policy, GHDL producer/reader, and wellen 0.25.6 implementation.
- [x] (2026-10-11) Inspect the relevant wellen and GHDL issues, including the linked Surfer reports.
- [x] (2026-10-11) Confirm the initial pinned corpus has 93 GHW sidecars, 139982142 artifact bytes, and zero nonempty GHW oracles.
- [x] (2026-10-11) Complete three focused Astra high reviews, resolve their three distinct substantive findings, and pass targeted re-checks plus an independent Astra high control review.
- [x] (2026-10-12) Inspect the current VCD/FST readers, shared query engine, independent oracle derivation, fixture-backed workload checks, and Criterion boundaries. Run 113 relevant self-contained tests successfully; one optional converter test remains ignored.
- [x] (2026-10-12) Review the VCD/FST-informed revision with two focused Astra high reviewers and a fresh independent Astra high control reviewer; all three report no substantive findings.
- [x] (2026-10-12) Create local feat/ghw-oracles branches in this worktree's fixtures submodule and in ondas-fixtures-oracle; inspect the factory workflow and its current GHW/provider-layout gaps.
- [x] (2026-10-12) Review the local fixture/factory workflow with Astra high, fix the unrelated full-suite gate, pass its targeted re-check and a fresh independent Astra high control review.
- [ ] (2026-10-12) Implement the independent native factory GHW reader and grouped-layout orchestration; create and verify external evidence.
- [ ] Milestone 1: establish independent fixture evidence, the source-format probes, and benchmark workload definitions. Factory foundation and source recipes are implemented and OCR-reviewed; image/producer validation, oracle generation/import and final receipt rebinding remain pending.
- [x] (2026-10-12) Add test-only native wire primitives: checked fixed fields, 32-/64-bit LEB128, both byte orders, lossless string octets and five-bit prefix decoding. Four GHW unit tests and check-local pass; public dispatch remains disabled until full reader integration.
- [x] (2026-10-12) Milestone 2: implement and test the checked binary primitives and supported type grammar. Eleven self-contained GHW tests pass, including 2048 nested constraints; eight named real-source headers pass. OCR completed; replace invariant panics with typed failures, defer its allocation/performance suggestions as instructed. Public backend dispatch remains disabled until milestone 4.
- [ ] Milestone 3: implement hierarchy mapping, ordered signal recipes, and alias identity.
- [ ] Milestone 4: implement bounded streaming reads and integrate the public opening/query APIs.
- [ ] Milestone 5: complete issue regressions and external GHW conformance.
- [ ] Milestone 6: add conformance-checked benchmark workloads and verify that they execute; ignore timings and defer tuning.
- [ ] Milestone 7: update authoritative documentation, run final gates, and remove tracked WIP before merge.


## Surprises & Discoveries


The imported original issue-99 recording was independently scanned by pinned libghw: cpu_core_tb.uut.reg_file.addr_c uses ordered scalar IDs 828..832 and changes to 00001 at tick 150000000, then 00000 at 152000000. The original issue-90 archive contains a 4232-byte GHW file with genuine multidimensional arrays. Both original artifacts are installed locally as ghw0093-wellen-issue-90 and ghw0094-surfer-issue-547; sidecars await the fixture commit and factory-oracle import.

GHW has no separate complete frozen standard. The GHDL documentation says that its writer and ghw utilities evolve together. Use the pinned writer and reader as the executable format specification, with the wire rules embedded below. An unofficial Kaitai description is incomplete and assigns some numeric tags differently; it is not normative.

A previous compatibility probe compiled the unmodified pinned GHDL libghw and scanned the original 93-file corpus in separate processes. It completed 89 files. The ghw0002-ghdl-issue-538 file caused exit(1) while decoding range type 35. Three files ended inside a cycle: ghw0044-cpu-uppercase, ghw0050-cpu-lowercase, and ghw0059-toplevel-tb. Their physical EOF offsets were respectively 20998463, 1611294, and 103139396. This is parser-compatibility evidence, not proof that the other values are correct or that the first failure is a valid unsupported construct. Milestone 1 must reproduce and classify these cases rather than accept or exclude them by filename.

Among those 89 completed scans, 88 contained repeated records at the same tick, and two had holes in their scalar-ID tables: ghw0001-wellen-ghw-bug and ghw0089-tippenslab1-waveform. Their cycle deltas must be interpreted through active scalar ranks, not added directly to raw source IDs.

The exact source inspection matters more than issue status. Wellen issues 6, 12, 32, 34, 35, 53, and 84 are closed, but they still describe valuable regressions. In the inspected 0.25.6 source, multidimensional arrays, TypeP32, some enum constraints, some vector aliases, and SubtypeUnboundedRecord still reach incomplete branches. TypeI64 is parsed but is not handled by add_var. Issue 33 describes an alias-filtering bug in the Python hierarchy iterator, not an independent GHW decoder defect.

GHW timestamps have a conventional interpretation of femtoseconds, but no encoded time-resolution field. GHDL issue 1769 reports incorrect physical scaling when a producer uses nondefault simulation resolution. Keep raw ticks and document that the reader cannot infer or repair missing resolution information.

The existing VCD reader already validates the whole source at opening and uses the shared engine for selected replay and completed-tick state. FST opening instead uses its section directory and geometry without validating every value. GHW DIR does not provide equivalent time/handle seeking. Reuse VCD's serial architecture and FST's model conversion practices; do not promise FST's opening or chain-filtering behavior for GHW.

VCD's 4 MiB limit applies to its additional query-boundary checkpoint, not all reader/selection memory or its last replay state. Slot accounting excludes allocator overhead. GHW scalar assembly buffers also need explicit accounting; existing slot counters alone cannot prove a new backend's memory bound.


## Decision Log


Decision: Implement a built-in native Rust backend named ghw-native without a new Cargo feature or runtime dependency. Rationale: it uses the existing input and private reader interfaces and needs no vendor runtime. Date: 2026-10-11.

Decision: Use KISS (“keep the implementation simple”) and YAGNI (“do not build a feature before it is needed”) as design constraints. Use a few private modules, explicit source-to-common-model conversion, and one sequential body decoder. Do not create a general decoder framework, configurable strategy system, or history database. Date: 2026-10-11.

Decision: Support format versions 0 and 1 and both encoded byte orders for the specified storage forms. Rationale: the canonical reader supports both versions; version 0 only changes the physical-unit table in this grammar. Cross-endian fixed fields and real values are necessary for the existing header contract. Tests must distinguish authored wire cases from real producer evidence. Date: 2026-10-11.

Decision: Preserve ordered explicit scalar-ID lists, and identify whole histories by their ordered IDs plus their public runtime encoding. Rationale: source IDs are not required to form an ascending interval, and declaration metadata must not control alias identity. Date: 2026-10-11.

Decision: Represent records and non-vector arrays as ordinary scopes with readable leaves. A one-dimensional integer-indexed array of bit/std_ulogic elements becomes a whole bit vector. A null vector remains a declaration without a signal; do not expose Bits width zero or a misleading nonempty BitRange. Date: 2026-10-11.

Decision: Use sequential prefix replay first, with a narrow private GHW tick-boundary callback in the existing query engine. Defer a bounded continuation and all tuning to a later performance pass. Rationale: value callbacks alone cannot finish a selected tick before an unrelated-only suffix, and a GHW byte offset alone cannot restore scalar/vector state. Neither need requires an index or framework. Date: 2026-10-11.

Decision: Successful conformance requires independent expected observations. Wellen can be compared where it supports the case, but is not the oracle for cases that demonstrate its limitations. Date: 2026-10-11.

Decision: Reuse established Ondas ownership, active-slot finalization, oracle normalization, fixture preflight, and benchmark setup boundaries. Add only GHW-specific decoding, recipes, tick notifications, and their necessary probes. Rationale: VCD/FST already exercise the public contracts; rebuilding those semantics inside GHW would create competing implementations. Date: 2026-10-12.

Decision: When GHW becomes a second caller of the pure query consumers in tests/support/fst_workloads.rs, move those unchanged functions to tests/support/query_workloads.rs and use them from FST and GHW checks/benches. Keep concrete GHW fixtures, paths and ranges in ghw_workloads.rs. Rationale: sharing actual identical consumers avoids drift without creating a test framework. The VCD-specific counter predicate remains VCD-specific. Date: 2026-10-12.

Decision: Accumulate fixture metadata/oracle commits on the local feat/ghw-oracles branch inside this worktree's fixtures submodule. Develop and commit GHW extraction tools, authored sources and recipes on the separate feat/ghw-oracles branch in the neighboring ondas-fixtures-oracle repository. Generate there, then explicitly copy reviewed outputs into fixtures. Do not push, create release tags, upload payloads or publish either repository; the user will release the accumulated fixture branch later. Rationale: this is the user's explicit cross-repository workflow and supersedes the earlier requirement for remote fixture availability before local acceptance. Date: 2026-10-12.

Decision: Use only the current grouped provider layout and Git-commit identity for GHW factory inputs. Do not implement a legacy catalog/flat-layout fallback or its compatibility tests. Rationale: the user explicitly removed that requirement; a single input layout keeps the GHW path smaller. Date: 2026-10-12.


## Outcomes & Retrospective


The native wire/type decoder is implemented and committed. The hierarchy mapper is implemented and has passed fifteen self-contained GHW tests plus the two named issue hierarchy probes. Public GHW opening is still disabled pending the streaming body and query integration. The independent factory tool/source foundation is committed at e9e4a3b; its pinned image is built, and all seven authored source cases have produced clean recordings. Original issue-90/99 oracle bundles have been generated; verification, import and final provider rebinding remain in progress. No GHW performance claim is made. Append implementation and gate results here as milestones finish.

Three read-only Astra high reviewers inspected specification/completeness, Ondas API integration, and executable/performance/KISS design. The specification lane reported no substantive findings. The other lanes identified three distinct medium-severity gaps: missing producer provisioning, unsupported-layout errors outside schema version 1, and missing selected-tick completion before unrelated-only suffixes. This revision provides a pinned GNAT/mcode build route, directed negative tests without schema expansion, and the private completed-Tick integration. Both affected lanes re-checked the fixes and reported no remaining substantive findings. A fresh independent Astra high reviewer checked the consolidated plan and also reported no substantive findings.

Planning validation passed: the existing recipe inventory was read through ./dev just --list, cargo fetch --locked completed, and the unmodified pinned ghwdump compiled with the documented GCC flags through ./dev and printed its help successfully. Structure checks confirmed heading spacing, checklist placement, and clean whitespace. These checks validate plan structure and available reference tooling; they do not substitute for the future backend tests, producer capture, conformance, or benchmark milestones.

The VCD/FST study validated the current starting point with cargo test --locked --lib (76 passed), the vcd_native/query_contract/query_composition targets (20/2/5 passed, one optional vcd2fst test ignored), and conformance's normalized_oracle filter (10 passed). All commands ran through ./dev. Fixture-backed VCD/FST conformance and Criterion timings were not rerun; source inspection establishes transferable design and test patterns, not fresh external coverage or a speed claim. Two fresh Astra high reviewers checked test/oracle/benchmark completeness and architecture adequacy. Both reported no substantive findings. A separate fresh Astra high control reviewer independently checked the consolidated revision and also reported no substantive findings. The revised plan and its completed review are the deliverables; production code is unchanged.

The user subsequently authorized development in the neighboring oracle factory and local fixture branches, with no push or release. Both feat/ghw-oracles branches now exist and retain their original contents. The factory starts at e030bbd1f27eade0f10311f46bd197fbe7d1df5c. Its GHW reader is only planned, and its current resolve_input expects the older flat catalog layout. Milestone 1 must add the GHW extraction path and current grouped-provider support there. The user removed old-layout compatibility. A focused Astra high review found that unconditional full factory test success would then require an unrelated legacy migration; focused GHW/helper checks are the acceptance gate, with full-suite before/after reports and no new failures. The reviewer re-checked the correction and reported no substantive findings. A fresh independent Astra high control review also reported no substantive findings. Structure/whitespace and local branch/path checks passed. No factory tools, recipes or oracle outputs have been implemented by this planning revision.


## Context and Orientation


Ondas is a read-only Rust library, currently version 1.0.2 with a minimum supported Rust version of 1.88.0. Public contracts live in src/ Rustdoc. docs/model.md explains the implementation boundaries. A declaration is a named source object with metadata; a signal is the identity of an observable value history. Several declarations can be aliases of the same history. A static bit projection is a normalized slice of a signal, not another source declaration.

src/waveform.rs owns open, open_with, open_bytes, open_bytes_with, read_metadata, format detection, the Format enum, and Waveform construction. Format::Ghw and GHDLwave signature detection already exist. An automatically recognized GHW currently has no reader; explicit ghw-native is currently an unknown backend. Content recognition wins over filename extensions, and explicit backend selection never falls back.

src/backends/mod.rs contains the private Reader enum and Input trait. Input means BufRead + Seek + Send + Sync. File opening supplies a buffered file; byte opening supplies a cursor over Arc<[u8]>. Add a private ghw module and Reader::Ghw without exposing either. Preserve Waveform: Send + Sync.

src/hierarchy.rs stores ScopeData, VariableData, EnumerationData, signal encodings, waveform identity, exact hierarchy components, whole alias identity, and projections. ScopeData has no public VHDL type object or multidimensional range field. VariableData.signal holds a whole signal index, not an arbitrary scalar-ID list or projected handle. Therefore partial-overlap declarations receive their own whole signal recipes; exact whole recipes share a signal. A scalar declaration is not required to compare equal to a slice of a wider declaration. Existing aliases(signal) returns declarations for the same base and does not invent slice declarations.

src/value.rs represents all nine Logic states, borrowed/owned bit vectors, binary64 reals, strings, and events. Numeric interpretation belongs to callers. Real equality preserves the supplied bit representation, including signed zero and NaN payloads. Do not serialize a real through decimal text during decoding.

src/query/engine.rs owns selection order, duplicate entries, base deduplication, projections, normalization at a completed tick, entering states, candidate traversal, and composed queries. Its existing specialized VCD/FSDB replay positions do not automatically support GHW. Reader::read supplies the common value interface; read_ticks_from also needs a narrow GHW arm for completed physical ticks. Preserve callback lifetimes, break propagation, foreign-handle validation, error propagation, and current/preceding-tick access.

tests/conformance.rs and tests/support/fixtures.rs discover pinned artifacts, verify sizes/hashes, and apply independent sparse oracles through explicitly selected readers. The current default pools are FST and VCD; add GHW explicitly. The runner supports only an executable subset of the fixture schema. Enumeration, unsupported-signal error, and optional spelling assertions need focused additions or directed public API tests; do not silently accept an assertion that the runner cannot execute. Signedness and logic-domain assertions are not in schema version 1 and must remain directed tests.

benches/vcd.rs and benches/fst.rs show Criterion registration with shared fixture loading. Add benches/ghw.rs as a format-specific target. Timing cases must use real pinned artifacts and successful semantic preflight, not malformed recordings or runtime fixture generation.

The current docs/ghw.md incorrectly describes a planned wellen adapter. Replace that direction when documenting the implemented native backend. The plan is temporary tracked material under docs/wip/261011-ghw-native/. Promote durable conclusions to docs/ghw.md and Rustdoc, then remove the task directory before merging into main.


## Local Fixture and Oracle Factory Workflow


The fixtures submodule and the oracle factory are separate Git repositories. Use feat/ghw-oracles in each. The library remains on feat/ghw-native. The factory checkout is the neighboring ondas-fixtures-oracle repository, not its unrelated WLF worktree. Follow each repository's own AGENTS.md and README: library commands use ./dev, fixture operations use its justfile through ./dev, and factory Python/Docker commands run on the host as its operating contract specifies. Host Git owns all branch/commit operations. The user explicitly authorized changes to this sibling repository; that authorization does not extend to other neighbors.

Implement the independent GHW path in the factory: readers/ghw.c as a thin adapter over pinned GHDL libghw, Dockerfile.ghw with pinned public build inputs, ghw.py for recipe-driven generation/verification/repetition, tests/test_ghw.py, and authored source/reproduction material under sources/ghw/. Add only the required GHW source/build files to its deny-by-default .dockerignore; generated outputs and provider inputs must stay outside build contexts. Reuse existing factory hashing, sparse-history and staged-output helpers where their contracts fit. Preserve exact native ticks, scalar identity, ordered same-tick observations, binary64 patterns and completion diagnostics. Never call Ondas or wellen to create expected values. Add GHW commands to the factory's README and keep its one-native-reader-per-format boundary.

The factory's current generate.py resolve_input expects catalog.json and flat provider directories. Implement GHW input resolution for ghw/<fixture>/fixture.json and Git-commit provider identity in ghw.py; reuse hashing/validation primitives rather than invoke the old resolver. Inspect the grouped-layout portion of provider_metadata/resolve_input in the factory's WLF branch as a concrete reference; do not merge unrelated WLF work wholesale. Use only the current grouped layout. Add tests for containment, Git identity, malformed sidecars and hash mismatches. No catalog/flat-layout fallback or legacy compatibility tests are required, and no unrelated corpus migration is part of this GHW task.

Use an ignored provider link dedicated to this GHW task which resolves to this worktree's fixtures checkout. Do not repoint the factory's existing default provider link or mutate inputs during generation. The reader mounts input read-only, pins its image/native source, and fails visibly on missing input, incomplete extraction or an unexpected native diagnostic. GHW plan/inspect must report unsupported or unreviewed cases without inventing negative oracles; generation and repetition operate on reviewed recipes and report omitted coverage separately.

Track factory reader/generator sources, tests, Docker build definitions, authored source recipes and fixtures/kleverhq.ondas-fixtures/<fixture>/recipe.json. Commit those changes locally in the factory branch. raw.jsonl, oracle.json, tags.json, receipt.json, generated waveforms and reports remain ignored. Produce authored waveforms only from copied sources in a neutral disposable workspace, including container-visible paths, and inspect identifying paths before accepting them. Store generation/provenance recipes in the factory; the fixture submodule stores sidecars and ignored waveform payloads.

Generate, verify and repeat the selected GHW bundles in the factory before import. Match each bundle's provider/fixture identity and artifact SHA-256 to the destination. Explicitly copy its oracle object into fixture.json.oracle and merge reviewed suggested tags, preserving artifact/provenance and existing tags. Do not hand-edit generated oracle output. Copy new waveform payloads separately into ignored waveform.ghw paths and verify hashes. Update fixture README statistics and validate/check/test, then commit sidecars, required licenses and README locally on the fixture branch. The factory generator itself never writes sidecars.

Factory receipts fingerprint provider commit and sidecar content. Importing an oracle and committing the fixture branch therefore invalidates the earlier receipts. After that fixture commit, run generate again against the final unchanged provider, require byte-identical raw/oracle/tags data relative to the verified pre-import bundles, then verify and repeat. Receipt provenance may change; observations must not. Keep the final provider commit stable during this pass. If any observations change, investigate before another import; do not enter an automatic copy/regenerate loop. Record final factory/fixture SHAs, image ID and evidence hashes in local task evidence. Generated receipts remain ignored.

Local acceptance uses the committed fixture branch and installed/generated payloads directly. The installer verifies existing valid local payloads without a release; old missing artifacts can still be downloaded from existing releases. Do not use --ignore-missing. Record the exact local fixture SHA used by conformance/benchmarks; a parent Git entry can select it when needed, while explicitly remaining unpushed. No remote availability or new release is a prerequisite for these local results. Fresh external checkouts will need those commits and payloads after the user's later separate delivery; do not claim independent remote reproducibility in the meantime.


## Established VCD and FST Practices to Reuse


### Reader ownership and efficient decoding


Read src/backends/vcd.rs Reader::open, walk_inner, read_from, Position, and body_token_buffers_grow_with_token_size_not_record_count. Its serial pass shares validation between opening and queries, reads buffered input with an explicit offset, reuses token/value buffers, and validates source payloads before selection filtering. Apply those choices to binary wire access and reusable GHW scalar/vector buffers. Keep opening validation free of selected-value allocation. After validating a source logic code and constructing valid digits, use BitsRef::from_validated_ascii as VCD does; do not validate a whole assembled vector again for each scalar update. Unknown identifiers and invalid unselected payloads still fail.

Use a source active-ID table and direct rank lookup for GHW cycle deltas. Use ordinary bounded vectors/maps for selected dependencies and affected recipes. Do not import VCD's printable base-94 identifier encoding, token parser, padding/truncation rules, dump controls, Unix chunk boundaries, or parallel worker machinery. GHW layouts determine exact scalar counts and value widths; its body state cannot be reconstructed from a VCD timestamp cut.

Read src/backends/fst.rs Reader::open and interpretation for declaration/history separation, compatible alias encodings, type metadata, scope balance, and absent-versus-present information. Apply those principles using GHW's own type IDs and WKT evidence. FST supplemental metadata and its name/range normalization are format-specific. Its Latin-1 byte conversion is a useful explicit lossless-conversion example, not proof of GHW string-pool encoding. Keep the source-byte probe required in milestone 1.


### Shared query state and resume semantics


Selection::new in src/query/engine.rs already groups projections by base history, deduplicates identical selected handles into retained slots, and keeps caller order/duplicate outputs through slot_indices. complete_tick compares and commits only active_slots; its callback sees entering and pending state before commit. project runs before per-slot comparison. Preserve these paths for GHW rather than implement another normalizer, projection cache, or candidate list. In the body decoder, track touched recipes so finalizing a change list does not scan every selected recipe.

VCD read_from clears its parser position on failed/stopped traversals. The engine takes provisional replay/checkpoint ownership before traversal and restores it only after successful completion. sample_visit also holds caches provisionally during user callbacks. Reuse this invalidation and ownership pattern for GHW, including visitor unwinding, while preserving its explicit Value/Tick notifications. FST has no corresponding cross-query replay cache; it restarts stateful reads from zero and uses a different start..=end filter only for candidate-only scans. Keep GHW on the shared candidate fallback initially; an additional candidate-only decoder needs measured justification.

The tests vcd_replay_avoids_prefix_and_invalidates_on_stop, vcd_missing_state_reuses_position_and_initial_break_discards_it, and vcd_boundary_checkpoint_skips_prefix_and_is_bounded in src/query/engine.rs show the required resume proof: successful reuse reduces actual record reads, missing selected state does not lose position, and a stop discards reuse. If a GHW checkpoint is added, retain decoder assembly state even when selected public slots contain only narrow projections. Bound all saved GHW continuation state as specified in milestone 6; that is a deliberate GHW bound, not a claim that all existing VCD memory is capped at 4 MiB.


### Tests and independent oracle derivation


Use tests/vcd_native.rs reused_selection_matches_fresh_replay, replay_after_break_eof_and_batched_projections, indexed_scan_identifies_alias_projection_and_event_slots, rejects_malformed_full_body_not_just_selected_values, and metadata_contextual_codes_and_buffer_boundaries as patterns. For GHW use persistent bits/enums/integers/reals, absent/delayed states, aliases and exact names. Do not invent GHW events or dump controls merely to duplicate VCD cases. Test tiny buffered reads crossing markers, fixed fields, varints and compressed prefixes, then reuse the same selection through repeated/forward/backward points, EOF, early stop and injected I/O failure. Fresh-versus-reused agreement is a consistency check; it needs literal or external expected values as well.

Keep src/waveform/semantics_tests.rs, src/query/engine/streaming_tests.rs, tests/query_contract.rs and tests/query_composition.rs as the shared semantic baseline. Their generated reader/counters already distinguish retained state, raw advances, completed delivery and caller-owned copies. Add narrow GHW counters for new scalar/recipe/buffer/continuation storage, not a configurable mock or public metrics interface. Existing active-slot counters should continue to pass after the GHW engine arm is added; they measure comparison/commit visits, not every possible loop in query or delivery code.

Reuse tests/support/fixtures.rs for path containment, sidecar loading and one-time size/SHA-256 verification. Reuse tests/conformance.rs discover, check_oracle_support, pool_queries, sample_queries, window_queries, scan_queries and full_pool. The full pool batches equal-time points and equal-bound windows and already derives composed-query current/preceding-tick expectations from WindowEvidence. Some extra candidate and duplicate checks in pool_queries are currently FSDB-only; adding GHW format dispatch alone does not enable all of them. Add explicit focused GHW coverage using the existing helpers and independent windows without changing unrelated reader behavior.

tests/support/normalized_oracle.rs is independent of production Ondas decoding. Reuse changes, sample_assertions, WindowEvidence and validate_overlaps. Preserve ordered same-tick source observations in each complete window; normalize final states only in this test-side derivation. Derive projected expected bit strings by ordinary string positions before normalization, not production Signal/Bits slicing. A raw changed_at outside complete coverage is not a normalized timestamp assertion. Version-1 NaN equivalence cannot prove payload identity; retain literal-byte tests for the stronger real contract. Do not author expected values from Ondas or silently replace raw sidecar records with the new backend's normalized output.


### Concrete workload and measurement patterns


Use benches/vcd.rs compact_wide, swerv and composed, benches/fst/hotpaths.rs topology, sparse_dense and wide, and benches/fst/composed.rs wide as concrete templates. Their useful questions are shared traversal versus individual points, traversal length versus output width, stable projection versus active base, distinct histories versus duplicates, and caller payload copies versus decoder work. FST section-boundary and whole-file-wrapper workloads have no direct GHW equivalent. For GHW replace them with SNP/CYC transitions, equal timestamps across sections and clean/partial footer boundaries.

Reuse the pure conditional_query, conditional_scan, first_high and adjacent_query consumers currently in tests/support/fst_workloads.rs. Once GHW uses them, move the functions without behavior changes to tests/support/query_workloads.rs, update the FST bench module path and conformance/workload-check imports, and use the same functions from GHW. conditional_scan retains only current values and supplies a bounded sequential comparison; it is still the same backend and cannot establish independent decoder correctness. Conformance must check the exact timed consumer with literal expectations and independent oracle windows. Keep vcd_workloads.rs temporal/conditional as references for predecessor reads and sparse acceptance; its source-specific paths and counter-mask condition are not common infrastructure.

Model one controlled GHW source on the established 4096-bit toggle workload: top.wide starts at zero, only its low bit alternates at ticks 1..4096, and top.control matches that bit. Generate changes at 1 fs intervals with default producer resolution and require the reference probe to confirm raw ticks 1..4096. This matters because the unchanged shared consumer uses absolute raw time modulo stride. The whole vector and low projection have 4096 post-initial transitions; the high projection has none. A stride-64 consumer accepts ticks 1,65,...,4033: 64 accepted payloads. Compare a separate scalar driver with a low-bit driver sharing the payload base and lazy borrowed payload with eager owned rejected payload. Range 2..=64 accepts no results; range 2..=129 accepts ticks 65 and 129, and a limit-one query stops at 65 after earlier rejected candidates. Tests compare values and counts, not exact backend candidate supersets. Keep physical timing/provenance explicit; these are controlled decoder workloads, not a physical-performance model.

For point reuse, use the same raw-tick patterns as fst/composed.rs: repeated [4094,4094,4094], forward [4093,4094,4095], backward [4095,4094,4093], and backstep [4094,4095,4094]. During a query compare reads at t-1,t,t-1, with a separate tick-zero case. A borrowed payload consumer reduces caller copies; it does not imply that the fallback stopped decoding or retaining selected payload state.

Create a short/long history pair with identical early observations and a longer real recorded prefix, a topology group with identical sparse/constant probes but more unrelated scalar identities or denser unrelated updates, and a 64-scalar sparse/dense group with unique selections 1,16,64. Compare duplicates of whole vectors as well as one-bit projections, and overlap recipes. Retain concrete fixture names, source paths, time conversions and settings in ghw_workloads.rs/benches/ghw.rs after milestone 1 verifies them; do not copy FST fixture paths or pretend a large end timestamp is dense activity.

Use a dedicated selection per benchmark case. Prime only a warm-repeat case outside timing. A fresh-selection case constructs, queries and drops the selection inside each iteration, excluding open and path resolution; an end-to-end case also includes those operations explicitly. A fresh point series allows reuse within its iteration but constructs/drops its selection inside timing, so it cannot inherit prior-iteration cached state. A warm series reuses one primed selection and is labeled separately. For first/second lookup, follow fst/hotpaths.rs iter_batched_ref with BatchSize::PerIteration: open in setup and do exactly one untimed lookup only for the second-index-build case. Input bytes are loaded once; timed bytes opening includes Arc clone and waveform drop. Report series throughput in actual queries, not vector width or candidate count. Success preflight stays outside timing and does not repeat the oracle.


## Format Sources and Executable Specification


### Pinned sources and how to read them


The primary format sources are GHDL commit 2c451de76b07b73fd338c91d68fd5cefcb152512: [writer src/grt/grt-waves.adb](https://github.com/ghdl/ghdl/blob/2c451de76b07b73fd338c91d68fd5cefcb152512/src/grt/grt-waves.adb), [reader ghw/libghw.c](https://github.com/ghdl/ghdl/blob/2c451de76b07b73fd338c91d68fd5cefcb152512/ghw/libghw.c), and [tags/types ghw/libghw.h](https://github.com/ghdl/ghdl/blob/2c451de76b07b73fd338c91d68fd5cefcb152512/ghw/libghw.h). The [official GHW documentation](https://ghdl.github.io/ghdl/ghw/index.html) describes their relationship and the ghwdump debugging tool.

Read Write_File_Header and Wave_Write_Size_Order for the header; Write_Strings_Compress for the string pool; Write_Types, Write_Range, and Write_Composite_Bounds for types; Write_Hierarchy_El and Wave_Put_Hierarchy_1 for hierarchy; Write_Signal_Value, Wave_Cycle, and Wave_Write_Directory for values and sections. Cross-check ghw_read_str, ghw_read_type, ghw_read_type_bounds, ghw_read_hie, ghw_read_snapshot, and ghw_read_cycle_* in the reader. If the two disagree, construct a diagnostic fixture, inspect producer bytes, record the resolution here, and add a regression before implementing an exception. Do not infer an alternative wire grammar from a single crashing file.

The comparison source is [wellen GHW at commit 34f40595c4330bcac364397679cdc3d5ae6b5c0c](https://github.com/ekiwi/wellen/tree/34f40595c4330bcac364397679cdc3d5ae6b5c0c/wellen/src/ghw). hierarchy.rs parses and maps types, common.rs defines tags and scalar/vector tracking, signals.rs assembles values, and mod.rs separates header/body loading. read_body builds its compressed waveform memory and time table before later selected loading. Inspect read_subtype_bounds, add_var, register_bit_vec, and process handling; do not inherit their assumptions. Its streaming API rejects GHW, whereas Ondas needs its own private chronological reader.

The initial fixture repository is aadb6d00597265fdef1a8227825465dbffecee43. Its [schema](https://github.com/kleverhq/ondas-fixtures/blob/aadb6d00597265fdef1a8227825465dbffecee43/schemas/fixture.schema.json) remains the only sidecar schema. Accumulate evidence-bearing local commits on feat/ghw-oracles and record the exact tested SHA here and in the parent Git entry when selected. A branch is the development location; benchmark/conformance evidence names a specific commit. Do not push or release it in this task.


### Header, sections, integer encodings, and names


A GHW header has 16 bytes. Bytes 0..8 contain GHDLwave followed by newline; byte 9 is 16; byte 10 is major version 0; byte 11 is minor version 0 or 1; byte 12 is byte order 1 for little endian or 2 for big endian; bytes 13 and 14 describe the producer word and offset sizes; byte 15 is zero. The pinned writer emits word-size 4 and offset-size descriptor 1. These descriptors do not turn the fixed 32-bit directory fields into arbitrary-width integers. Decode the specified fixed fields independently, verify reserved bytes, and return a clear unsupported-layout error for a descriptor combination whose meaning is not established.

Section tags are four bytes: three ASCII letters and a zero byte. A normal header consists of STR, TYP, optional WKT, HIE, EOH. STR closes with EOS; TYP ends with a zero byte; HIE's entry stream has its own zero terminator before EOH. The body contains SNP snapshots and CYC cycles, followed optionally by DIR/EOD and TAI. Several snapshots or cycle sections are permitted. There is no universal section length, so an unknown section cannot be safely skipped.

Fixed counts and offsets use encoded-endian 32-bit fields; absolute body times use encoded-endian signed 64-bit fields. Reject negative counts and negative times. Use u64 for actual stream positions and checked conversions to allocation sizes. ULEB128 means an unsigned integer carried in seven-bit groups, low group first, with bit 7 indicating continuation. Use bounded 32-bit reads for source IDs and counts. SLEB128 carries signed values in the same groups, with sign extension from bit 6 of the terminating byte; use bounded 32- or 64-bit reads according to the storage kind. Reject truncation, excess groups, and invalid high bits without wrapping or panicking. Do not require minimal encodings unless producer evidence establishes that rule.

STR has four zero bytes, a string count, and an advertised decoded-string size after its tag. String ID 0 denotes an anonymous/absent type name. For each stored string, copy the prefix of the previous string selected by the preceding prefix count, then read suffix bytes until a byte in 0..31 or 128..159. That byte starts the shared-prefix count for the next string. The prefix count uses five-bit groups, with the high bit marking continuation; it is not ordinary ULEB128. Bound both the prefix and reconstructed storage against valid previous strings and checked advertised limits.

Keep raw pool bytes until conversion. Public names must not be produced with lossy replacement. Milestone 1 must establish the pinned producer's encoding for non-ASCII names/literals using a diagnostic source and byte inspection; adopt one lossless documented conversion before merging the hierarchy mapper. ASCII is unambiguous. Standard character-enum labels can include non-ASCII bytes, even when the variable names are ASCII. If a source byte string has no established lossless conversion to the public UTF-8 representation, return Error::Backend with an explicit encoding limitation rather than silently changing identity.

Do not apply the Verilog normalize_name helper to VHDL strings. Preserve case, punctuation, brackets, whitespace, and source spelling as exact hierarchy components. A display escape is not extra hierarchy structure. Handle actual VHDL extended-identifier delimiters only after a source test proves how the producer stores them; reader_name can retain the original spelling. Do not split source names containing a dot or brackets.


### Type grammar and constrained layouts


RTIK is GHDL's numeric runtime type-kind tag. Use the serialized tags from the pinned header, not an Ada runtime enum from another version. The supported base tags are b2=22, e8=23, i32=25, i64=26, f64=27, p32=28, p64=29, array=31, and record=32. The supported subtype tags are scalar=34, constrained array=35, unbounded array alias=37, constrained record=38, and unbounded record alias=39. E32=24 is listed in the header but is not emitted by the pinned type writer or decoded by its type reader; do not guess its payload. Access, file, protected, and obsolete tags are likewise not promises of implemented wire storage.

TYP carries four zero bytes, a fixed type count, that many entries in reference order, then a zero terminator. String/type IDs inside entries are ULEB128. Type IDs are one-based references to already declared entries. Reject zero type references, unresolved references, and contradictory definitions. Anonymous constrained layouts are real type information, not a reason to discard the declaration.

For b2/e8, an entry carries a name ID, literal count, and literal string IDs. i32/i64/f64 carry a name ID. p32/p64 carry a name ID; version 1 then carries a unit count and pairs of unit-name ID and signed integer multiplier. Version 0 has no unit list. Both physical storage kinds are signed integer values, not binary64 reals.

A scalar subtype carries a name ID, base type ID, and a range. A range begins with its base kind in the low seven bits and its direction in bit 7: zero means to, one means downto. Endpoint payloads use byte values for b2/e8, signed LEB128 for integer/physical kinds, and encoded-endian binary64 for real kinds. Keep subtype constraints independently of runtime bit width. Do not turn integer value bounds into BitRange indices or renumber an enum subtype's ordinal values.

An array base entry carries a name ID, element type ID, dimension count, and one index type ID per dimension. A constrained array entry carries a name ID and base type ID, then one range per base dimension. If the base element is unbounded, the stream immediately continues with that element's composite bounds recursively. The number of bounds is determined by the type graph, not a terminator or a guessed fixed byte count. This distinction is central to wellen issue 102.

A record base entry carries a name ID, field count, and field-name/type-ID pairs in declaration order. A constrained record entry carries a name ID and base record ID. It then carries extra bounds only for fields whose base layout is unbounded, recursively, in field order. A bounded record subtype has no extra bounds. Tags 37 and 39 carry a name ID and base ID; they preserve an unbounded alias that can be constrained later. Resolve the effective base shape without discarding the declared subtype name.

Store private raw type descriptions and resolved scalar layouts. A scalar layout is the ordered list of scalar storage forms reached by expanding an object's constrained type. Walk array dimensions in source direction, left endpoint to right endpoint, with the last dimension varying fastest. Walk records in field order. A to range with left greater than right, or a downto range with left less than right, has zero elements. Use checked arithmetic for dimension products and scalar counts; do not take an absolute difference to make a null range nonempty. Use an explicit work stack where input-controlled nesting would otherwise exhaust the Rust call stack.

WKT identifies Boolean, Bit, and std_ulogic base enums. Validate these markers against the enum storage/literals, then propagate their known logic domain through subtypes. Do not infer a nine-state domain from a convenient type-name string or from observed 0/1 values. A ordinary custom enum retains its original ordinal encoding and labels even if its label names resemble logic states.


### Hierarchy and identity


HIE has four zero bytes and fixed scope, declaration, and scalar-table counts. The scalar table uses IDs starting at 1; 0 is reserved. Keep the maximum source-ID boundary and each established scalar storage kind. Filtered producer scope counts can overstate what is emitted; the actual entry stream and balanced stack define the hierarchy, not a strict equality assertion on the advertised scope count.

Hierarchy entries carry a tag and a name ID. Block, generate-if, generate-for, instance, generic, and package entries open scopes; end-scope closes one; the zero entry terminates the stream. A process entry is a leaf scope and does not push another nesting level. Generate-for also carries an iterator type ID and scalar value. Include that source integer or enum literal in the generated scope component, such as gen(0), so distinct iterations do not merge. There is no public generate-iterator metadata field to add.

Signal/port entries additionally carry a constrained type ID and exactly the scalar IDs required by that layout. They are an explicit ordered list: retain it before converting the declaration. Respect input, output, inout, buffer, and linkage tags; ordinary signals have implicit direction. Preserve all declarations during traversal, even if another declaration already uses their scalar IDs. Generic scopes do not establish the existence of generic-value records. The pinned producer generally emits ports/signals and process scopes, not source constants or generic values.

A signal recipe is a private description of how to build one public whole value from source scalars. Use an ordered scalar-ID list plus its public encoding as its identity key. Scalar and vector aliases with the exact same whole recipe share one Signal index and retain separate metadata. Subsets, supersets, reversed orders, and partially overlapping vectors have distinct whole recipes and decode correctly; they need no changes to Signal equality, slice identity, or VariableData. A source scalar can update several selected recipes.

Prefer one-dimensional integer-indexed Bit/std_ulogic arrays as Bits of the full constrained width. Store their source endpoints in BitRange and put their leftmost source element at the leftmost public value position; normalized position zero remains the rightmost bit. Preserve the distinction between array direction and scalar-ID numerical order. Records and other arrays are record/array scopes. Nested dimensions use nested exact index components such as [2] then [-1]; enum-indexed arrays use the source enum labels as components, not fabricated integer indices. Multidimensional logic arrays remain scopes with scalar leaves; do not pretend they are one flat source vector.

For a null logic array keep the variable, its type name, and signal=None. Its range is None because BitRange's absolute-endpoint width cannot represent a directional null interval correctly. Keep its real bounds in the private layout. A null composite array is an empty array scope. Do not drop either object, invent a zero-width value, or classify it as an unsupported history.

Use common scope kinds block, for-generate, if-generate, instance, package, process, record, and array where applicable. Use ghw:generic for the format-specific generic scope. Synthetic record/array scopes have Packing::Unpacked; ordinary scopes have no packing information. Do not add a synthetic design path component that the source did not provide. Definition names, writer/date/comments, and hidden status remain absent/default unless the file actually establishes them.


### Snapshots, cycles, time, and errors


A SNP tag is followed by four zero bytes, an absolute signed 64-bit time, and one scalar value per active scalar ID in increasing ID order, then ESN. b2/e8 values are bytes; i32/p32 use signed 32-bit LEB128; i64/p64 use signed 64-bit LEB128; f64 is eight IEEE binary64 bytes in the header byte order. Validate enum codes and storage sizes. Keep f64 payloads as bits until conversion with f64::from_bits; the reference C reader has a byte-order FIXME and is not a trustworthy cross-endian real oracle.

A CYC section begins with an absolute signed 64-bit time, then changed scalar records. Each record has a positive ULEB128 delta and a scalar payload. Zero ends the change list. The delta advances an active-scalar rank, skipping holes in the source-ID table, not a declaration or public signal index. Reset that rank at the beginning of every change list. After a list, signed 64-bit LEB128 gives the next time increment: zero is another update at the same tick; -1 ends the section and must be followed by ECY. Other negative increments are malformed. Check addition and global nondecreasing times across snapshots/cycle sections.

DIR contains a size/order descriptor, fixed entry count, tag/32-bit-offset pairs, and EOD; TAI contains a size/order descriptor and the directory offset. Decode their structure without trusting offsets to skip body validation. They are not a per-signal or per-time index. Keep physical stream positions in u64 even though those recorded fields are 32-bit. An unknown footer layout is a clear unsupported-layout error, not a reason to reinterpret arbitrary bytes.

Accept clean EOF after EOH with no body as an empty recording, or after a completely closed body section without the optional directory/tailer. The initial snapshot remains absent if no values were recorded. Reject EOF in a marker, fixed field, varint, scalar payload, change list, or unclosed cycle. Optional footer absence must not make a partially written scalar acceptable.

Metadata retains raw first/last recorded ticks and uses the conventional GHW Timescale(1, Femtosecond). Do not rescale the source or claim to recover a producer's nondefault resolution. Unknown simulator version, date, and comments remain absent. A GHW format minor version is not a simulator version.

Map bad markers, invalid references, incompatible scalar storage, truncation, overflow, and backward time to Error::Malformed { format: Format::Ghw, backend: "ghw-native", message }. Include a byte offset and section/type context. Preserve real operating-system errors as Error::Io. Use Error::Backend with operation "open" and an explicit unsupported-version/tag/layout/encoding message when the format is recognized but decoding knowledge is missing. If a known storage shape is decoded but its public value class is unrepresentable, preserve that leaf with Encoding::Unsupported and let selection validation return UnsupportedSignal. Do not claim an unknown, lengthless wire tag can be preserved while skipping its payload.

There is no GHW event-occurrence wire kind in this supported grammar. Ordinary value updates are persistent observations. Never create Event values from changes or process declarations.


## Compatibility and Completeness


### Issue-driven acceptance


[Wellen 6](https://github.com/ekiwi/wellen/issues/6), closed: separate generate-for iterations rather than merge repeated names. Use ghw0006-wellen-issue-6 and a diagnostic integer/enum iterator case. All expected instance paths must be distinct and retain their values.

[Wellen 12](https://github.com/ekiwi/wellen/issues/12), closed: a three-bit state had silent value corruption. Use ghw0001-wellen-ghw-bug, resolve test_rom_tb.soc_inst.core_inst.state, and record independent complete windows around reset and later transitions. Its hole in the scalar table must not corrupt the middle bit. Do not treat wellen's historical incorrect printout as the expected history.

[Wellen 32](https://github.com/ekiwi/wellen/issues/32), [34](https://github.com/ekiwi/wellen/issues/34), and [35](https://github.com/ekiwi/wellen/issues/35), closed: plain record aliases, constrained records with unbounded vector fields, and nested unconstrained records/arrays. Use ghw0003-wellen-issue-32, ghw0004-wellen-issue-34, and ghw0005-wellen-issue-35. Prove named subtypes, field widths/order, and actual field values, not just lack of panic.

[Wellen 33](https://github.com/ekiwi/wellen/issues/33), closed: Python traversal omitted alias declarations. Ondas already separates variables from signals; add a directed GHW traversal test with several declarations sharing a recipe. Both variables() and recursive roots/children traversal must visit them, while signals() deduplicates whole histories.

[Wellen 53](https://github.com/ekiwi/wellen/issues/53) and [GHDL 2970](https://github.com/ghdl/ghdl/issues/2970), closed: descending arrays of vectors were mapped in the wrong order. Use ghw0000-bcd-counter-debug. Assert corresponding outer-array members and dec0/dec1/dec2 leaf histories against independently observed values, and repeat with an ascending/negative-bound diagnostic array.

[Wellen 84](https://github.com/ekiwi/wellen/issues/84), closed: physical32, mixed types, arrays of records, character arrays, and units. The pinned source still lacks TypeP32 handling. Add independently generated p32/p64 and i64 cases with positive/negative values and subtype limits. Preserve physical numeric storage and type names; physical units stay private. Character arrays must remain readable enum leaves with exact labels/ordinal values, not be dropped or converted through an assumed string encoding.

[Wellen 90](https://github.com/ekiwi/wellen/issues/90), open: a true multidimensional array. Import its vhdlarrays.zip source/artifacts into the fixture workflow, validate provenance/hash, and author an independent oracle. Check both dimensions, mixed ascending/descending order, enum indices, null dimensions, and distinct element values. A nested one-dimensional array alone does not prove multidimensional grammar support.

[Wellen 99](https://github.com/ekiwi/wellen/issues/99), open, links [Surfer 547](https://gitlab.com/surfer-project/surfer/-/work_items/547): addr_c should be 00001 at 150 ns and remain so for 2 ns, while Surfer displays 00000. Import the original dump.ghw attachment with provenance and a checksum. Resolve its exact declaration from hierarchy/scalar IDs rather than assuming a unique basename. Establish the complete relevant window independently with the canonical reader and ordered scalar IDs. The regression must check the point, neighbors, trace, batch, projection, and candidate/composed query behavior at the corresponding raw ticks.

[Wellen 102](https://github.com/ekiwi/wellen/issues/102), open, links [Surfer 552](https://gitlab.com/surfer-project/surfer/-/work_items/552): an array of unbounded std_ulogic_vector fails where the std_logic_vector counterpart works. Create paired VHDL-2008 declarations of an integer-indexed outer array constrained to 0 to 0 and an inner vector constrained to 0 downto 0, initialized to zero and then changed to one. Both must yield the same readable one-bit leaf history. Verify consumption of nested bounds and the following type entry; do not patch around “three missing bytes” without grammar evidence.

[Wellen 146](https://github.com/ekiwi/wellen/issues/146), open: generated gate-instance outputs connected to a descending vector have swapped values. Create an independently authored reproducer with a(1 downto 0)="10", b="11", y=a and b, and two ascending generate iterations i=0..1 connecting a(i), b(i), y(i). Change a to "01" at 10 ns. After settling, at 1 ns the whole y is 10, iteration 0 y is 0, and iteration 1 y is 1. At 11 ns the whole y is 01, iteration 0 y is 1, and iteration 1 y is 0. Compare each declaration, the corresponding normalized vector slice, duplicates, and a composed query. Both traversal order and scalar-ID order must leave these answers unchanged.

[GHDL 538](https://github.com/ghdl/ghdl/issues/538), closed: older unconstrained-array recordings could be incomplete or incompatible. Classify ghw0002-ghdl-issue-538 from its bytes and producer grammar. Either document and test a specific valid historical grammar, or assert Malformed/explicit unsupported-layout with evidence. Do not make the release claim depend on accepting an unrecoverable recording.

[GHDL 1769](https://github.com/ghdl/ghdl/issues/1769), open: missing nondefault time resolution is a producer/format limit. A test must prove no speculative scaling. [GHDL 2548](https://github.com/ghdl/ghdl/issues/2548), closed, documents expensive scalar-hole traversal in libghw; use the active-rank map rather than repeated linear searches through holes. [Wellen 47](https://github.com/ekiwi/wellen/issues/47), closed, includes viewer parameter-loading costs as well as FST problems; it is motivation to measure the actual Ondas boundary, not evidence that every GHW parse itself takes 30 seconds.


### Coverage beyond the issued reproducers


In addition to the cases above, acceptance includes real64 with signed zero/infinities and authored payload-preserving NaN tests; constrained enum subsets with unchanged original ordinals; unbounded record aliases; one-/two-/three-dimensional arrays; arrays of records and records of arrays; empty arrays; integer, enum, and negative indices; filtered hierarchy; processes; all port directions; exact escaped/path components; vectors with nonconsecutive, reversed, subset, superset, partial-overlap, and scalar-first alias recipes; and both byte orders/format versions for the known grammar. Validate storage through the source specification even when a synthetic edge case has no real producer example.

This establishes a concrete superset of the pinned wellen GHW paths: multidimensional layouts, p32, queryable i64, unbounded record aliases, proper enum constraints, unrestricted ordered alias recipes, retained null declarations/processes, endian-correct real decoding, and Ondas streaming/query semantics. Each claimed addition needs a named passing test and an independent expected result. Historical fixes that wellen already has are baseline regressions, not evidence of superiority.


## Plan of Work


### Milestone 1 — Independent evidence and diagnostic prototypes


At this milestone's end, the implementation has a trustworthy input/expected-output set and known benchmark workloads, before production parsing begins. Inspect the initial 93 metadata entries and explicitly install the needed GHW recordings. Compile pinned ghwdump/libghw into ignored tmp/ as a development oracle, using a separate process per artifact because malformed input can terminate it. Add a small disposable probe that emits raw scalar IDs, types, times, and lossless f64 bit patterns; do not parse human-formatted %g reals as exact expected values.

Reproduce the four earlier scan failures and distinguish incomplete bodies from valid producer constructs. Inspect the named existing issue fixtures. Acquire the original issues 90/99 artifacts and paired source reproducers for issues 102/146. Record expected histories from source behavior plus the canonical reader; for wire-only cases use independently authored literal bytes. Inspect non-ASCII pool bytes, extended identifiers, generate iterator spelling, and endian behavior here. Resolve the name/literal conversion decision before milestone 3.

The corpus currently has no GHW oracles, so it cannot supply a passing GHW pool. Implement the independent GHW extraction/generation path and reviewed recipes in the neighboring factory first, commit its tracked changes locally, and generate nonempty sparse oracles with complete small windows. Add authored sources for the newer issues and primitive/type gaps there. Verify/repeat factory outputs, explicitly copy them into the fixture branch, and validate them using the fixture repository tools. Preserve original artifact hashes for unmodified recordings. If the fixture schema supports a desired assertion that the current Ondas runner lacks, either implement that exact assertion in milestone 5 or keep the oracle in the executable profile and test the metadata separately.

Accumulate local factory and fixture commits as described above. Local acceptance requires verified observations and matching local payloads, not publication. Do not push, tag or release either repository. Never edit expected observations to match Ondas output, treat {} as success, or silently substitute a different provider. Final generation/verify/repeat must bind the committed post-import fixture state; later delivery belongs to the user's separate release work.

Use a GHDL producer only in the factory's fixture-generation environment, never as a library or normal-test dependency. Keep authored VHDL and exact producer revision/options in the factory's tracked reproduction material, with portable schema-supported provenance in fixture sidecars. Build the pinned producer's mcode backend with GNAT in its public GHW generation image. Copy sources to neutral /tmp/work.* paths inside the container before analysis/simulation; no repository/home path may become a producer path. Use --std=08 and default resolution, finish all observed states cleanly, and verify actual raw ticks. The native oracle reader still runs through the factory's independent extraction path. Library scratch probes may inspect bytes, but do not generate the delivered oracle objects.

Create the proposed workload definitions in tests/support/ghw_workloads.rs once paths/ticks are verified. Start with ghw0000 for compact complete histories, ghw0023-keyboardwave and ghw0030-vga-controllerwave for sustained activity, and ghw0090-ws2815b-driver for approximately 7.28 MB and 65722 source scalars. Add controlled external recordings for long/short history, sparse/dense selected activity, scalar-ID holes, alias-heavy hierarchy, and a wide vector with a changing low bit and stable high slice. Never use the three incomplete large recordings as successful full-open/scan workloads.

Use the VCD/FST workload definitions above to author the controls before tuning production code. The wide source must have the confirmed 4096-transition/raw-tick schedule; the topology controls must keep selected sparse/constant histories identical while changing unrelated identities or updates. Record source-derived accepted rows, not just benchmark counts. Select concrete fixture IDs only when the matching payload, provenance and independent oracle are available. Keep the existing VCD/FST artifacts as separate regression inputs; cross-format equality of file sizes, physical duration or scan timings is not required.

Prove this milestone by validated hashes/schema, exact raw/normalized windows for the named issues, and source-byte probes. New GHW production tests still fail because the backend is absent. Keep probe logs and downloaded upstream sources ignored; embed only compact conclusions and provenance in this plan and eventual authoritative docs.


### Milestone 2 — Wire decoder and supported type layouts


Add src/backends/ghw/mod.rs, wire.rs, and types.rs. Keep module boundaries tied to actual responsibilities, not one file per tag or one generic trait per parsing concept. wire.rs owns checked byte access, stream offsets, encoded endian integers/reals, bounded LEB128, section markers, and string-pool reconstruction. types.rs owns source type definitions, ranges, recursive bound consumption, scalar storage forms, and checked layout expansion.

Add test-only diagnostic byte builders in src/backends/ghw/tests.rs or a narrow test submodule. These are not a production writer and must not reuse decoding helpers to create expected bytes. Use literal inputs for LEB overflow/truncation, f64 endian patterns, and section boundaries. Follow every supported type branch through both named and anonymous constraints, including tags 37/39 and multidimensional element bounds. Test input-controlled depth without a stack overflow.

Adapt VCD's buffer-boundary regression to BufReader capacities 1 and 2 over Cursor input. Make tags, fixed integers/reals, multibyte LEB values and string-prefix continuation cross those boundaries. Opening and selected reads must preserve offsets and values. Reuse scratch buffers; at fixed maximum layout/value size, extending history must not cause record-count-dependent buffer growth. Decode unselected fixed scalar payloads into fixed local storage without building public values, while validating their source codes and constraints.

For malformed cases, assert Error category/context rather than an entire incidental message. For unsupported future version or unknown wire tags, assert the explicit limitation and absence of panic. Validate nonempty/empty layouts, scalar counts, and exact next-section position independently. Prove that unknown lengthless forms never get “skipped” as if they were an Unsupported signal.

Run the filtered library GHW tests, then the current self-contained tests. At completion, the private decoder can explain the header, types, and scalar storage of all classified supported artifacts, but public opening is not enabled yet. Source shape/type tests distinguish real-corpus evidence from manually encoded edge cases.


### Milestone 3 — Hierarchy mapping and signal recipes


Add src/backends/ghw/hierarchy.rs. Translate source entries into existing ScopeData and VariableData. Preserve declaration names, available named types and enum labels/keys, directions, real integer-indexed bit ranges, and all readable leaves. Resolve finite array/record layouts with the grammar in this plan. Implement exact alias-key interning after preserving complete ordered ID lists; this requires ordinary maps/vectors, not a backend-independent database or public type object.

Use signed 32-/64-bit two's-complement Bits for i32/i64/p32/p64, independent of subtype value bounds. These tags establish signed numeric storage; preserve Signedness::Signed where that interpretation is source-supported. Real and ordinary enums have no signedness. Named signed/unsigned vector aliases lack an authoritative signedness marker in GHW, so their signedness stays None unless a source-based identifier exists; never infer it from the type-name string. WKT Bit/Boolean have TwoState and std_ulogic has NineState; propagate to relevant logic vectors. Ordinary numeric/enum encodings need not claim a source logic domain.

For ordinary enum values, use the minimum positive width that represents the base literal ordinals. Encoded Enumeration keys are fixed-width binary strings matching those ordinals. An enum subtype exposes its allowed labels in base declaration order without renumbering and without changing its shared runtime width. Type-name selection prefers the named declaration/subtype and falls back through anonymous constraints to an available named base.

Directed tests must include arbitrary source-ID order, vector/scalar declaration-order permutations, exact whole aliases with different declaration metadata, overlap recipes, enum-indexed arrays, null objects, and unselected unrelated declarations. Do not add a public representation for empty vector bounds or physical units to make these tests easier.

Run GHW hierarchy/type tests through the private mapping and existing Hierarchy methods. At completion, traversal visits every expected source declaration, lookup retains exact components, identical whole recipes compare equal, and unsupported/no-history declarations remain distinct. The mapper cannot silently substitute observed-value equality for source identity.


### Milestone 4 — Streaming body and public API integration


Add src/backends/ghw/body.rs and complete ghw::Reader. Use one body state machine for opening validation and queries. It must traverse snapshots/cycles without accumulating value histories or a global timestamp table. Opening walks the body once to validate complete records, establish first/last ticks, and retain its start position. read_metadata uses regular GHW opening and drops the reader; it must not advertise an inexpensive header-only time-span path.

For a query, prepare the distinct selected whole recipes and their scalar dependency set. A scalar dependency is a source scalar whose value contributes to a selected public signal. Keep a reverse map from those source IDs to selected recipe/value positions. Decode every record's length and necessary validity checks, but do not allocate runtime values for unrelated histories. Allocate/reuse current selected vector buffers. Mark changed selected recipes, then emit each affected assembled whole value once per source change list rather than copy a wide vector for every scalar bit. Preserve later change lists at the same tick and let the shared query engine determine net final-tick observations. A snapshot establishes even an all-unknown value as recorded state.

Use a touched-recipe list or equivalent bounded marks to deliver only affected recipes. Preserve the common engine's active_slots comparison/commit loops and projection-before-comparison behavior. Do not scan or copy all selected vectors for every scalar record, and do not add per-duplicate caller state in the decoder: Selection's existing base/group/slot mapping owns that distinction. Malformed unselected records still fail full opening as in VCD; FST's reported late failures are not permission to weaken GHW opening validation.

Make the input read lazy after opening: query read must stop at the requested end or callback break and return its break payload. Never collect the rest of the recording before the first callback. Early stop may require completing the current tick to satisfy normalization; account for that work. Do not claim that selecting one signal avoids reading the other encoded records or that a narrow late range avoids establishing its prefix.

Give the body reader a private callback that distinguishes an assembled Value from a completed Tick. Use a GHW-specific arm in Selection::read_ticks_from to consume values and call the existing complete_tick at physical tick boundaries, including ticks with no affected selected recipe. Factor only the local value-consumption/completion logic needed for this arm; do not change every backend's protocol or emit fabricated unchanged values. Complete a tick only after a greater timestamp is known, a bounded read ends after all records at that tick, or clean EOF. Equal timestamps can span SNP/CYC sections, so ECY alone is not completion. Visit before Slot commit to preserve current/preceding-tick context; clear pending_time after explicit completion to prevent duplicate completion on the next value or EOF. Propagate a callback Break before consuming the next tick's values.

Update src/backends/mod.rs to dispatch Reader::Ghw. Update src/waveform.rs check_backend, opening match arms, read_metadata, Waveform::format, and Waveform::backend. Automatic GHW selection and explicit ghw-native both use the native reader. No temporary files are created for Arc<[u8]> input. Wrong-format explicit opening returns BackendDoesNotSupport; unknown backend validation still precedes file I/O. Update the GHW branches of existing format-detection tests from NoBackend to the correct open/malformed behavior.

Add tests/ghw_native.rs with a self-contained small diagnostic recording. It should include a nine-state vector, signed numeric/enum/real values, aliases, same-tick changes that cancel, delayed first state, and exact path components. Exercise open/open_with and both byte-opening functions, metadata, lookup, whole/sliced samples, prepared/one-shot batches, traces/scans, candidates, driver/readable subsets, and current/preceding-tick context reads. Preserve duplicate input slots and changed_at for a stable slice while discarded bits change.

Test late read errors by an internal failing/changed source, callback break, fresh queries after failure, repeated selection use, and independent waveform instances. Keep errors from unrelated unsupported public value classes from breaking valid lookups, while malformed wire records still cannot be safely skipped. Send/Sync assertions must continue to compile.

Add a decisive early-stop regression: one selected update at an initial tick followed by a long suffix that changes only unrelated signals. Breaking at the first completed observation must not decode that suffix. Also test equal timestamps across snapshot/cycle section boundaries and a read error before a physical tick is complete; neither may publish an unfinished state.

Apply the concrete VCD reuse schedules and shared query tests identified above. Compare all results with independently supplied expected histories, then also compare fresh and reused selections. Check repeated/forward/backward/backstep points, stable projections, aliases, missing selected state, EOF, scan/query Break, fallible callbacks and callback unwinding. Saved state remains provisional until successful completion; fresh queries after stop/error must work. Run the complete library and existing query/VCD contract targets after editing engine dispatch, not only GHW-filtered tests. Optional resume read-count assertions belong to milestone 6; initial replay is allowed here.

This milestone is observable through a native open that reports Format::Ghw and backend ghw-native, and public results matching the diagnostic history in file/bytes and prepared/one-shot modes. All pre-existing default tests continue to pass.


### Milestone 5 — Independent conformance and completeness


Use the completed local fixture-branch commit from milestone 1 and verify its installed/copied payloads. Confirm final factory receipts and repeated observations correspond to that post-import commit. Add Format::Ghw parsing, the ghw-native backend name, a logical .ghw filename, and an ignored full_ghw_pool test to tests/conformance.rs. Reuse the existing file/bytes pool machinery and independent normalization; do not add another GHW-specific oracle schema or a configurable test language.

Extend the runner narrowly for enum metadata if the new oracles use it, preserving omission/null semantics and exact encoded keys/labels. Add unsupported encoding/sample-error handling only if an actual accepted GHW fixture needs it. Use tests/support/ghw_workload_checks.rs or focused ignored tests for signedness, logic-domain, complete traversal, alias sets, and issue cases not expressible by version-1 sidecars. Do not change the semantics of existing FST/VCD oracles while adding GHW.

Keep one independent test-side normalization through normalized_oracle. Derive slice windows from raw expected bit strings before final-tick normalization and validate overlap consistency. Reuse sample_queries/window_queries/scan_queries for focused GHW wrappers, duplicate slots and candidate-superset tests; adding only full_ghw_pool is insufficient because some current pool checks are reader-specific. Use the same exact pure query consumers for GHW conformance and later Criterion registration: move the format-independent functions from fst_workloads.rs to query_workloads.rs when this second caller exists, update benches/fst.rs and conformance/fst_workload_checks imports without semantic changes, and add GHW checks for the controlled accepted rows. Borrowed/eager/sequential variants must agree with independent values; same-backend agreement alone is insufficient. Run the existing FST workload checks after the extraction.

Execute every discovered GHW artifact's evidence profile. Empty-oracle fixtures remain explicitly skipped after hash validation. A pool with no executed observations fails. Every failed compatibility-probe artifact needs a justified classification. Use a malformed opening-error sidecar for a confirmed invalid recording. If a valid old layout is unsupported, add a mandatory directed file/bytes test for Error::Backend with its limitation; schema version 1 cannot express that error category, so leave its sidecar oracle empty and report that pool skip separately from the directed negative test. A valid supported recording instead needs value evidence. Do not expand the sidecar schema for this distinction or add a hardcoded whitelist of “files that libghw likes.” The release comparison requires the named issues and new type gaps to have actual independent value evidence, not merely one arbitrary successful GHW oracle.

Run the focused issue regressions and full GHW pool in both file/bytes modes. Report passing observations separately from no-oracle artifacts and authored edge tests. Verify the entire default conformance set remains consistent. At completion, the concrete superset claims in this plan have named passing tests and no unexplained silent value mismatch.


### Milestone 6 — Benchmark execution and bounded streaming state


Add the ghw Criterion target in Cargo.toml and benches/ghw.rs, using the established workloads and shared artifact checks. Add it to just bench-smoke after conformance. Keep workload IDs globally unique, beginning with ghw/<fixture>/<operation>/<mode>/<parameters>, and use explicit ghw-native for principal cases.

Measure open/drop including the body's validation pass; hierarchy traversal and first/second exact lookup; selection construction; individual versus batched points; fresh versus prepared point series; equal-width early/late ranges; early break after the first completed observation; full callback scans; bounded owned traces; whole vectors versus changed/stable slices; overlapping aliases and duplicate selection entries; and candidates/composed queries with a narrow driver and wide readable payload. Keep output materialization/destruction inside its stated timed operation.

Register the concrete patterns from Established VCD and FST Practices: sustained whole/low/stable-high vector and equivalent scalar, sparse/dense unique selections 1/16/64, topology controls, repeat/forward/backward/backstep series, and current/predecessor composed reads. Test conditional stride-64 payload consumers with distinct/shared-base drivers, lazy/eager copies, no result and first acceptance after rejection. Use dedicated primed selections for warm-repeat and fresh selection creation inside timed fresh-series iterations; setup must not accidentally warm a cold case. Use real GHW paths/ticks from ghw_workloads.rs, reuse the exact conformance-checked consumers, and extract bench modules only if the target grows enough to need the fst/hotpaths.rs/composed.rs organization. Do not fill a Cartesian workload matrix or add a benchmark framework.

Compare identical supported end-to-end workloads against wellen 0.25.6 where its public API permits an equivalent consumer. Include its read_header/read_body/load-signals work when comparing open-plus-first-query; do not exclude eager loading to make a false cold comparison. Compare warm repeated queries separately and label retained full-history storage. A direct libghw full-stream probe is decoder throughput context, not an equal Ondas public-API benchmark. Unsupported wellen cases are regression demonstrations, not zero-time benchmark competitors.

Use the controlled history pair to separate an unneeded suffix from late prefix establishment. Use constant/sparse probes with increasing unrelated scalar count and time density. Use a sustained 4096-bit vector, not only a wide value with two writes, to expose whole-vector assembly and projection copying. Source-ID holes must not add a repeated linear search through all source IDs. Increasing declaration aliases at fixed unique source histories must not multiply wire decoding.

Execute these workloads with Criterion's --test mode. Ignore timing output, do not compare performance numbers, and do not tune the reader in this pass. The user's implementation instruction supersedes the earlier measurement/optimization scope. Keep correct setup and ownership boundaries so a later performance pass can use the benchmarks without rewriting their consumers.

The initial query path replays from the beginning. Do not add a checkpoint, continuation, eager history store, or speculative optimization in this implementation. Document this limit. Add test-only counters for records consumed, selected scalar/value storage, and selected-vector deliveries when needed to prove bounded streaming behavior. At fixed topology and selection, extending history must not increase retained decoder/query values. Batching must share one traversal, alias recipes must share source scalar decoding, and callback break must avoid an unused suffix. These deterministic correctness checks do not depend on benchmark numbers.

Promote no timing claims in this pass. Acceptance is successful benchmark registration, fixture preflight, conformance-checked consumers and execution of all selected smoke cases. Tuning and any bounded continuation belong to the user's later separate performance pass.


### Milestone 7 — Documentation and complete gates


Rewrite docs/ghw.md around the native reader, its format sources, type mapping, alias recipes, streaming model, full-open validation cost, and limits. Update src/lib.rs reader-details and src/waveform.rs Rustdoc for default/explicit GHW, file/bytes support, metadata cost, and unsupported forms. Update docs/fixtures.md, docs/api-coverage.md, docs/benchmarking.md, relevant README statements, and justfile recipe descriptions for actual GHW conformance/smoke coverage.

Use concise neutral English and practical Simplified Technical English in permanent docs/Rustdoc. Do not put implementation status lists, research history, machine-local settings, or upstream source clones there. Preserve the source-resolution and null-object representation limitations honestly. A claim of fuller coverage must link to actual tests/fixture evidence; a performance claim must specify its measured boundary.

Run full default CI, the library MSRV check, and doc tests after public documentation changes. Native GHW must not alter feature-enabled FSDB dispatch. If the configured environment supports those existing feature checks, run the appropriate existing recipe; missing proprietary runtime is not a reason to make the native backend depend on it or add local software details to documentation.

Record the final evidence-bearing fixture commit, test names/results, benchmark execution results, any remaining source-format limits, and OCR review resolution. Promote durable conclusions, then remove docs/wip/261011-ghw-native/ before merge into main, retaining docs/wip/AGENTS.md. Do not add cleanup automation. Implementation commits can be incremental and reviewable; keep host hooks installed and the worktree container running before any host git commit.


## Concrete Steps


Unless explicitly marked as factory commands, relative paths below assume the current Ondas worktree root. Git and host hooks run on the host; library/fixture commands use ./dev. Factory host Python/Docker commands use its own checkout and operating contract. These commands distinguish investigation already possible from tests/targets created by later milestones.

Start with:

    git status --short --branch
    ./dev --install-hooks
    ./dev just --list
    git -C fixtures status --short --branch
    git -C fixtures switch feat/ghw-oracles
    git -C fixtures rev-parse HEAD
    ./dev just --justfile fixtures/justfile install ghw/ghw0000-bcd-counter-debug ghw/ghw0001-wellen-ghw-bug ghw/ghw0002-ghdl-issue-538 ghw/ghw0003-wellen-issue-32 ghw/ghw0004-wellen-issue-34 ghw/ghw0005-wellen-issue-35 ghw/ghw0006-wellen-issue-6 ghw/ghw0090-ws2815b-driver

The initial fixture SHA is aadb6d00597265fdef1a8227825465dbffecee43; feat/ghw-oracles was created there and will accumulate local commits. Initialize a missing submodule with git submodule update --init fixtures, then create its missing task branch from that initial SHA. Do not rerun submodule update over the existing task branch: it can detach the checkout at the parent-selected commit. The selected install prepares investigation only; before final default CI run ./dev just fixtures-install against the current committed branch and matching local payloads. Do not use --ignore-missing. If a new worktree copied another checkout's hook path, set only its worktree-local core.hooksPath to its own git-directory/ondas-hooks after reviewing dev and tools/repo/git-hook, then rerun installation. Do not disable or bypass hooks.

Acquire the exact reference sources in ignored tmp/ if they are not already present:

    mkdir -p tmp/ghw-reference
    git clone --filter=blob:none --sparse https://github.com/ghdl/ghdl.git tmp/ghw-reference/ghdl
    git -C tmp/ghw-reference/ghdl sparse-checkout set ghw src/grt
    git -C tmp/ghw-reference/ghdl checkout --detach 2c451de76b07b73fd338c91d68fd5cefcb152512
    ./dev gcc -std=c99 -D_POSIX_C_SOURCE=200809L -O2 -I tmp/ghw-reference/ghdl/ghw tmp/ghw-reference/ghdl/ghw/libghw.c tmp/ghw-reference/ghdl/ghw/ghwdump.c -o tmp/ghw-reference/ghwdump
    ./dev tmp/ghw-reference/ghwdump -h -t fixtures/ghw/ghw0000-bcd-counter-debug/waveform.ghw

Read wellen from the exact published 0.25.6 source obtained by ./dev cargo fetch --locked or a detached clone at the recorded commit. It is a dev-only comparison. Do not change the library MSRV to the reference crate's MSRV. No normal Ondas test should require an externally installed GHDL or ghwdump.

Factory setup is a separate host workflow. Resolve its checkout from the shared repository directory and retain the absolute paths only in local shell variables:

    ghw_library_root=$PWD
    ghw_factory_root=$(realpath "$(git rev-parse --path-format=absolute --git-common-dir)/../../ondas-fixtures-oracle")
    git -C "$ghw_factory_root" status --short --branch
    git -C "$ghw_factory_root" switch feat/ghw-oracles
    ghw_provider_root="$ghw_factory_root/providers/ghw-task"
    mkdir -p "$ghw_provider_root"
    ln -s "$ghw_library_root/fixtures" "$ghw_provider_root/kleverhq.ondas-fixtures"

The two local branches already exist. In another checkout create a missing factory branch from the reviewed baseline e030bbd1f27eade0f10311f46bd197fbe7d1df5c; inspect local changes first and never force/reset another branch. Skip an existing provider link only after confirming that its resolved target is this fixtures checkout. Do not overwrite an existing link. Factory commands below run from ghw_factory_root and must be executed only after milestone 1 adds the named GHW tools.

Dockerfile.ghw must pin GHDL 2c451de76b07b73fd338c91d68fd5cefcb152512 and the public base image, compile the canonical libghw plus readers/ghw.c, and provide the mcode/GNAT producer used by tracked sources/ghw/generate.sh. That script copies sources into neutral /tmp/work.* paths before running ghdl -a/--elab-run --std=08 and --wave; version/options and inspected raw timing go into local provenance. Keep generated outputs in ignored .work-ghw/, add that path to the factory ignore rules, and mount the source/provider read-only. The library no longer needs a temporary producer profile or a direct sibling mount.

From the factory repository, build its public reader tools according to its operating contract:

    docker build --network=host -t ondas-oracle-fst .
    docker build --network=host -f Dockerfile.vcd -t ondas-oracle-vcd .
    docker build --network=host -f Dockerfile.ghw -t ondas-oracle-ghw .

Run full factory discovery before source changes and again afterward, retaining logs under ignored .work-ghw/. Existing tests call the old flat/catalog resolver and default providers; the GHW task does not migrate those formats or repoint their inputs. Record pre-existing failures explicitly, compare the after report, and investigate every new failure. Do not describe a failing full suite as passing or silently skip its execution. Once added, the focused grouped-layout GHW and reused-helper tests must all pass:

    python3 -m unittest discover -s tests -v
    python3 -m unittest discover -s tests -p 'test_ghw*.py' -v

The first command is a before/after baseline check with separately reported pre-existing failures; the second is the GHW delivery gate. Include used sparse-history/hash/receipt helper contracts in test_ghw*.py using self-contained grouped inputs. No legacy layout test or migration is required to make those focused checks pass.

Commit reviewed native adapter/generator/tests and authored recipes locally in feat/ghw-oracles, excluding all generated evidence. Use explicit staged paths and inspect the diff before host git commit. For authored sources, sources/ghw/generate.sh accepts an output root and places each completed case under <output>/<case>/waveform.ghw. Run it from the factory checkout:

    mkdir -p .work-ghw/generated
    docker run --rm --network none --read-only --tmpfs /tmp:rw,nosuid,nodev,exec \
      --mount "type=bind,src=$ghw_factory_root/sources/ghw,dst=/sources,readonly" \
      --mount "type=bind,src=$ghw_factory_root/.work-ghw/generated,dst=/output" \
      ondas-oracle-ghw bash /sources/generate.sh /output

The script uses only copied neutral temporary sources and writes completed outputs atomically. Copy resulting waveform.ghw payloads explicitly into the fixture branch, verify their size/SHA-256, and prepare metadata/provenance before oracle generation. Do not change imported waveform bytes. Commit initial new sidecars locally so provider identity is stable for the first extraction.

The new ghw.py must implement these actions, with no sidecar mutation:

    python3 ghw.py plan --provider-root "$ghw_provider_root" --image ondas-oracle-ghw
    python3 ghw.py generate --provider-root "$ghw_provider_root" --image ondas-oracle-ghw
    python3 ghw.py verify --provider-root "$ghw_provider_root" --image ondas-oracle-ghw
    python3 ghw.py repeat --provider-root "$ghw_provider_root" --image ondas-oracle-ghw

plan may create missing reviewed recipe candidates but must not overwrite existing choices; inspect unsupported/needs-review outcomes. generate/verify/repeat cover the committed reviewed recipes, not unselected signals or unexplained unsupported artifacts. Capture omitted coverage in the report. Add tests for complete-window ordering, missing versus unknown state, native failures, staged-output rollback and post-import receipt invalidation.

After checking recipe/receipt identity, prepare an explicit import for one reviewed bundle. Run these host copies from the library root:

    mkdir -p tmp/ghw-oracle-import/ghw0000-bcd-counter-debug
    cp "$ghw_factory_root/fixtures/kleverhq.ondas-fixtures/ghw0000-bcd-counter-debug/oracle.json" tmp/ghw-oracle-import/ghw0000-bcd-counter-debug/
    cp "$ghw_factory_root/fixtures/kleverhq.ondas-fixtures/ghw0000-bcd-counter-debug/tags.json" tmp/ghw-oracle-import/ghw0000-bcd-counter-debug/
    cp "$ghw_factory_root/fixtures/kleverhq.ondas-fixtures/ghw0000-bcd-counter-debug/receipt.json" tmp/ghw-oracle-import/ghw0000-bcd-counter-debug/

Before changing the destination, verify that receipt.provider/fixture identify this bundle, receipt.artifact equals the destination artifact, and receipt output hashes match the copied oracle/tags bytes. Import only the oracle object and the union of reviewed tags into fixtures/ghw/ghw0000-bcd-counter-debug/fixture.json. Preserve every other field; no waveform bytes or receipts belong in the sidecar. Perform the same explicit reviewed operation for each selected bundle. Keep a verified pre-import raw/oracle/tags snapshot ignored in the factory for the later comparison. Do not create an automatic provider-editing generator.

Once the corresponding modules/tests exist, run:

    ./dev cargo test --locked --lib ghw
    ./dev cargo test --locked --test ghw_native
    ./dev cargo test --locked --lib
    ./dev cargo test --locked --test vcd_native --test query_contract --test query_composition
    ./dev cargo test --locked --test conformance normalized_oracle
    ./dev just check-local

Validate fixture-authoring changes using the fixture repository's existing tools:

    ./dev just --justfile fixtures/justfile setup
    ./dev just --justfile fixtures/justfile validate
    ./dev just --justfile fixtures/justfile check
    ./dev just --justfile fixtures/justfile test

After the explicit import, update README statistics, validate fixture metadata and commit the exact sidecar/license/README changes locally on feat/ghw-oracles. Record its SHA. Return to the factory, regenerate against this final provider commit, compare raw/oracle/tags with the verified pre-import snapshot, and run verify then repeat. Do not copy receipts into the provider or reimport identical outputs. All checks must succeed without changing the final provider. Commit any necessary tracked factory fixes and repeat the evidence pass if source/recipes changed.

Record both local commits and the tested fixture SHA in this plan. When selecting the parent Git entry, stage fixtures only after inspecting git -C fixtures status/log; keep that commit unpushed. Do not commit waveform payloads into either repository. There is no release/tag/push step. Matching locally generated payloads must already be present; the installer verifies them and may retrieve only existing published artifacts. Then run:

    ./dev just fixtures-install
    ./dev cargo test --locked --profile conformance --test conformance full_ghw_pool -- --ignored --nocapture
    ./dev cargo test --locked --profile conformance --test conformance ghw_issue -- --ignored --nocapture
    ./dev cargo test --locked --profile conformance --test conformance ghw_workload_checks -- --ignored --nocapture
    ./dev cargo test --locked --profile conformance --test conformance fst_workload_checks -- --ignored --nocapture
    ./dev just conformance

After benches/ghw.rs and its Cargo target exist:

    ./dev cargo bench --locked --bench ghw -- --test
    ./dev cargo bench --locked --bench ghw -- --save-baseline ghw-native-correct
    ./dev cargo bench --locked --bench ghw -- point-series
    ./dev cargo bench --locked --bench ghw -- wide
    ./dev cargo bench --locked --bench ghw -- --baseline ghw-native-correct

The reference and comparison revisions must retain matching Criterion IDs, workloads, corpus pin, input/selection boundaries, and destruction boundaries. Worktrees do not automatically share Criterion outputs.

Final default gates after implementation and permanent documentation updates:

    ./dev just ci
    ./dev just msrv
    ./dev cargo test --doc --locked
    git diff --check
    git status --short

Expect all gates to exit successfully, every named GHW regression to execute, and full_ghw_pool to report zero failed cases plus separately listed no-oracle skips. An initial all-skipped GHW pool is an expected failure, not a valid release result.


## Validation and Acceptance


Before implementation, automatic GHW open returns NoBackend and explicit ghw-native returns UnknownBackend. After milestone 4, both opening choices succeed on the small valid diagnostic recording and expose Format::Ghw, backend ghw-native, the same hierarchy, and the same metadata in file/bytes mode. A malformed header/body returns Malformed with GHW/backend context; a foreign signal remains InvalidSignal and wrong-format explicit selection remains BackendDoesNotSupport.

A user can resolve an ordinary source vector through HierarchyPath::from_components, take a normalized slice, and run samples/traces/scans through both Waveform and Selection. Aliases, duplicates, same-tick excursions, entering states, query ranges before/after recorded time, and callback breaks retain the existing API meanings. Unsupported storage limits must not appear as invented zero values or empty successful results.

The issue-146 diagnostic supplies a small independently understandable demonstration: y=10 at 1 ns and y=01 at 11 ns, with corresponding generated scalar outputs and vector slices. The issue-99 recording independently proves addr_c=00001 in the specified 150-ns window. The issue-102 pair proves correct nested bound consumption. Issue-90 plus directed type cases proves true multidimensional support. Record/enum/physical/i64/null/process tests prove the remaining named extensions over wellen.

No claim is complete until its expected values, paths, type metadata, and traversal obligations are exercised, with real external evidence for producer-supported constructs and independently authored bytes for decoder edge forms. Same-tick normalization expectations come from complete ordered windows, not from isolated raw changed_at values. Version-1 oracles cannot prove NaN payload identity or source signedness/logic domain; keep the stronger directed tests.

The performance report must include matched cold/open-plus-first-query and warm/prepared boundaries, full traversal, sparse selected work, long-prefix late work, early break, and wide/alias cases. Logical storage/record counters establish the bounded behavior; Criterion measurements establish workload cost. Do not substitute smoke timings, a large numerical end tick, or a parser-open success count for those findings.

The transferred VCD/FST decisions are accepted only when the unchanged shared semantic tests pass, GHW's literal/external expectations pass in file/bytes mode, and the extracted FST consumer checks still pass. GHW raw timing must match the controlled stride schedule, projected expectations must be derived before normalization, and benchmark fresh/warm labels must match actual ownership/setup. Existing VCD parallel paths and FST directory/chain seeking remain format-specific implementation choices.

Fixture acceptance is local: committed factory reader/recipes, generated and byte-identically repeated bundles, explicitly imported committed sidecars, hash-matching ignored payloads, final post-import receipts and local conformance. Push, release tags and uploads are prohibited throughout this task. The user's later separate delivery is outside these implementation gates; report that remote clean-checkout reproduction is pending rather than claim it was performed.


## Idempotence and Recovery


Do not recreate an already existing worktree, reset shared branches, or delete another agent's files. Repeated hook installation and fixture installation are safe after configuration/source review. If a source directory already exists, verify its revision and clean state before using it rather than cloning over it.

Interrupted fixture installation can be repeated; the installer verifies sizes/hashes and accepts matching local payloads. Preserve feat/ghw-oracles and its accumulated commits when changing library branches; inspect the parent-selected SHA and task-branch status instead of automatically running submodule update over it. Reattach the existing task branch after a deliberate detached checkout. Keep factory provider links bound to this submodule, and preserve ignored generated payloads/receipts needed by local evidence. Never push, tag, release, change hashes for unchanged recordings, or turn an unresolved upstream issue into a passing skip. Only the explicit verified oracle/tag import may change sidecars.

A failed or broken query must discard incomplete retained state and permit a new independent query. An oversized optional checkpoint must fall back to correct prefix replay. Partial visitor delivery before a late source error is permitted; owned operations must return an error rather than partially accepted owned histories.

Keep library probes, upstream clones, decoded dumps, downloaded archives and profiling output under ignored tmp/. Factory builds/generated payloads and receipt snapshots stay in its ignored .work-ghw/ and bundle outputs. Track only factory sources, recipes, tests and reviewed public build definitions; never put machine-local infrastructure, credentials, proprietary versions or artifact payloads in Git/Docker build contexts.


## Artifacts and Notes


The planning evidence from the initial pin is:

    fixture commit: aadb6d00597265fdef1a8227825465dbffecee43
    GHW metadata entries: 93
    GHW artifact bytes: 139982142
    nonempty GHW oracles: 0
    previous canonical-reader complete scans: 89
    previous canonical-reader failures: 1 fatal type parse, 3 EOF inside CYC

These counts do not establish semantic coverage. Reproduce the failure classifications in milestone 1 and replace the initial pin in this plan when independent oracle evidence is available.

After each milestone, add the exact named tests and short decisive outcomes here or in Outcomes & Retrospective. Retain benchmark raw results only in ignored output. Give reviewers concrete source/type/issue mappings and measurement boundaries rather than a large undifferentiated passing-test count.


## Interfaces and Dependencies


The private backend value entry points must fit the existing Reader contract:

    pub(crate) struct Reader { /* input, source layout, body start, private state */ }

    impl Reader {
        pub(crate) fn open(
            input: Box<dyn crate::backends::Input>,
            name: String,
        ) -> crate::Result<(Self, crate::Hierarchy, crate::Metadata)>;

        pub(crate) fn read<B>(
            &mut self,
            signals: &[crate::Signal],
            end: crate::Time,
            visitor: impl for<'v> FnMut(usize, crate::Time, crate::ValueRef<'v>)
                -> std::ops::ControlFlow<B>,
        ) -> crate::Result<std::ops::ControlFlow<B>>;
    }

The visitor index is the public base signal's private index used by the common engine, never a source scalar ID or caller's duplicate selection position. Deliver nondecreasing source ticks and valid borrowed views. Empty selected signals return Continue without traversal.

The body implementation must additionally expose the narrow GHW Value/Tick callback described in milestone 4. A private GHW-specific observation enum is sufficient. Reader::read can adapt this stream by forwarding values, while the GHW arm of read_ticks_from consumes both variants. This is required for completed-tick delivery and early stopping, independent of the optional checkpoint optimization. Keep normalization and Slot ownership in the query engine.

Private wire/type/body/hierarchy modules may share concrete enums and structs for storage kinds, ranges, source IDs, and recipes. Keep source type descriptors separate from public declaration metadata. The normal dependency list and FST reader remain unchanged; no unsafe FFI is necessary. Tests and Criterion use the existing dev dependencies. An optional continuation is a narrow extension of the actual query/reader code, not a speculative shared abstraction.


## Revision Notes


2026-10-11: Expanded the initial skeleton into a source-based executable plan. Added pinned format grammar, issue-driven completeness, native API integration, independent fixture evidence, honest performance boundaries, and KISS/YAGNI constraints. Applied the three substantive Astra high review findings as described in Outcomes & Retrospective. Targeted re-checks and a fresh independent control pass reported no remaining substantive findings. Backend implementation and its validation remain future milestones.

2026-10-12: Studied the current VCD/FST implementations and strengthened the plan with exact reuse boundaries, active-slot/recipe work, provisional replay ownership, independent oracle derivation, existing regression schedules, controlled workload behavior and fresh/warm measurement setup. Plan the minimal extraction of pure FST query consumers only when GHW uses them. Record the 113 passing self-contained baseline tests separately from unrun external conformance/timings. Two focused Astra high reviews and a fresh independent control pass reported no substantive findings.

2026-10-12: Apply the user's local fixture workflow. Create feat/ghw-oracles in the submodule and neighboring factory, move delivered oracle/source generation into the factory, add its missing GHW/grouped-layout work, specify explicit verified imports and post-import receipt rebinding, and remove remote publication as a local acceptance prerequisite. Apply the user's follow-up by requiring only the grouped layout, without a legacy fallback or compatibility tests. Preserve task branches during submodule recovery. No push, tags or releases are authorized. Fix the focused Astra high review's full-suite gate finding; its targeted re-check and a fresh independent control pass report no substantive findings.

Revision (2026-10-12): Start implementation under an active goal. Review completed milestones with OCR, use no Astra agents, and commit independently verified work locally. Execute benchmarks only to verify that they work; ignore timing results and defer tuning and query continuation to a separate pass.

Revision (2026-10-12): Implement the independent factory foundation and four native wire tests; import the two original issue attachments without changing their payloads. Focused factory tests initially pass (10); full factory before/after suites have the same one failure and 17 errors. Image build, OCR reviews and fixture gates are still in progress.

Revision (2026-10-12): Complete the initial wire/type decoder and OCR review. Install and hash-verify all 95 GHW inputs; reuse 71 matching local artifacts after unauthenticated release-download rate limiting. Reproduce 91 canonical completions and four diagnosed failures. Fixtures commit 765cf66 adds unchanged original issue-90/99 payload metadata. Fixture validation/check passes (469 fixtures, 42 tool tests). Use an ignored, read-only Git wrapper for the fixture checker because container worktree paths do not resolve the submodule relative Git pointer; do not forward this override to temporary-repository tests. Factory source review and image/producer validation remain pending.

Revision (2026-10-12): Complete two OCR foundation reviews in the oracle factory. Fix enum subtype index validation, isolate the canonical hole diagnostics, use explicit generation settings, capture producer diagnostics, and correct the reproduction comment. Add thirteen focused factory tests. Ignore OCR performance suggestions in the native type decoder, as required by the user. The initial HTTP-heavy image build is superseded by a build that reuses the reviewed public common dependency layer and downloads GNAT over HTTPS; no reader tuning is included.

Revision (2026-10-12): Implement hierarchy conversion, ordered alias recipes, active-ID ordering, generate/process handling, enum metadata and null objects. OCR identifies zero-payload geometry amplification. Bound derived empty-layout scopes to 1000000, return an explicit unsupported resource-limit error, and test both compact valid empty records and huge layouts; a remaining-input bound would incorrectly reject valid compact input. This is a metadata resource limit, not performance tuning. Pin the later factory/fixture SHAs before final conformance.
