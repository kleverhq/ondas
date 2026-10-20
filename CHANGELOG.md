# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- Restrict reader string samples to exact ASCII (0 through 127). Non-ASCII
  values now return `Error::NonAsciiString`, including valid non-ASCII UTF-8.
  Names, enum labels and diagnostics keep their separate text rules. This
  narrows the string support in 1.1.1 and requires a major release.
- Return no GHW timescale because the file does not store time resolution.
  Raw ticks are unchanged. Consumers must not assume the previous conventional
  1 fs scale when converting time.
- Expose enum tables only when every key matches the whole queryable bit value.
  Change `EnumerationVariant::encoded` from `&str` to `BitsRef`. Inspect keys
  with the bit API; use `to_string()` when text is required. Incompatible
  mappings remain absent instead of requiring reader-specific interpretation. Labels
  retain reader spelling and are not restricted to ASCII.

### Added

- Compare `Scope` and `Variable` views by object identity. Hierarchy clones keep
  identity; independent opens and alias declarations stay distinct.
- Add cross-format exact-name, same-path alias, and scope-collision regression
  fixtures and tests.

### Fixed

- Remove recognized Verilog escape syntax from FSDB and WLF logical names.
  Preserve literal punctuation and backslashes. Keep available reader spelling
  for diagnostics.
- Keep distinct VCD, FST, and FSDB declarations with equal logical paths separate.
  Preserve their parent and child links and report ambiguous lookup.

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

[Unreleased]: https://github.com/kleverhq/ondas/compare/v1.1.1...HEAD
[1.1.1]: https://github.com/kleverhq/ondas/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/kleverhq/ondas/compare/v1.0.2...v1.1.0
[1.0.2]: https://github.com/kleverhq/ondas/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/kleverhq/ondas/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/kleverhq/ondas/releases/tag/v1.0.0
