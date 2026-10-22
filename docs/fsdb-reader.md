# FSDB loading, state reuse and SDK boundaries

`native/fsdb.cpp` calls the Reader API. `src/backends/fsdb.rs` copies its observations into the common engine. `docs/images/fsdb-lib-flow.drawio.svg` shows the paths. Events and mixed values use the chronological cold path. Bit-only reads can establish state by seeking.

## Linking is a downstream deployment constraint

The selected Reader libraries have no ELF SONAME. Generated linker scripts preserve their absolute paths in the application's dependencies. SDK variants with SONAME are rejected. A crate-local RPATH would not fix downstream loading. A missing or moved SDK can prevent process startup before Rust runs. This also affects applications that intend to open another format.

The Reader does not declare all its native dependencies. A two-address link anchor keeps libnsys and zlib from being discarded by lld's as-needed handling. It does not read vendor symbol contents. Only the Ondas shim is static. The crate has no separate shim shared library and contains no copied vendor SDK. The standalone consumer checks downstream dependencies outside Cargo's loader environment.

## Independent Reader objects still share an SDK lock

Each waveform owns a separate Reader object. All adapter SDK calls, also destruction, use one lock. This protects process-global SDK behavior. It does not coordinate unrelated SDK users in the application. The adapter copies values while locked. Rust visitors run after unlocking. A visitor can query another waveform without retaining a vendor buffer or native callback frame.

Metadata-only opening skips datatype and hierarchy callbacks. It can succeed when full opening fails on conflicting declarations. Full opening also does not validate every value history. Keep these qualification boundaries separate.

The SDK stdout control is process-global. It does not redirect application descriptors or suppress stderr. Native exception handling does not contain aborts or faults. Consumers that need diagnostic or crash isolation must use a process boundary.

## Loading precedes delivery

The SDK loads selected histories before chronological callbacks. View windows can reduce the loaded interval, but they load complete intersecting flush sessions. A short window or early `Break` does not bound this allocation or work.

Cold bit-only points seek and walk earlier completed ticks until the selected projection differs. A constant projection can require its whole prefix to establish change-time uncertainty. Mixed values and events establish state in time order. A cold bit-only window seeks to obtain state before its start. A point snapshot after that tick cannot supply the entering state.

Selections can retain bounded normalized final and boundary state. They do not retain live SDK cursors or a growing history cache. Event selections keep the reference path for exact preceding-tick counts. A traversal guard releases its cursor and SDK selection on completion, stop, error or panic. A later traversal creates another cursor, also when it reuses owned state.

The cursor checks the inclusive end before adapter value conversion. This keeps excluded payloads out of conversion. Selected payload inside an advanced interval can still fail before a consumer asks for it. Deferred errors are not guaranteed.

## Native spelling and support evidence

An SDK scope and a record/struct container can occupy the same path. Repeated source objects can merge, but equal logical paths do not identify them. Verilog name conversion needs verified language and spelling evidence. Matching SDK range metadata does not permit removal of a suffix from an escaped identifier. `reader_name` keeps diagnostic spelling.

Enum storage determines what can be decoded. Per-bit logic and integral codes use different transports. A table needs a proven mapping to whole values. Integral enum widths beyond the native representation are a support limit. They do not permit key truncation. Real precision also follows storage, not the source type name. String transport is NUL-terminated and cannot preserve embedded-NUL contents.

Confirmed newer file versions, unsupported timestamp storage and event queries with dump-off ranges have specific support causes. Unknown SDK statuses do not prove corruption or unsupported input. An indistinguishable failed open does not prove either cause. A newer-file classification needs comparable file-header and Reader API versions. Without them, the failure stays generic. `fixture-conformance.md` explains why compatibility skips are not query coverage.
