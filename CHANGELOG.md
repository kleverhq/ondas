# Changelog

## Unreleased

### Added

- Independent `fst-native` reader for FST files and shared in-memory bytes.
- Hierarchy lookup, aliases, bit projections, point/range queries, and reusable selections.
- Self-contained API tests and an explicit, fixture-backed FST conformance suite.

### Changed

- Minimum supported Rust version is 1.88.0 to support `fst-reader` 0.17.0.
- Hierarchy lookup avoids constructing paths for unrelated declaration names.

### Fixed

- Preserve present-but-empty FST header text and canonical declaration kinds.

### Reader limitations

- The upstream reader is unmodified; some malformed inputs can panic.
- First-tick event callbacks can include initialization. FST string bytes map as Latin-1.
