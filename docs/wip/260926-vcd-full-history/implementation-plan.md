# Reduce cold full-range VCD selected-history cost

This ExecPlan is a living document. Update Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective at each milestone; add a change note at the bottom whenever the plan changes. Remove this temporary task directory before merging into `main`, after promoting durable backend conclusions to `docs/vcd-native.md` and durable benchmark policy to `docs/benchmarking.md` if needed.

## Purpose / Big Picture

Issue #33 reports that the first full-range trace of nine SCR1 AXI VCD signals takes about three seconds *after* opening, despite issue #31 making opening fast and issue #32 fixing late narrow queries. The nine traces contain 1,286,446 observations. Make the same exact query and complete streaming scans materially faster without shifting work into `open()`, accepting malformed files, losing event multiplicity, or silently changing the bounded-memory and early-break contracts. First commit a public, locked-fixture benchmark and measure the unfixed code; only then change the reader. A fresh standalone query on the issue's converted public SCR1 AXI FST is the end-to-end check.

## Non-Goals

Do not alter Wavepeek, the public fixture provider, sibling repositories, or private assets. The locked persistent benchmark can use the existing SCR1 **AHB** VCD, which exercises the same full-body parser but is not the issue's AXI artifact. Do not describe its timing as the original AXI timing. No new dependency, opening-time history cache, automatic data download, or CI timing threshold is warranted. Do not change FSDB/FST semantics.

## Progress

- [x] (2026-09-26 17:17Z) Reproduce issue #33 without modifying the library. `fst2vcd` of the *original* public rtl-artifacts FST produces a 953,668,940-byte VCD. Its body SHA-256 matches the converted locked `fst0022` body, while the headers differ only in their `$date` text. The unmodified release-mode library opens the original in ~0.53 s and traces nine AXI signals over ticks `0..=6244302` in ~2.97 s, returning 1,286,446 records and stable observation hash `07be384e4a5312a4`. Streaming scans cost ~2.90 s and yield 1,286,446 observations or 1,248,862 candidate ticks. Scratch inputs and logs are ignored under `tmp/issue33/`.
- [x] (2026-09-26 17:22Z) Add three locked public SCR1 AHB full-history cases for nine named signals to `benches/vcd.rs` and verify over one million changes before timing. Every iteration creates a fresh selection on an already-open waveform. Before any reader change, Criterion 10-sample medians are **3.139 s traces, 3.001 s scan, 2.982 s candidate times**. Save baseline `issue33-before` and commit this benchmark separately before changing query code.
- [ ] Measure the cost of selected-history replay through `src/query/engine.rs::read_ticks_from` and `src/backends/vcd.rs::Reader::read_from`. Prototype the smallest safe on-demand reuse of file chunk boundaries for eligible full-range queries. Keep callbacks chronological and stop on `Break`; bound any in-flight batches, retain serial fallback for byte inputs and ambiguous or resource-limited cases. Recheck issue #31 `open()` and issue #32 narrow-query controls.
- [ ] Add focused boundary/equivalence tests including same-tick net changes, events and slices, early-break behavior, error propagation, and one-shot/repeated queries. Compare every record of the nine original AXI traces before/after, not just their counts. Run public FST/VCD conformance, clean-tree `./dev just ci`, MSRV and doctests, benchmark matched before/after, and obtain a read-only Astra Medium review before claiming completion.

## Surprises & Discoveries

- The published locked `fst0022-scr1-max-axi-coremark` FST (`6280b6d9…`) differs in its FST checksum from the issue's rtl-artifacts release FST (`14130025…`), but `fst2vcd` produces the same 953,668,940-byte size and **identical value-change body** (`fe2c56dc…`). Only the VCD header's `$date` differs. Therefore the exact original release artifact stays the standalone reproduction while the locked provider remains usable as a control; neither body hash alone nor an empty oracle proves trace correctness.
- The present cold late-query fast path in `src/query/engine.rs::read_ticks_from` only prepares a small selected-state prefix when a range starts after tick zero. A full-range query starts at zero and instead rereads almost one gigabyte serially. Nine AHB control signals yield 2,365,644 records in ~3.06 s, a useful locked-fixture workload but not an AXI result.

## Decision Log

- Decision: Commit public benchmark coverage and capture a before-fix Criterion baseline *before* any reader changes, as requested. Rationale: separating opening, selection, and iteration time prevents an opening-time regression from masquerading as a fix. Date/Author: 2026-09-26, main session.
- Decision: Keep the exact rtl-artifacts SCR1 AXI VCD under ignored `tmp/issue33/` and use existing locked AHB VCD for the permanent benchmark. Rationale: editing/publishing a sibling fixture provider requires separate authorization and the existing public AHB file already exercises the same large VCD reader; label the distinct waveforms explicitly. Date/Author: 2026-09-26, main session.

## Outcomes & Retrospective

Both the exact AXI scratch reproduction and the committed-fixture AHB Criterion baseline have been measured before a reader edit; implementation, post-fix measurements and review remain pending. The goal is a material reduction from roughly three seconds on the exact AXI nine-history query, with no first-open or narrow-query regression. Wellen's ~14 ms selected-history load is a different API/representation and is a diagnostic, not a promised equal-speed target.

## Context and Orientation

`src/waveform.rs::Waveform::traces` creates a fresh `Selection` and forwards to `src/query.rs::Selection::traces`; its `scan_each` collects owned results via callback. `src/query/engine.rs::Selection::read_ticks_from` normalizes records by timestamp and signal, handles entering values, event counts, duplicate selections and change times, and asks `src/backends/vcd.rs::Reader::read_from` for raw records. The VCD reader validates the entire body eagerly during `Reader::open` and retains safe Unix same-file chunk boundaries and a serial replay reader. Issue #32 uses selected-prefix summaries for late narrow queries, but not full-range scans starting at tick zero. `benches/vcd.rs::scr1` currently benchmarks opening, a narrow scan and points using locked public `vcd0097-scr1-max-ahb-coremark`; `tests/support/fixtures.rs` checks its SHA-256. The original AXI artifact is *not* that fixture. `docs/vcd-native.md` describes the streaming and opening contracts.

## Open Questions

Measure how much of the three seconds is parser/tokenization versus selected-value copying and chronological tick normalization before choosing a permanent fast path. Any parallel attempt must batch records to avoid the per-record cross-thread channel regression observed on issue #32; unbounded selected-history buffering would violate streaming `scan()` memory behavior. If batching cannot beat the serial path, keep correctness and record the limitation rather than shifting work to opening.

## Plan of Work

Milestone one is a separate benchmark commit. Extend only the SCR1 locked-file group in `benches/vcd.rs`, resolve nine named AHB signals once outside the timer, then measure fresh one-shot full-range trace and streaming scan/candidate calls. Assert the expected work is nonempty before timing. Compare exact original AXI separately with the ignored standalone program in `tmp/issue33/`; its output hashes all owned trace values and times after the timer. Commit the benchmark alone, so a testable pre-fix revision survives later optimization.

Milestone two investigates measured hotspots before selecting a minimal file-only fast path. Prefer the existing chunk partitions and `Reader::walk` over a second parser or index; deliver bounded groups of selected records chronologically to the existing `read_ticks_from` tick-normalization callback instead of buffering an entire history or forwarding one record at a time. Use serial replay when the file-only fast path cannot safely start, and preserve partial-callback/error and `Break` behavior. Keep narrow point/window queries and opening unchanged. Update this plan with the chosen path and its constraints *before* promoting a prototype.

Milestone three proves equality using unit tests across adversarial cuts plus the original AXI nine-signal output hash, reruns the same committed benchmarks and open controls, and documents only the backend's durable limits. Keep the implementation local until reviewed; report the final commit and verification on issue #33 after publishing the branch if authorized.

### Concrete Steps

Work from the repository root. On a fresh checkout install locked public fixtures with `./dev just fixtures-install`; for an independent reproduction fetch `scr1_max_axi_coremark.fst` from `https://github.com/kleverhq/rtl-artifacts/releases/download/v1.0.0/` into ignored `tmp/issue33/`, then run the host's existing `fst2vcd -f tmp/issue33/rtl-artifacts-scr1-max-axi-coremark.fst -o tmp/issue33/scr1_max_axi_coremark-rtl.vcd`. Verify VCD length 953,668,940 bytes and SHA-256 `36ea63ed30dac723a1392e280f4fc6125e71db1bdb886848095d45417a912747`. Run `./dev cargo run --release --manifest-path tmp/issue33/Cargo.toml -- traces`, `scan` and `candidates`; the scratch program is ignored and may be recreated from the exact nine paths and time span in issue #33. Expect ~1,286,446 trace/scan records and 1,248,862 candidate ticks on the exact AXI VCD.

After the benchmark-only commit, use `./dev cargo bench --locked --bench vcd -- 'vcd0097-scr1-max-ahb-coremark/file/cold-' --warm-up-time 1 --measurement-time 5 --sample-size 10 --save-baseline issue33-before` (adapt the filter to the committed case name) to establish a locked old baseline; rerun it using `--baseline issue33-before` after the change. Run `./dev cargo test --locked --test vcd_native`, `./dev just check-local`, `./dev just ci`, `./dev just msrv`, and `./dev cargo test --doc --locked` for final validation. `just ci` requires a clean committed worktree and includes the public FST/VCD conformance corpus, benchmark smoke and dry-run package. Scratch programs, produced VCDs and Criterion logs remain ignored under `tmp/`.

### Validation and Acceptance

Before a fix, full-range first `traces`, `scan`, and `scan_candidate_times` on the issue's exact original AXI VCD should each take about 2.9–3.1 s after a separate ~0.5 s open. After the fix, require a clear, reproducible large speedup for traces and streaming scans on identical bytes, with open staying near ~0.5 s and #32's first late point/narrow trace not regressing. The nine AXI traces must still yield exactly 1,286,446 records *and* the same complete observation hash `07be384e4a5312a4`; same-tick and event cases in focused tests must match the serial byte reader. Public tests and conformance must pass with no missing locked fixtures accepted. Do not call the new VCD0098 empty-oracle skip positive conformance. Favor a roughly sub-second full selected-history query on this machine, but do not encode timing as a CI pass/fail threshold because local machine load varies.

### Idempotence and Recovery

Generation into ignored `tmp/issue33/` can be rerun on the same public FST without changing tracked files; verify byte hashes before interpreting comparisons. Benchmark and implementation must be separate commits. A failed or ambiguous parallel attempt must revert to serial *before* any callback is emitted, or report the actual error after emissions; it must not restart and duplicate callbacks. Do not overwrite private profiles, mutate fixture providers, or delete other workers' scratch files. Use the already installed worktree hooks and start the current container before each host `git commit`.

### Artifacts and Notes

    Original release FST SHA-256: 1413002528f41c9e1dcbb5ff9f80e6e5edcb0e06093a18b8ada23da4ab283053
    Original converted VCD SHA-256: 36ea63ed30dac723a1392e280f4fc6125e71db1bdb886848095d45417a912747
    Original AXI, pre-fix: open 0.533 s; traces 2.974 s; records 1,286,446; observation hash 07be384e4a5312a4.
    Original AXI, pre-fix: scan 2.903 s / 1,286,446 records; candidate scan 2.893 s / 1,248,862 candidate ticks.
    Locked AHB analogous: open 0.491 s; nine traces 3.062 s / 2,365,644 records; not the original AXI waveform.
    Locked AHB, pre-fix Criterion medians (10 samples each): traces 3.139 s; scan 3.001 s; candidate times 2.982 s. All run after opening with a fresh one-shot selection.

## Interfaces and Dependencies

Keep public `Waveform::traces`, `scan`, `scan_candidate_times`, `Selection`, `ScanRef`, and all existing error, callback and source contracts. `Reader::walk` owns parsing and validation; do not bypass it on hostile or changed file input. Use only standard-library Unix positional reads, scoped threads and bounded buffers, with a serial path on other platforms and for `open_bytes`. No new public API, dependency, provider lock change, unsafe code, or retained history cache is needed. `wellen` remains a benchmark-only pinned development dependency.

Change note (2026-09-26): Created a separate issue #33 plan after reproducing the exact issue workload and verifying the original release input differs from the locked FST only in VCD `$date` after conversion. Preserved benchmark-before-fix ordering and separated permanent locked AHB coverage from the disposable original AXI comparison.

Change note (2026-09-26): Captured all three public AHB Criterion baselines using new fresh-selection full-range cases, before a reader edit. Marked the benchmark milestone complete while keeping the query implementation and validation milestones open.
