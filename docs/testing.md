# Testing

Tests check the public contracts in Rustdoc. Memory-reader tests check shared
semantics. Real artifacts test adapters. [fixtures.md](fixtures.md) links the
fixture-owned schema and defines integration and oracle interpretation;
[api-coverage.md](api-coverage.md) maps contracts to executable tests. A passing
test count alone does not establish coverage.

## Self-contained tests

These tests need no initialized fixture submodule, vendor libraries or external
readers. Cover
path parsing, escaping and round trips; hierarchy lookup, ambiguity and aliases;
time bounds and entering state; packed and non-byte-aligned values; projection
bounds and composition; same-tick ordering, redundant writes and events; selection
order, duplicates, candidates and errors.

Use small diagnostic cases. Use property tests for invariants, such as parsing a
formatted path back to its exact components. If you use broad random inputs,
make sure failures have a clear meaning.

The private memory reader models normalized waveforms, not VCD syntax or a vendor
API. Exercise it through ordinary waveform and query objects. Cases include bits,
reals, strings, events, aliases, delayed first values, wide values, whole signals
and projections. Narrow counting or failing readers can check batching, resource
reuse/release, borrowed lifetimes, early termination and partial failure. A
configurable mock framework or simulated proprietary format is unnecessary.
`src/waveform/semantics_tests.rs` keeps raw observations separate from manually
specified final-state histories. It uses the same reference for public query
operations and different cross-signal orders. Its expected projections use string
positions, not production slicing or normalization.

The generated reader retains no history. Logical counters observe actual pending
slot values after each raw record, read starts, requested bases and resource
leases. A separate test-only tick-work counter counts comparison/commit loop
body visits on a sparse 66-slot VCD schedule; one setup pass over all slots is
counted separately. The counter does not measure scan delivery, SDK work or
other loops over the selection. Fixed-width runs and an early-stop limit on reader
advances check bounded state and lazy delivery. They do not use timing or RSS
assertions. Repeated adjacent-tick reads must
not advance or restart the reader. These counters exclude caller-owned results
and memory used by the reader or SDK. They are not a public metrics API. They observe the
current slot structures, not arbitrary future allocations. New query-owned queues
or caches need their own accounting or directed tests; unchanged slot high-water
marks alone do not prove that a new implementation remains bounded.

Caller payload visits and copies are counted separately from sequential fallback
decoding and state copies. The narrow reader need not validate an unselected
failing channel. This does not promise deferred decoding of selected payload in
the fallback path. Source failures are injected during advancement. Selective
read failures use the reference path's fallible visitor. Both preserve completed observations,
release read resources and permit fresh queries.

`tests/query_composition.rs` checks a caller's ordinary Rust conditions through
public APIs over small VCD inputs. It confirms candidates before reading current
control and a caller-selected event/payload tick. Literal expected rows distinguish
prior-tick state from the range baseline and current state. Cases include sparse
events and non-driver updates. Different signal orders, extra candidates and long
rejected prefixes must not change output policy. Staged payload is committed only
after all fallible reads succeed. This is a local composition regression, not an application
integration or expression evaluator.

## Reader conformance

The common runner applies a backend-neutral oracle through the public API:

```text
fixture × explicitly selected compatible backend × supported input mode
```

Use real artifacts for adapter checks. Inline VCD works for small diagnostic
cases but does not replace the external corpus. Test each reader independently,
even when readers share a decoder. Proprietary readers need their actual runtime
libraries in an explicitly selected environment.

Select the reader explicitly so a priority change cannot remove it from coverage.
Test automatic selection separately, and compare file/bytes modes where supported.
Derive samples, traces, scans, entering states and projections from oracle windows
instead of repeating histories in assertions. Check metadata, traversal, paths,
identity, encodings, batches and selections. Candidate times may include extras,
but must contain every required change time. The test-only normalized-oracle
module derives final tick states and event counts independently of production
code while preserving [version-1 evidence limits](fixtures.md#normalized-expectations).
Do not assert raw change times as normalized times, infer NaN payload identity,
or overwrite installed sidecars to make a comparison pass. Indexed scan checks
retain input-slot identity even when aliases or projections share histories.
Composed-query checks use a driver subset and compare all readable entries at
current/preceding ticks covered by the oracle, in each advertised input mode.

Ordinary tests cover selection order, duplicate inputs, early termination, invalid
handles/slices, path errors and late callback failures. Sidecars are observations,
not a test-command language. Reserve reader-specific cases for behavior the common
oracle cannot express, such as runtime discovery or vendor error translation.

## Validation and failures

Before conformance, check the pinned sidecars, supported assertions, paths,
sizes, hashes and oracle semantics once. Do not rehash artifacts per query.
[fixtures.md](fixtures.md) defines the checks and the runner's supported profile.
Producer schema validation is separate from Ondas conformance.

Missing required inputs or readers or selected fixtures must fail, as must
invalid data and an empty selection. Do not describe skipped inputs as conformance
coverage. A deliberately malformed waveform can still be a valid fixture with a
negative oracle.

Producer metadata validation and conformance are separate results. An empty
oracle is valid fixture data but supplies no semantic coverage. Tests do not download, generate,
repair or publish fixtures.

## Commands and coverage

Run `./dev just --list` for recipes. `./dev just test` runs self-contained Rust
tests and doctests. `just check-local` adds static checks and repository-tool tests
and is the fixture-free pre-commit gate. `just ci` also requires conformance.
Formatting, Clippy, compilation and Rustdoc do not replace runtime tests.

`./dev just conformance` runs the ignored FST/VCD integration tests and
feature-disabled FSDB routing against the pinned fixtures. The routing check also
requires its FSDB artifact. Missing artifacts or required oracle observations
fail the suite. Default test runs leave these external tests ignored
and do not claim their coverage.

`./dev just conformance-fsdb` enables `fsdb-lib` and checks FSDBs in file mode
against the pinned fixtures. Missing inputs fail. The runner reports passed,
failed and skipped cases separately; sidecars without oracle observations supply
no conformance evidence.

Both conformance recipes use the optimized `conformance` Cargo profile for the
library, runtime dependencies and tests. Debug assertions and integer overflow
checks remain enabled. Self-contained tests use the default test profile.

The fixture-backed gate executes `just bench-smoke` after conformance;
the vendor gate runs `just bench-smoke-fsdb` after its conformance. Criterion
`--test` checks that registered workloads execute. It uses no timing thresholds
and does not prove a speedup. New sparse/dense file/bytes and adapter diagnostic tests
require the pinned corpus; local submodule edits supply only
development evidence. The conflict fixture asserts a malformed opening error
in its oracle. The focused `fsdb_conflicting_scope_diagnostic_and_metadata_bypass`
test separately checks the adapter's diagnostic and metadata-only opening. The
RocketTile regression checks that same-path module and struct scopes remain
accessible through traversal and return `LookupError::Ambiguous` on scope lookup.
Both tests are selected by the vendor gate. The installed FST checks escaped
variable spelling and alias identity in file/bytes mode. No installed FST has an
escaped scope, so an optional ignored converter test retains that separate
scope-provenance check; it is not a release gate.

`./dev just ci-fsdb` adds vendor-enabled static, unit, documentation and MSRV
checks. Focused FSDB regressions cover path/bytes routing, reentrant callbacks,
independent opens, cross-thread use/drop, panic cleanup, repeated selections,
slices and declaration diagnostics. The vendor gate also runs
`just fsdb-consumer` on development Rust and MSRV: a separate executable launches
outside Cargo's runtime environment to verify native dependency propagation.
The value classes asserted by supplied oracle observations, not merely successful
opens, determine adapter coverage. Request missing cases from the fixture producer;
never turn the new adapter's output into expected observations.

`full_fst_pool` and `full_vcd_pool` discover matching artifacts inside the
`fixtures/fst/` and `fixtures/vcd/` directories without a fixture whitelist. They validate
inputs before opening and compare every listed sample and window in file/bytes
modes. Equal-time samples and equal-bound windows are
batched. One hierarchy traversal matches listed declarations; focused tests cover
exact lookup and alias iterators. The runner does not cache the pool's artifact
bytes.

Named regressions cover query wrappers, projections, duplicate handles and early
termination. The full-pool runner reports every case and aggregates assertion and
reader panics. Any mismatch fails, with no expected-failure allowlist. Input
errors abort before conformance. Panic interception belongs to tests, not the
library. Corpus discovery does not certify every signal, tick, reader behavior
or schema rule.

Public examples should be doctests: runnable for self-contained behavior,
`no_run` when they need external artifacts. Check them directly with
`./dev cargo test --doc --locked`. [Performance measurements](benchmarking.md)
are neither correctness tests nor timing-based CI gates.
