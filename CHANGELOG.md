# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Optional `fsdb-lib` feature/backend using a local Verdi FSDB Reader SDK on
  Linux x86_64 GNU. FSDB input is file-only; executables require the SDK at its
  build-time path. Vendor handles and synchronization remain private.
- Public FSDB conformance and optional private conformance, with a strict
  `ONDAS_REQUIRE_PRIVATE_FIXTURES=1` mode and separate local private version lock.
- Independent `fst-native` reader for FST files and shared in-memory bytes.
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

- Public quality gates and docs.rs use nonvendor features; explicit FSDB recipes
  validate SDK-dependent configurations without weakening public checks.
- FST content detection includes the header length to avoid mistaking FSDB for FST.
- Minimum supported Rust version is 1.88.0 to support `fst-reader` 0.17.0.
- Hierarchy lookup avoids constructing paths for unrelated declaration names.
- Fixture provider pin is 4.1.3; FST and VCD share oracle checks.

### Fixed

- Preserve present-but-empty FST header text and canonical declaration kinds.

[Unreleased]: https://github.com/kleverhq/ondas/compare/v0.1.0...HEAD
