# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.0.0] - 2026-09-28

Initial release of Ondas, a read-only Rust library for waveform analysis.

### Added

- Built-in VCD reader, FST reader, and optional FSDB reader backed by the local Verdi SDK.
- Unified metadata and hierarchy inspection, aliases, bit projections, and declaration metadata.
- Point samples, owned traces, streaming scans, candidate times, and conditional queries with early stopping.
- Final-tick state normalization and event occurrence counts, with documented reader limits.
- Fixture-backed conformance tests, performance benchmarks, and automated release checks.

[1.0.0]: https://github.com/kleverhq/ondas/releases/tag/v1.0.0
