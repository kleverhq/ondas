# Changelog

All notable changes follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [2.0.0] - 2026-10-30

### Breaking changes

- `EnumerationVariant::encoded` is now `BitsRef` instead of `&str`. Compare keys with the bit API; use `to_string()` for text. Enum tables are absent unless every key matches the complete queryable bit value. Labels retain reader spelling and can contain non-ASCII text.
- String samples now accept exact ASCII (0 through 127) only. Non-ASCII values, including valid UTF-8, return `Error::NonAsciiString`. Names, enum labels and diagnostics keep their separate text rules.
- GHW metadata now returns no timescale because the file does not store time resolution. Raw ticks are unchanged. Supply an external scale for physical-time conversion instead of assuming 1 fs.

### Added

- Optional `wdb-lib` reader for WDB files, with support for older reader installations and typed errors for confirmed newer DBG recordings. Dynamic SystemVerilog String and named Event reads are unsupported. Changing unsigned 129-bit enums are unqualified and can return stale values. Reader setup and other limits are in `wdb-reader.md`.
- Static bit composition with `Waveform::concat`, including nested signals, slices and repeated parts. Missing parts become X when another part is observed; an entirely Missing composition and its slices remain Missing.
- Checked translation of signed HDL indices with `BitRange::slice`.
- Explicit-time batches with `Selection::samples_at` and `Waveform::samples_at`. Results preserve time and signal order, repetitions and quiet ticks, and return a complete owned batch or an error.
- Borrowed saved-history reads with `Trace::sample` and `Trace::samples_at`, typed `Error::TraceCoverage` outside coverage, and `SampleRef::to_owned` for independent observations.
- Exact equality for `BitsRef`, `Bits` and mixed pairs, including width and every logic state.
- Object-identity equality for `Scope` and `Variable`. Hierarchy clones keep identity; independent opens and alias declarations stay distinct.
- Complete recorded SHM and WLF enum tables with full-width bit keys, including wide, signed and X/Z codes and variants absent from history. WLF array elements are exposed as declarations.

### Changed

- SHM delivers completed ticks with bounded adapter buffers and stops traversal after bounded read-ahead. SDK history storage and total memory use have separate costs.
- Improve WDB reads, hierarchy lookup and traversal, and bit value formatting.
- Disable FSDB SDK messages to stdout; stderr remains visible. Consumers that need diagnostic or crash isolation must use a process boundary.
- Replace format-specific benchmarks with the verified OPB runner and quick checks.

### Fixed

- Preserve literal punctuation and backslashes in logical names while removing recognized Verilog escape syntax in FSDB and WLF. Keep distinct VCD, FST and FSDB declarations with equal logical paths and report ambiguous lookup.
- Preserve entering state in unbounded traces and scans that start after EOF, without synthetic changes.
- Report confirmed GHW and FSDB source or query limits as `Error::UnsupportedFeature` instead of `Error::Backend`.
- Preserve confirmed SHM initialization failures as `Error::BackendUnavailable` and known native filesystem failures as `Error::Io`. Keep available operation, status, history and tick details in diagnostics; unknown failures remain `Error::Backend`.
- Classify unfinished FST gzip wrappers as `Error::Backend` instead of `Error::Malformed`.

## [1.1.1] - 2026-10-17

### Changed

- Reduce SHM cold point and bounded-window collection for qualified four-state
  wire and integer layouts. Keep ordinary fallback local to each unqualified
  history in a mixed point batch.
- Resolve selected SHM declarations through owned enumeration routes instead of
  importing the full hierarchy on each query.

## [1.1.0] - 2026-10-14

### Added

- Read uncompressed GHW files and shared bytes with the built-in `ghw-native`
  backend, including composite VHDL declarations and the existing query APIs.
- Check native GHW values with independent fixture oracles and execute GHW
  benchmark workloads through the default CI gate.
- Read recorded SHM directories with the optional `shm-lib` backend through a
  compatible Xcelium reader (2021+), using the existing waveform query APIs.
- Add SHM fixture conformance, SDK sweeps, downstream linking checks, and
  controlled benchmark workloads.

- Read WLF files with the optional `wlf-lib` backend through an installed Questa
  SDK, including metadata, declarations and the existing waveform query APIs.
- Add WLF fixture conformance, SDK sweeps, downstream linking checks, and
  controlled benchmark workloads.

### Changed

- Run FSDB checks and benchmarks for each explicitly selected SDK.
- Use optimized builds with debug assertions and overflow checks for fixture
  conformance.

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

[Unreleased]: https://github.com/kleverhq/ondas/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/kleverhq/ondas/compare/v1.1.1...v2.0.0
[1.1.1]: https://github.com/kleverhq/ondas/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/kleverhq/ondas/compare/v1.0.2...v1.1.0
[1.0.2]: https://github.com/kleverhq/ondas/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/kleverhq/ondas/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/kleverhq/ondas/releases/tag/v1.0.0
