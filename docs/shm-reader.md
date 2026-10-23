# SHM recording selection and SST2 ownership

`native/shm.cpp`, `src/backends/shm.rs` and `tools/repo/shm_build.rs` define the reconstructed reader interface. Export discovery alone does not qualify it. Check the selected runtime with recorded inputs. Alias identity is optional. Separate handles can therefore describe the same source history.

## Runtime lookup can change the C++ runtime

SHM supports readers with and without the libsst2.so SONAME. A library without SONAME keeps its canonical absolute path in downstream dependencies. A SONAME library needs explicit runtime lookup. It must resolve to the selected build-time reader. Use a directory with only a symlink to that reader. The full vendor library directory can load an older bundled C++ runtime. Rebuild the application if you move the SDK. Missing native libraries can prevent startup before Rust selects an input format.

## A database directory is one recording selection

The base TRN and numbered components open in numeric order. SST2 selects the latest matching recording and joins its split intervals. It does not concatenate every design in a directory. Components that report No matching design are omitted. Other component errors fail. The exact DSN/TRN set is therefore part of fixture identity. `fixture-conformance.md` explains that identity.

Native names already contain logical components. These can include literal backslashes. Some VCD imports preserve an escape marker. The exposed language metadata cannot distinguish that marker from a native literal. General Verilog cleanup would corrupt names. Packed values and unpacked indexed elements also need proven layout. Name text alone cannot establish it.

Enum codes use their declaration's type for formatting. The adapter does not infer codes from label positions or observed occurrences. Quoted string transport supplies byte escapes, also for embedded zero. Malformed quoting is a transport error. It differs from a valid non-ASCII value that the common string contract rejects.

## Reopen under the lock, then release native state

SST2 shares open files and a global time unit. Each native operation opens the recording components, copies the required data and closes under one process-wide mutex. No vendor handle or borrowed buffer survives the operation. Hierarchy import copies enum tables. Rust visitors run after unlocking. They can read another waveform with a different scale.

Selected queries follow owned enumeration routes from the imported hierarchy. These routes contain names and positions, not native handles. They require unchanged source components. They skip unrelated subtrees. Adapter counters still cannot measure SST2's internal work.

## Certification is not a constant-time point operation

Qualified bit histories can certify continuous availability and seek a point. They then walk back through equal values to establish normalized change time. Certification can itself process history inside SST2. Unsupported layouts and projections of a non-scalar base use ordinary reading. An unqualified history need not put every qualified history in the same batch on that path.

Ordinary cold reads collect selected prefixes through the query end. They then merge these records for delivery. `Break` cannot stop collection that already completed. A prepared selection can reuse bounded owned state or a validated entering boundary. It does not cache native histories or handles. Events and incompatible requests keep the reference path. Callback streaming therefore does not bound total reader memory use.

A leading unavailable interval can remain Missing. A later unavailable recording interval is an error. It must not silently become a held physical value. Timed decode errors can allow delivery of earlier completed ticks before failure. Known native filesystem codes keep their I/O cause. Unknown SDK failures remain diagnostics, without guessed categories.
