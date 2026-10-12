# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Read uncompressed GHW files and shared bytes with the built-in `ghw-native`
  backend, including composite VHDL declarations and the existing query APIs.
- Check native GHW values with independent fixture oracles and execute GHW
  benchmark workloads through the default CI gate.

### Fixed

- Classify missing GHW inner-array bounds as malformed data and require oracle
  evidence for every GHW artifact in conformance.

## [1.0.2] - 2026-10-07

### Changed

- Use a pinned Git submodule for fixtures, with tools that support selective payload installation.

### Fixed

- Preserve FSDB record/struct containers separately from SDK scopes with the same path; scope lookup reports ambiguity while both objects remain accessible through traversal.
- Classify conflicting repeated FSDB scope metadata as `Error::Malformed`.

## [1.0.1] - 2026-09-29

### Fixed

- Accept exact decimal and abbreviated FSDB SDK timescales without changing source tick counts.

## [1.0.0] - 2026-09-28

Initial release of Ondas, a read-only Rust library for waveform analysis.

### Added

- Built-in VCD reader, FST reader, and optional FSDB reader backed by the local Verdi SDK.
- Unified metadata and hierarchy inspection, aliases, bit projections, and declaration metadata.
- Point samples, owned traces, streaming scans, candidate times, and conditional queries with early stopping.
- Final-tick state normalization and event occurrence counts, with documented reader limits.
- Fixture-backed conformance tests, performance benchmarks, and automated release checks.

[Unreleased]: https://github.com/kleverhq/ondas/compare/v1.0.2...HEAD
[1.0.2]: https://github.com/kleverhq/ondas/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/kleverhq/ondas/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/kleverhq/ondas/releases/tag/v1.0.0
