# FST reader

`fst-native` reads binary FST through
[fst-reader](https://github.com/ekiwi/fst-reader), without Wellen. The dependency
decodes and decompresses; Ondas maps metadata, handles and values to its query
contracts. The adapter is in `src/backends/fst.rs`, with shared queries in
`src/query/engine.rs` (repository-relative paths). Reader state, indices and
buffers stay private. Other format readers remain independent, and another FST
reader can coexist under the [common model](model.md).

## Decoder constraints

The unmodified published `fst-reader` 0.17.0 is BSD-3-Clause, uses Rust compression
libraries and accepts buffered, seekable file/bytes input. Its release source is
[`92b3b821`](https://github.com/ekiwi/fst-reader/tree/92b3b821e88b8058a72a7cd04607a327b8397152),
not an interchangeable newer checkout. Let-chains require Rust 1.88. Keep the MSRV
in `Cargo.toml` and check dependency updates on it even if upstream omits its MSRV.

Opening and traversal do not prove hierarchy completeness. The adapter rejects
scope-stack underflow, unclosed scopes and conflicting repeated scope metadata;
it merges compatible repeated scopes.

Malformed files, including wrong header-section lengths, can panic in debug and
release builds. Ondas neither patches the dependency nor intercepts its panics.
Reported failures become Ondas errors; Rustdoc documents the panic limitation.
This is not a hardened parser for untrusted files. Treat FST as binary data and
inspect it with a reader or fixture tools.

Queries traverse selected histories from the beginning through the requested end
and retain the previous value per selection entry. Decoder time filtering can
emit earlier observations; shared code derives entering state and applies range
bounds. Reusing a selection preserves handles and grouping, not cached histories.

Frame snapshots and changes share a callback shape. Ondas preserves event
callbacks, including initialization at the first recorded tick. Exact event counts
there are a reader limitation: neither payload guessing nor dropping all first-tick
records resolves it.

Declaration metadata distinguishes digital values from character byte slices.
Bits preserve nine logic states. Character bytes map to U+0000 through U+00FF
(Latin-1), including NULs and padding; the reader does not infer UTF-8.

## Normalization and checks

Keep declarations and aliases separate from reader history handles. Time uses
absolute ticks, not reader tables. Declaration ranges differ from normalized bit
positions. Caching or shared projection preparation must not alter selection
order, duplicates or slice histories. Conservative activity candidates do not
replace decoded changes.

`./dev just conformance` discovers every FST in the locked provider and checks all
listed observations in file/bytes modes. Samples and windows are batched; focused
cases cover selections, scans, candidates, projections and termination. The runner
reports each case and aggregates failures rather than excluding them. Provider
additions need no manual case-list update.

See [testing](testing.md) for memory-reader versus adapter evidence. Running the
pool is not the same as passing it, and sparse oracles do not cover every signal,
tick, compression variant or first-tick event behavior.

Converted artifacts retain [provenance](fixtures.md). Conversion alone does not
prove equivalence with the source: publish only observations established for the
result. FST benchmarks use their own format target and explicit reader selection.
Compare the same artifact and workload under common
[measurement boundaries](benchmarking.md), not unrelated FST/VCD datasets.
