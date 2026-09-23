# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [1.0.0] - 2026-09-21

### Added

- `Scope::name_was_escaped` and `Variable::name_was_escaped` preserve VCD/FST
  declaration escape provenance independently of canonical names.
- FSDB enum datatype names, declared value/label tables and supported integral
  logic histories, including datatype-specific variable callbacks.
- `Scope::is_hidden` preserves explicit FSDB visibility metadata without filtering
  the hierarchy; callers can inspect ancestors when suppressing hidden subtrees.
- Optional declaration signedness and logic-domain metadata, independent of
  shared signal histories; unavailable interpretation remains absent.
- Indexed borrowed scans with selection-position identity for aliases,
  duplicates and projections.
- Bounded callback queries with separate driver/readable subsets and fallible
  current/preceding-tick reads through one query owner.
- Independent legacy-oracle derivation, streaming/composition regressions and
  runnable public usage examples.

- Optional `fsdb-lib` feature/backend using a local Verdi FSDB Reader SDK on
  Linux x86_64 GNU. FSDB input is file-only; executables require the SDK at its
  build-time path. Vendor handles and synchronization remain private.
- Public FSDB conformance and optional private conformance, with a strict
  `ONDAS_REQUIRE_PRIVATE_FIXTURES=1` mode and separate local private version lock.
- `fst-lib` adapter to `fst-reader` for FST files and shared in-memory bytes.
  The unmodified upstream reader can panic on malformed input and include
  initialization in first-tick event counts. Strings map bytes as Latin-1.
- Independent `vcd-native` reader with full opening validation and batched replay.
  Checkpoint event records do not count as occurrences; the reader does not
  reconstruct hidden physical activity or support nonzero timezero and EVCD
  strength records.
- Self-contained VCD regressions.
- Hierarchy lookup, aliases, bit projections, point/range queries, and reusable selections.
- Self-contained API tests and a public-contract coverage map.
- FST/VCD conformance discovers the full locked pool, batches listed observations
  in file/bytes modes and reports all case failures.

### Changed

- Seek cold FSDB bit-only point samples and prove their normalized projected
  change times from prior completed ticks, rather than replaying the whole prefix.
- Build the shared variable-path index lazily, avoiding its allocation and teardown
  for metadata-only opening and hierarchy traversal.
- Rename the FST backend from `fst-native` to `fst-lib` to reflect its external
  reader dependency. Explicit backend selection and reported names use `fst-lib`.

- Persistent observations use final tick states across samples, scans and traces.
  Net-equal excursions no longer appear as changes or advance `changed_at`;
  unfinished ticks are not published on read failure.
- Borrowed and owned event values carry `occurrences: u64`; scans and traces emit
  one aggregate per selected entry/tick instead of repeated unit markers. Update
  matches on `ValueRef::Event` and `Value::Event` to account for the count.
- Real representation identity compares exact bits, including NaN payload/sign
  and signed zero. Numerical equality policies remain caller-owned.
- Version-1 fixture sidecars keep their original meaning; conformance derives
  normalized expectations without inventing NaN identity or change timestamps.

- Public quality gates and docs.rs use nonvendor features; explicit FSDB recipes
  validate SDK-dependent configurations without weakening public checks.
- FST content detection includes the header length to avoid mistaking FSDB for FST.
- Minimum supported Rust version is 1.88.0 to support `fst-reader` 0.17.0.
- Hierarchy lookup avoids constructing paths for unrelated declaration names.
- Fixture provider pin is 4.6.1; FST and VCD share oracle checks.

### Fixed

- Preserve attached literal range suffixes in escaped FSDB scalar and vector
  identifiers, including ambiguous duplicate names, without losing separately
  printed declaration ranges.
- Preserve present-but-empty FST header text and canonical declaration kinds.

[Unreleased]: https://github.com/kleverhq/ondas/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/kleverhq/ondas/releases/tag/v1.0.0
