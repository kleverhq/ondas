# Testing

Tests check the public contracts in Rustdoc. Memory-reader tests isolate shared
semantics; real artifacts test adapters. [fixtures.md](fixtures.md) defines the
catalog and oracle contract, and [api-coverage.md](api-coverage.md) maps contracts
to executable tests. A passing test count alone does not establish coverage.

## Self-contained tests

These tests need no `ONDAS_FIXTURES`, vendor libraries or external readers. Cover
path parsing, escaping and round trips; hierarchy lookup, ambiguity and aliases;
time bounds and entering state; packed and non-byte-aligned values; projection
bounds and composition; same-tick ordering, redundant writes and events; selection
order, duplicates, candidates and errors.

Use small diagnostic cases. Property tests suit invariants such as parsing a
formatted path back to its exact components; broad random inputs need a clear
failure interpretation.

The private memory reader models normalized waveforms, not VCD syntax or a vendor
API. Exercise it through ordinary waveform and query objects. Cases include bits,
reals, strings, events, aliases, delayed first values, wide values, whole signals
and projections. Narrow counting or failing readers can check batching, resource
reuse/release, borrowed lifetimes, early termination and partial failure. A
configurable mock framework or simulated proprietary format is unnecessary.

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
but must contain every required change time.

Ordinary tests cover selection order, duplicate inputs, early termination, invalid
handles/slices, path errors and late callback failures. Sidecars are observations,
not a test-command language. Reserve reader-specific cases for behavior the common
oracle cannot express, such as runtime discovery or vendor error translation.

## Validation and failures

Before conformance, validate the selected locked providers, sidecars, paths,
sizes, hashes and oracle semantics once. Do not rehash artifacts per query.
[fixtures.md](fixtures.md) defines the checks.

Missing configuration, required readers or required selected fixtures must fail,
as must invalid data and an empty required selection. Optional private provider
or payload absence is reported explicitly as a skip; corruption is never a skip.
Do not describe skipped inputs as conformance coverage. A deliberately malformed waveform can still be a valid
fixture with a negative oracle.

Catalog validation and conformance are separate results. An empty oracle is valid
catalog data but supplies no semantic coverage. Tests do not download, generate,
repair or publish fixtures. Public runs must work without private credentials or
inaccessible artifacts; private providers use the same local contract.

## Commands and coverage

Run `./dev just --list` for recipes. `./dev just test` runs self-contained Rust
tests and doctests. `just check-local` adds static checks and repository-tool tests
and is the fixture-free pre-commit gate. `just ci` also requires conformance.
Formatting, Clippy, compilation and Rustdoc do not replace runtime tests.

`./dev just conformance` runs the ignored FST/VCD integration tests and
feature-disabled FSDB routing against the locked provider. The routing check also
requires its public FSDB artifact. Missing environment, provider, version, artifacts or oracle data
fails the suite. Default test runs leave these external tests ignored and do not
claim their coverage.

`./dev just conformance-fsdb` enables `fsdb-lib` and checks public FSDBs in file
mode, plus available private FSDBs. The private version is pinned separately in
ignored `fixtures.private.lock.toml`; set `ONDAS_REQUIRE_PRIVATE_FIXTURES=1` to
require that provider and every selected payload. Missing public inputs always
fail. The runner validates installed metadata even when an optional payload is
absent, and reports passed, failed and skipped cases separately.

`./dev just ci-fsdb` adds vendor-enabled static, unit, documentation and MSRV
checks. Focused FSDB regressions cover path/bytes routing, reentrant callbacks,
independent opens, cross-thread use/drop, panic cleanup, repeated selections,
slices and missing/corrupt payload policy. The vendor gate also runs
`just fsdb-consumer` on development Rust and MSRV: a separate executable launches
outside Cargo's runtime environment to verify native dependency propagation.
The value classes asserted by supplied oracle observations, not merely successful
opens, determine adapter coverage. Request missing cases from the fixture producer;
never turn the new adapter's output into expected observations.

`full_fst_pool` and `full_vcd_pool` discover all matching artifacts without a
whitelist. They validate inputs before opening and compare every listed sample
and window in file/bytes modes. Equal-time samples and equal-bound windows are
batched. One hierarchy traversal matches listed declarations; focused tests cover
exact lookup and alias iterators. The runner does not cache the pool's artifact
bytes.

Named regressions cover query wrappers, projections, duplicate handles and early
termination. The full-pool runner reports every case and aggregates assertion and
reader panics; any mismatch fails, with no expected-failure allowlist. Catalog
errors abort before conformance. Panic interception belongs to tests, not the
library. Corpus discovery does not certify every signal, tick, reader behavior
or schema rule.

Public examples should be doctests: runnable for self-contained behavior,
`no_run` when they need external artifacts. Check them directly with
`./dev cargo test --doc --locked`. Public and private suites stay separate under
[automation policy](automation.md). [Performance measurements](benchmarking.md)
are neither correctness tests nor timing-based CI gates.
