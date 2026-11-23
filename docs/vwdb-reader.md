# VWDB recording selection and cursor ownership

`native/vwdb.cpp` and `src/backends/vwdb.rs` define the reader adapter. Ondas qualifies recordings by reading their values through the selected native ABI. The presence of the reader library alone gives no guarantee about a recording or a reader release.

## Select the recording directory

Open the directory that contains `vwdb_info.json` and the native design components. Older recordings can use a `.shm` suffix. The native layout takes priority over the suffix. A combined recording can contain a separate SHM directory. Some SystemC recordings store digital histories there. Select that directory with the SHM reader to access those histories, using its own time grid.

Native array containers are visible scopes. An element of a confirmed array has a typed index on its own component and shares the container's parent. Native names can contain literal brackets or backslashes. A VHDL architecture can have an empty root component. Its logical path starts with `""`. Declaration identity is local to an opened hierarchy. Each declaration has a separate signal handle because the reader has no qualified alias key.

## Keep the selected runtime separate

The build selects the reader under `XCELIUM_HOME`. The shim loads it at its canonical path on first use. Some native entry points have the same exported names as the SHM reader. A local, deep-bound load and handle-scoped symbol lookup keep each reader's calls on its own ABI. Keep the runtime at its build-time path. The shim needs a compatible system C++ runtime and zlib. A failed runtime load returns `BackendUnavailable`. The reader can write a diagnostic log in the working directory.

## Traverse selected histories in time order

Ondas copies the declaration tree and records native enumeration routes. A query opens the recording and resolves its selected declarations through those routes. Keep the recording unchanged while a waveform is open. Only one VWDB stream holds a native session at a time. Another VWDB operation can suspend it. On resumption the adapter reopens the recording and advances each cursor past its consumed observations. The source ordinal distinguishes writes that share a tick and sequence number.

The adapter merges cursors by tick and sends copied batches to the common normalizer. Rust invokes user callbacks with the SDK lock released. The first batch holds up to 64 records. Later batches hold up to 1024. Each batch stops after 64 KiB of copied payload, with one record allowed to cross the threshold. SDK history loading and native storage have separate costs. A cold request can traverse a prefix to establish state. Stops, errors and callback unwinding release cursors and unload histories.

## An unavailable interval removes the held value

Point and batch samples and query contexts report Missing while an explicit gap is active. After a gap, the first observed value establishes state with unknown change time. A scan or trace returns `UnsupportedFeature` if a selected persistent result loses known state inside the range. Its records carry values and have no representation for that transition. An earlier successful scan callback remains valid. An interval that starts after the loss can have an absent initial value.

The reader exposes recorded bit representations, qualified enum keys, binary64 reals, binary32 reals widened to binary64, and recorded event occurrences. Dynamic SystemVerilog strings can be absent from the inventory. A declared type or width reflects the recording's representation. Source-language types can have a different width. Unsupported value classes retain their declarations.
