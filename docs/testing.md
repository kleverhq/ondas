# Testing

Tests establish the public contracts documented in Rust source. Shared waveform
semantics are tested independently of file formats; reader adaptation is tested
on real waveform artifacts. Fixture structure and oracle interpretation are
owned by [the fixture contract](fixtures.md), not by individual test cases.

## Self-Contained Semantics

Unit tests do not require `ONDAS_FIXTURES`, vendor libraries, or external waveform
readers. Cover:

- hierarchy path parsing, canonical formatting, escaping, and round trips;
- in-memory hierarchy traversal, exact lookup, ambiguity, and aliases;
- time bounds, empty ranges, entering states, and queries beyond recorded ticks;
- packed and non-byte-aligned values, logic states, and projection boundaries;
- composed projections and suppression of changes outside the observed slice;
- history merging, same-tick ordering, redundant writes, and event multiplicity;
- selection order and duplicates, candidate normalization, and error propagation.

Use property tests when an invariant is clearer than enumerated examples, such as
parsing a displayed path back into the same exact components. Start with small,
diagnostic cases rather than broad random inputs with unclear failure causes.
The [API coverage map](api-coverage.md) connects public contract areas to named
executable tests; a passing test count alone does not establish completeness.

## Logical Test Backend

A private, test-only memory reader represents normalized metadata, hierarchy,
and histories. It models a waveform, not VCD syntax or a vendor API. Exercise it
through ordinary Ondas opening/query objects and public observations, rather
than asserting against its private methods.

Its cases include bits, real, string, events, aliases, a first value after tick
zero, distinct changes within a tick, redundant persistent writes, wide values,
and whole/projected signals. This isolates public semantics from reader quirks.

Use narrow counting, failing, or callback test backends for specific questions:
resource reuse and release, batching, shared base-signal preparation, borrowed
lifetimes, early termination, or failure after partial observation. Do not build
a configurable mock framework or simulate a whole proprietary binary format.

## Real Reader Conformance

One common runner applies the same backend-neutral oracle through the public API
across this matrix:

```text
fixture × explicitly selected compatible backend × supported input mode
```

Use real artifacts for adapter correctness. A tiny inline VCD is acceptable when
its text makes a unit test clearer; it is not a general replacement for external
fixtures. Exercise each reader implementation independently; sharing a decoder
dependency does not establish coverage for another format. Proprietary readers
are exercised against actual runtime libraries in an explicitly selected environment.

Select the backend explicitly when testing that adapter. Test automatic backend
selection separately so priority changes cannot silently remove an implementation
from coverage. Compare file and bytes inputs where the reader supports both.

Derive observations from fixture windows rather than restating the same history
in many assertions. Exercise metadata, traversal, spellings, identity, encodings,
samples, batches, selections, traces, scans, entering states, events, projections,
and candidate timestamps. Check the public allowance for extra candidate times,
not equality with an exact list of decoded changes.

Selection order, duplicate inputs, early termination, invalid handles, invalid
slices, path errors, and late callback failures also need ordinary test cases;
the sidecar is not an imperative test language. Reader-specific cases are limited
to behavior the common observations cannot express, such as runtime discovery or
mapping a particular vendor error.

## Fixture Validation and Failure

Consume a materialized local catalog through `ONDAS_FIXTURES`. Validate selected
locked providers, sidecars, artifact paths, sizes, hashes, and oracle semantics
once before conformance assertions. Do not rehash a large artifact per query.
Follow [fixtures.md](fixtures.md) for the authoritative validation rules.

An explicit suite fails on missing configuration, unavailable required readers,
missing selected fixtures, invalid data, or an empty required selection. These
conditions must not become skipped cases or a successful zero-test run.

Catalog validation and semantic conformance are separate results. An empty oracle
is valid catalog data but supplies no semantic assertions; do not report that as
backend conformance coverage. A malformed waveform with an intentional negative
oracle is a test input, not a malformed catalog.

The runner does not download, generate, repair, or publish fixtures. Public and
private providers share the same contract, but a public run must not depend on
private credentials or inaccessible artifacts.

## Gates and Test Execution

The root `justfile` defines executable recipes; discover them with
`./dev just --list`. `./dev just test` runs self-contained Rust tests and doctests;
`just ci` includes them along with static checks and repository-tool tests.
Formatting, compilation, Clippy, and rustdoc alone do not prove runtime correctness.

`./dev just conformance` explicitly runs the ignored real-FST integration tests
against the provider pinned in `fixtures.lock.toml`. Missing environment, provider,
version, artifacts, or oracle data fails this requested suite. The default test
run does not execute these external tests or claim their coverage. `full_fst_pool`
discovers every FST in the provider rather than maintaining a fixture whitelist.
It validates selected artifacts before opening them, then compares every listed
sample and window in file and bytes modes. Equal-time samples and equal-bound
windows are batched to avoid decoding large files once per signal or transition.
Listed declarations are matched from one public hierarchy traversal rather than
repeating linear lookups for thousands of aliases. Exact lookup and alias-iterator
contracts are exercised by the focused cases. Artifact bytes are not cached for
the whole pool.

Small, named regressions additionally exercise query wrappers, projections,
duplicate handles and early termination. The full-pool test reports every case
and aggregates assertion/reader panics so one failure does not conceal later
cases. Panic handling belongs only to the test runner, not the library. Any
mismatch fails the suite; there is no expected-failure allowlist. Catalog errors
abort before conformance. Corpus discovery is not exhaustive signal/time coverage,
reader certification, or a claim of complete schema validation.

Public API examples should compile as doctests. Use `no_run` for examples that
need an external artifact; use runnable examples for self-contained behavior.
The direct documentation check is `./dev cargo test --doc --locked`.

Public and private suites remain explicit and separate, following
[automation policy](automation.md). Performance measurements are not correctness
tests or timing-based CI gates; see [benchmarking](benchmarking.md).
