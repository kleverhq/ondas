# SHM recording selection and SST2 ownership

`native/shm.cpp`, `src/backends/shm.rs` and `tools/repo/shm_build.rs` own the reconstructed reader interface. Export discovery alone does not qualify it: the selected runtime must pass recorded-input checks. Alias identity is optional, so separate handles can still describe the same source history.

## Runtime lookup can change the C++ runtime

SHM supports readers with and without the libsst2.so SONAME. A library without SONAME is retained by canonical absolute path. A SONAME library needs explicit runtime lookup that resolves to the selected build-time reader. Use a directory containing only a symlink to it; adding the whole vendor library directory can load an older bundled C++ runtime. Moving the SDK still requires rebuilding. Missing native libraries can prevent startup before any Rust format routing runs.

## A database directory is one recording selection

The base TRN and numbered components are opened in numeric order. SST2 selects the latest matching recording and joins its split intervals; it does not concatenate every design found in a directory. Components reporting No matching design are omitted, while other component errors fail. The exact DSN/TRN set is therefore part of fixture identity, as described in `fixture-conformance.md`.

Native names already contain logical components, including possible literal backslashes. Some VCD imports preserve an escape marker, but the exposed language metadata cannot distinguish it from a native literal. Broad Verilog cleanup would corrupt names. Packed values and unpacked indexed elements also need their proven layout, not reconstruction from name text.

Enum codes are formatted with their declaration's type, not inferred from label positions or observed occurrences. Quoted string transport supplies byte escapes, including embedded zero; malformed quoting is different from a valid non-ASCII value rejected by the common string contract.

## Reopen under the lock, then release native state

SST2 shares open files and a global time unit. Each native operation opens the recording components, copies the required data and closes under a process-wide mutex. No vendor handle or borrowed buffer survives the operation. Enum tables are copied during hierarchy import. Rust visitors run after unlocking and can read another waveform with a different scale.

Selected queries follow owned enumeration routes from the imported hierarchy. Those routes contain names and positions rather than native handles and assume unchanged source components. They skip unrelated subtrees, but do not make SST2's internal work visible to the adapter counters.

## Certification is not a constant-time point operation

Qualified bit histories can certify continuous availability and seek a point, then walk back through equal values to establish normalized change time. Certification can itself process history inside SST2. Unsupported layouts and non-scalar projections use ordinary reading; an unqualified history need not force every qualified history in the same batch onto that path.

Ordinary cold reads collect selected prefixes through the query end before merging them for delivery. `Break` cannot stop collection that already completed. A prepared selection can reuse bounded owned state or a validated entering boundary, but that is not a native history/handle cache. Events and incompatible requests retain the reference path. This is why callback streaming and total reader residency are separate claims.

A leading unavailable interval can remain Missing. A later unavailable recording interval is an error; it must not silently become a held physical value. Timed decode errors can allow delivery of earlier completed ticks before failure. Known native filesystem codes retain their I/O cause, while unknown SDK failures remain diagnostics rather than guessed categories.
