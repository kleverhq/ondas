# FST Integration

## Reader Boundary

FST is a binary waveform format. `fst-native` names the direct Rust integration
with [fst-reader](https://github.com/ekiwi/fst-reader), without Wellen. Native does
not mean rewriting the decoder: Ondas owns public metadata, handles, values, and
query semantics; the dependency owns binary decoding and decompression. Reader
adaptation lives in `src/backends/fst.rs`; shared observation logic lives in
`src/query/engine.rs` (paths relative to the repository root).

This integration is independent of readers for other formats and can coexist
with another FST implementation. Reader indices, state, and buffers remain
private; see the [shared model](model.md). Treat artifacts as binary data and
inspect them with a suitable reader or fixture tooling.

## Decoder Constraints

Dependency selection includes the decoder's behavior, not just its API shape.
The published `fst-reader` 0.17.0 is BSD-3-Clause, uses Rust compression libraries,
and accepts buffered, seekable input suitable for files and in-memory bytes.
Its release source is pinned by commit
[`92b3b821`](https://github.com/ekiwi/fst-reader/tree/92b3b821e88b8058a72a7cd04607a327b8397152).
A newer upstream checkout is not interchangeable with that release.

- Version 0.17.0 uses let-chains and requires Rust 1.88. The package MSRV is
  recorded in `Cargo.toml`; test dependency updates on that compiler even when
  upstream does not declare its own MSRV.
- Successful decoder opening and hierarchy traversal do not prove hierarchy
  completeness. The adapter rejects scope-stack underflow, unclosed scopes, and
  conflicting repeated scope metadata. Compatible repeated scopes are merged.
- Version 0.17.0 can panic on malformed input, including a wrong header section
  length, in both debug and release builds. The dependency is used without patches
  or panic interception. Ordinary reported failures become Ondas errors; the
  public rustdoc explicitly documents upstream panic behavior. Do not present this
  integration as a hardened parser for untrusted files.
- Time filtering chooses relevant sections but can emit observations before the
  requested lower bound. Shared query code derives entering state and applies
  Ondas range bounds. Each query traverses selected histories from the beginning
  through its end, retaining only the previous value per selection entry. Selection
  reuse preserves validated handles and grouping, not a full-history cache.
- Frame snapshots and value changes use the same callback shape. The adapter
  preserves event callbacks, including any initialization exposed at the first
  recorded tick. Exact occurrence counts at that tick are a documented reader
  limitation. Do not guess from payload characters or drop all first-tick records.
- Digital values and character strings both arrive as byte slices and are
  distinguished using declaration metadata. Character bytes map reversibly to
  Unicode U+0000–U+00FF (Latin-1), preserving NULs and padding. UTF-8 is not inferred
  from byte contents. Bits retain all nine supported logic states.

These constraints govern decoder selection. Passing selected fixtures does not
establish complete FST coverage.

## Normalization and Queries

Preserve declarations and alias relationships independently of reader-local
history handles. Map timestamps to absolute ticks without exposing a reader time
table. Keep declaration ranges distinct from normalized value-bit positions.

Reader caching and shared preparation for several projections are internal
optimizations. They must not change selection order, duplicate results, or the
observed history of a slice. A cheap activity index may provide conservative
candidate times, but it is not a replacement for decoded value changes.

## Verification and Measurement

Run `./dev just conformance` from the repository root with the locked provider
available. The explicit suite checks three positive fixtures (counter, complex
Icarus values, and NVC shortstring) and four malformed fixtures in file and bytes
modes. It exercises public metadata, hierarchy, aliases, values, samples, traces,
selections, scans, candidate times, projections, and callback termination.

The [common conformance strategy](testing.md) separates real-reader evidence from
self-contained query tests. These selected oracles do not claim full corpus,
compression-variant, or first-tick-event coverage. Additional reader cases should
add targeted fixtures rather than silently widen a claimed support matrix.

A converted FST artifact retains conversion provenance in its
[fixture sidecar](fixtures.md). Conversion does not by itself prove semantic
equivalence with the source artifact; publish only observations actually
established for the resulting waveform.

FST benchmarks use a format-specific target and explicit backend selection.
Compare alternative FST readers on the same artifact and workload, with shared
[measurement boundaries](benchmarking.md), rather than comparing unrelated
VCD and FST datasets.
