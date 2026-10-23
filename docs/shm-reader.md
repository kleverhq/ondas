# SHM recording selection and SST2 ownership

`native/shm.cpp`, `src/backends/shm.rs` and `tools/repo/shm_build.rs` define the reconstructed reader interface. Export discovery alone does not qualify it. Check the selected runtime with recorded inputs. Alias identity is optional. Separate handles can therefore describe the same source history.

## Runtime lookup can change the C++ runtime

SHM supports readers with and without the libsst2.so SONAME. A library without SONAME keeps its canonical absolute path in downstream dependencies. A SONAME library needs explicit runtime lookup. It must resolve to the selected build-time reader. Use a directory with only a symlink to that reader. The full vendor library directory can load an older bundled C++ runtime. Rebuild the application if you move the SDK. Missing native libraries can prevent startup before Rust selects an input format.

## A database directory is one recording selection

The base TRN and numbered components open in numeric order. SST2 selects the latest matching recording and joins its split intervals. It does not concatenate every design in a directory. Components that report No matching design are omitted. Other component errors fail. The exact DSN/TRN set is therefore part of fixture identity. `fixture-conformance.md` explains that identity.

Native names already contain logical components. These can include literal backslashes. Some VCD imports preserve an escape marker. The exposed language metadata cannot distinguish that marker from a native literal. General Verilog cleanup would corrupt names. Packed values and unpacked indexed elements also need proven layout. Name text alone cannot establish it.

Enum codes use their declaration's type for formatting. The adapter does not infer codes from label positions or observed occurrences. Quoted string transport supplies byte escapes, also for embedded zero. Malformed quoting is a transport error. It differs from a valid non-ASCII value that the common string contract rejects.

## Suspend shared native state before another operation

SST2 shares open files and a global time unit. One stream can retain its session and selected cursors between calls. All native access uses one process-wide mutex. Before another operation opens a session, the adapter destroys the active cursors and closes their session. The suspended stream retains owned values and completed-tick checkpoints. It restores its recording and seeks past those ticks when it needs more data. Repeated opens of the same directory follow this rule too. Hierarchy import copies enum tables. Rust visitors run after unlocking, with copied value bytes. They can read another waveform with a different scale. Interleaved reads can therefore add reopen, load and seek work.

Selected queries follow owned enumeration routes from the imported hierarchy. These routes contain names and positions, not native handles. They require unchanged source components. They skip unrelated subtrees. Adapter counters still cannot measure SST2's internal work.

## Certification is not a constant-time point operation

Qualified bit histories can certify continuous availability and seek a point. They then walk back through equal values to establish normalized change time. Certification can itself process history inside SST2. Unsupported layouts and projections of a non-scalar base use ordinary reading. An unqualified history need not put every qualified history in the same batch on that path.

Ordinary reads merge source cursors in time order. Each cursor folds a complete tick into one pending value or event count. Intermediate values are still checked. A history that moves backwards is an error; the adapter does not collect and sort that history. The first batch holds at most 64 merged records; later batches hold at most 1024 records. Each batch stops when retained payload capacity reaches 64 KiB. One value can cross that byte threshold. The adapter also owns one pending tick per selected history, a decoded replacement and an output record. The merge queue has at most one index per history. These buffers depend on selected histories and value sizes, not the number of source records or writes at one tick. No full fallback history is collected for an uncertified point.

`Break` releases the batch and cursors. It can leave one batch and pending history ticks already read. Completing one tick can process many source writes while retaining only its final value or event count. A prepared selection can reuse bounded owned state or a validated entering boundary. Cold reads can still traverse a prefix to establish state. SDK loading, histories and internal indices have separate costs; this is not a bound on total SDK memory. Stops, errors and Rust callback panics release native traversal state.

A leading unavailable interval can remain Missing. A later unavailable recording interval is an error. It must not silently become a held physical value. Timed decode errors can allow delivery of earlier completed ticks before failure. Known native filesystem codes keep their I/O cause. Unknown SDK failures remain diagnostics, without guessed categories.
