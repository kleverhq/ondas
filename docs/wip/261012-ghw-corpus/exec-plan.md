# Give every GHW artifact independent oracle evidence

## Purpose / Big Picture


Every GHW fixture must have a nonempty independent oracle. The current corpus has 102 artifacts, but only 21 profiles: 18 positive and three malformed. The previous implementation proved parser compatibility for other inputs without checking their values. This task closes that coverage gap. An oracle is a sparse set of native observations; every listed signal window must include all changes within its inclusive bounds. Covering every file does not imply covering every signal and tick.

## Progress


- [x] Create the goal and inspect the clean library, factory and fixture branches.
- [x] Confirm 81 missing recipes and preserve the 21 existing reviewed recipes.
- [x] Create and inspect all 80 supported-file recipes. Historical issue 538 omits required nested vector bounds: outer range ends at byte907, the next TYP entry starts at908, and native reading stops at909. It is a diagnosed malformed profile; no schema expansion is needed.
- [x] Generate and repeat all102 profiles with zero failures (98 positive and four malformed). All raw/oracle/tags are repeat-identical.
- [x] Factory OCR completed the generator with zero findings. Focused16 tests pass; the full88-test suite retains exactly the previous one failure and17 errors, with the same18 failing cases. Sources and all102 recipes are committed locally.
- [x] Import102 profiles after receipt verification. Fixture check passes477 JSON/476 fixtures/476 installed artifacts and42 script tests. Fixture branch committed at6f4ee05; factory at9f4570d. Full GHW public conformance passes204 modes with zero failures and zero skips; profiles select1632 histories.
- [ ] Rebind receipts against the final provider commit, compare pre-import output hashes and repeat all profiles.
- [x] Full GHW conformance passes204 modes with zero errors/skips. Coverage and malformed-input docs are updated. Final runner/Rustdoc OCR completed via a focused retry after provider compaction failure; only a source-line-wrap nit remains and is fixed. The low future-tag coupling suggestion is addressed with an explicit guard-reference comment without adding an abstraction.
- [ ] Run final gates on clean commits, promote conclusions and remove task WIP.

## Surprises & Discoveries


The initial 81 no-oracle artifacts consist of 80 supported inputs and one previously unclassified historical error. Inspection proves that ghw0002 omits inner-vector bounds required by both supported wire versions. Bytes902..907 hold a bounded outer array with an unbounded element; byte908 begins another TYP entry, not a scalar range. The pinned C reader reports invalid data and stops at909. Range::read now classifies known type-entry tags in scalar-range position as malformed; unknown storage tags remain unsupported. Directed unit and file/bytes tests pass.

## Decision Log


Use the existing grouped-layout native GHDL factory. Keep existing recipes unchanged. Apply its reviewed sparse selection policy to new supported files: all recipes up to 64, otherwise at most 24 storage representatives; complete recording windows up to 1 MiB, otherwise the first one percent capped at 4,000,000 ticks. Record selected and omitted counts. Investigate any actual coverage or decoding gap rather than change the policy speculatively.

Generation remains read-only with respect to the provider. Only recipes and factory tools are tracked there. Explicitly import verified oracle objects into the fixture submodule on local feat/ghw-oracles. Commit in all three repositories as needed. Do not push, tag, release, involve Astra, tune the backend or compare benchmark figures.

## Outcomes & Retrospective


Candidate recipes exist for all 102 artifacts (98 positive, four inspected malformed). Focused factory tests pass16. Reader OCR completed with only a low-severity future-tag abstraction suggestion, deferred under KISS/YAGNI. Full generation and repeated validation remain in progress. Completion requires all 102 artifacts to carry nonempty independently justified profiles, no unexplained generation failures, final receipt rebinding, repeated identical output and passing public conformance.

## Context and Orientation


The library is in this ghw-integration worktree on feat/ghw-native. fixtures is its Git submodule on local feat/ghw-oracles, initially at 6932ba3. The neighboring ondas-fixtures-oracle repository also uses local feat/ghw-oracles, initially at 7e9a787. Its ghw.py uses unmodified GHDL revision 2c451de76b07b73fd338c91d68fd5cefcb152512 through readers/ghw.c and the ondas-oracle-ghw Docker image. providers/ghw-task/kleverhq.ondas-fixtures is an ignored link to this submodule. The native reader emits hierarchy/type/scalar-ID observations, then selected raw change lists; Python derives the schema-1 oracle without using Ondas or wellen.

src/backends/ghw contains the Rust decoder. tests/conformance.rs executes sidecar evidence in files and shared bytes through the public API. fixtures/schemas/fixture.schema.json owns the shared sidecar/oracle shape; the factory's oracle.schema.json is its matching exported oracle definition. Permanent documentation is in docs/ghw.md, docs/fixtures.md and docs/api-coverage.md.

## Milestones and Concrete Steps


First, run factory candidate planning for missing names and examine its complete report. Read each recipe's exact paths, storage widths, selected counts and bounds. Native failures remain investigation outcomes. For historical issue 538, inspect upstream issue discussion, producer history, the byte position and the native diagnostic. Preserve waveform bytes. Prefer a truthful diagnosed malformed profile or proven historical decoding; extend the existing schema narrowly only if an actual unsupported-layout profile requires it.

Second, run python3 ghw.py generate with the selected names and --provider-root providers/ghw-task from the factory root. Run its focused tests and record the full-suite baseline delta. Review changed tools with ocr review --audience agent and a bounded task-specific background. Resolve substantive findings, then commit tools and recipes. Run verify and repeat for every GHW name. No report may leave an unexplained failed profile.

Third, verify all receipts before copying. Save raw/oracle/tags hashes, then merge each oracle object and reviewed suggested tags into its exact sidecar. Run the fixture just stats and just check recipes through ./dev from the library and update README statistics. Commit the fixture branch. Generate again against that post-import commit, require unchanged raw/oracle/tags hashes, verify and repeat all profiles. Provider changes during this final pass are forbidden.

Fourth, run ./dev cargo test --locked --profile conformance --test conformance full_ghw_pool -- --ignored --nocapture. It must execute all discovered files in both supported input modes. Investigate mismatches against native raw evidence and the pinned producer grammar. Repair actual consumer or decoder errors with the smallest directed test; do not weaken independent expectations. Update the library Git entry and accurate coverage documentation, run OCR on code changes, then ./dev just ci, ./dev just msrv and ./dev cargo test --doc --locked. Benchmark smoke only checks execution; ignore timing output. Finish with clean local commits, remove task WIP while retaining docs/wip/AGENTS.md, and report profile counts and remaining source-format limits precisely.

## Validation and Acceptance


All 102 artifacts have nonempty profiles backed by the native reader and inspected failure evidence where relevant. Positive windows preserve native integer ticks, exact paths, aliases, ordinal labels, ordered same-tick writes and binary64 payloads. Artifacts retain their original hashes. Factory fresh repeats match raw/oracle/tags byte-for-byte; receipts bind the final fixture commit. The required full GHW pool has no no-oracle skips and no observation mismatches. Focused factory tests have no new failure; unrelated baseline failures remain explicitly separate. No publication or performance tuning occurs.

## Idempotence and Recovery


Planning refuses to overwrite existing recipes. Reuse generated outputs only after receipt verification. Regenerate after source, recipe, image or provider fingerprint changes. Failed extraction never becomes empty success or an inferred negative oracle. Use ignored tmp/ in the library and .work-ghw/ in the factory for logs, snapshots and scratch. Do not delete other task outputs. Import can be retried from verified generated objects, preserving artifact and provenance fields. Preserve the local fixture/factory branches and unrelated provider links.

## Interfaces and Dependencies


No new production dependency or public backend framework is needed. Native C evidence is isolated in a Docker process; Rust production does not link it. An actual unsupported historical format can require a narrow oracle error-profile change, which must be reflected in the fixture schema, exported factory schema and runner together. Otherwise preserve existing API and schema semantics.

## Revision Notes


2026-10-12: Start full-corpus oracle work after the user explicitly requested all GHW files; preserve the implemented backend and broaden independent coverage beyond the prior selected profiles.
