# Changelog

## Unreleased

### Added

- Independent `fst-native` reader for FST files and shared in-memory bytes.
- Independent `vcd-native` reader with full opening validation and batched replay.
- Self-contained VCD regressions and full-pool file/bytes conformance.
- Hierarchy lookup, aliases, bit projections, point/range queries, and reusable selections.
- Self-contained API tests and a public-contract coverage map.
- FST conformance discovers every artifact in the locked provider, batches listed
  observations in file/bytes modes, and reports all case failures.

### Changed

- Minimum supported Rust version is 1.88.0 to support `fst-reader` 0.17.0.
- Hierarchy lookup avoids constructing paths for unrelated declaration names.
- Fixture provider pin is 4.1.0; FST and VCD share format-neutral oracle checks.

### Fixed

- Preserve present-but-empty FST header text and canonical declaration kinds.

### Reader limitations

- The FST upstream reader is unmodified; some malformed inputs can panic.
- VCD checkpoint event records are not occurrences; hidden physical activity is
  not reconstructed. Nonzero timezero and EVCD strength records are unsupported.
- First-tick event callbacks can include initialization. FST string bytes map as Latin-1.
