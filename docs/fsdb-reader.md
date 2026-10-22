# FSDB SDK loading, serialization and selected state

`build.rs`, `native/fsdb.cpp` and `src/backends/fsdb.rs` own linking and adaptation. `docs/images/fsdb-lib-flow.drawio.svg` shows the chronological cold path; bit-only reads can establish state by seeking.

## Linking is a downstream deployment constraint

The selected Reader libraries have no ELF SONAME. Generated linker scripts preserve their absolute paths in the application's dependencies; SDK variants with SONAME are rejected. A crate-local RPATH would not solve downstream loading. A missing or moved SDK can prevent process startup before Rust runs, even when that application intends to open another format.

The Reader does not declare all its own native dependencies. A two-address link anchor keeps libnsys and zlib from being discarded by lld's as-needed handling. It does not read vendor symbol contents. Only the Ondas shim is static; there is no separate Ondas shim shared library or copied vendor SDK in the crate. The standalone consumer checks the propagated dependencies outside Cargo's loader environment.

## Independent Reader objects still share an SDK lock

Each waveform owns a non-shared Reader object, but all adapter SDK calls, including destruction, are serialized. This protects process-global SDK behavior; it does not coordinate unrelated SDK users in the application. Values are copied while locked and Rust visitors run after unlocking, so a visitor can query another waveform without retaining a vendor buffer or native callback frame.

Metadata-only opening skips datatype and hierarchy callbacks. Its success can coexist with full-open failure on conflicting declarations. Full opening also does not validate every value history. Keep those qualification boundaries separate.

The SDK stdout control is process-global. It does not redirect application descriptors or suppress stderr. Native exception handling also does not contain aborts or faults; consumers needing diagnostic or crash isolation require a process boundary.

## Loading precedes delivery

The SDK loads selected histories before chronological callbacks. View windows can reduce the loaded interval, but they load complete intersecting flush sessions. A short requested window or early `Break` does not bound that allocation or work.

Cold bit-only points seek and walk earlier completed ticks until the selected projection differs. A constant projection can require its whole prefix to establish change-time uncertainty. Mixed values and events use chronological state establishment. Seeking a cold bit-only window obtains state before its start; a point snapshot after that tick cannot substitute for the entering state.

Selections can retain bounded normalized final and boundary state, not live SDK cursors or a growing history cache. Event selections keep the reference path for exact preceding-tick counts. A traversal guard releases its cursor and SDK selection on completion, stop, error or panic. An actual later traversal creates another cursor even if it reuses owned state.

The cursor checks the inclusive end before adapter value conversion. This keeps excluded payloads out of conversion. Selected payload inside an advanced interval can still fail before a consumer asks for it; deferred errors are not guaranteed.

## Native spelling and support evidence

An SDK scope and a record/struct container can occupy the same path. Repeated source objects may merge, but equal logical paths do not identify them. Verilog normalization requires verified language/spelling evidence. Even matching SDK range metadata does not permit stripping an attached suffix from an escaped identifier. `reader_name` retains diagnostic spelling.

Enum storage determines what can be decoded: per-bit logic and integral codes are different transports. A table is exposed only with a proven whole-value mapping. Integral enum widths beyond the supported native representation are a support limit, not permission to truncate a key. Real precision likewise follows storage rather than the source type name. String transport is NUL-terminated and cannot preserve embedded-NUL contents.

Confirmed newer file versions, unsupported timestamp storage and event queries with dump-off ranges have specific support causes. Unknown SDK statuses or indistinguishable failed opens do not establish corruption or unsupported input by themselves. A newer-file classification requires comparable file-header and Reader API versions; otherwise the generic failure remains. `fixture-conformance.md` explains why compatibility skips are not query coverage.
