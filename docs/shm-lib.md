# SHM Lib backend

`shm-lib` reads recorded SHM database directories through Cadence's Xcelium
reader (2021+), `libsst2.so`. A private C++ shim calls the reader. The Rust adapter
copies declarations and observations into the common Ondas model. It does not
convert the source.

## Dependencies and deployment

Enable the additive Cargo feature `shm-lib`. Set `XCELIUM_HOME` to the installed
Xcelium root in the filesystem where Cargo runs. The build requires Linux x86_64
GNU, a C++17 compiler, binutils `nm`, and zlib development files.
It selects `tools/lib/64bit/libsst2.so` from that installation.
Feature-disabled builds do not inspect or link Xcelium.

The build checks required exports in the reconstructed private interface.
All supported readers use the quoted-value getter for strings. Alias identity
is optional. Readers without it keep declarations as separate signals.
Successful opening or symbol discovery alone does not establish reader
compatibility. Older readers can reject newer dumps.

The shim is a static archive. The selected vendor library is linked dynamically.
For libraries without an ELF `SONAME`, a generated linker script retains the
canonical absolute path in the application's dependencies. For libraries with
the `SONAME` `libsst2.so`, make the selected library available at runtime through
`LD_LIBRARY_PATH`, or set the application's runtime search path.
Use a directory with a symlink to that library to avoid selecting an older
bundled C++ runtime. The C++ runtime must support both the reader and application.
The runtime library must match the library selected at build time. Keep the
installation at that path. Rebuild the application if you move it. A missing
library can prevent process startup before Rust runs, including VCD/FST use.
Vendor libraries and documentation are not distributed with Ondas.

## Input and declarations

Input must be a complete database directory. Keep its DSN/TRN components
unchanged and available while the waveform is open. Individual components and
byte input are unsupported. Explicit selection does not try another reader.
Automatic detection uses a case-insensitive `.shm` suffix, or one DSN header and
at least one TRN header in a directory without a `.shm` suffix.

The directory must contain one DSN, a TRN with the same stem, and optional
TRNs with that stem followed by a positive numeric suffix. The adapter opens
the base TRN, then the numbered components in numeric order. SST2 selects the
latest matching recording and joins its split intervals. TRNs for another
design return `No matching design` and are omitted, as in the native directory
reader. Other component errors stop opening.

Scopes and variable names retain their exact components. Unpacked array elements
retain their indexed names. Packed values remain vectors. Confirmed aliases
share a signal, while their declarations remain separate. Without alias identity,
each declaration has its own signal. The reader can then read the same source
history more than once. It does not infer aliases from equal values. Unknown
declaration and scope kinds use `shm:decl:` and `shm:scope:` names.
Unsupported value types retain their declarations with `Encoding::Unsupported`.

Native recordings supply logical component names, including literal leading
backslashes. The reader preserves these names. Some VCD imports retain the VCD
escape marker in the stored name. The exposed language metadata does not
distinguish that marker from a literal backslash in a native name. The reader
does not guess or remove it. Use a faithful recording when comparing source
names across formats; conversion can lose source identifier information.

The adapter exposes HDL bit ranges, constants, scope definition names, exact
integer ticks, timescale, and source span. Direction is unknown. Type names,
signedness, logic-domain metadata, enumeration labels, writer, date, and comments
are unavailable. Metadata-only reads skip hierarchy import.

## Observations and ownership

Bits preserve all nine source logic states. Real values preserve binary64 bits,
including negative zero. Strings use a quoted representation with byte escapes.
The adapter decodes those escapes and preserves embedded zero bytes. It copies
packed indices with their type-proven width. Decoded strings must contain only
ASCII bytes. Other bytes return `Error::NonAsciiString`, including valid non-ASCII
UTF-8 and invalid UTF-8. Malformed quoted transport returns a reader error. Native empty-string
values map to the empty string. Event records are occurrences. The reader omits
the native untriggered initial event state. It preserves same-tick record order
per history.

SST2 shares open files and a global time unit. Each native operation opens the
base TRN and joins the numbered parts. If the base cannot open, it first opens
the directory and then retries the base. It copies the required data and closes
the source under a process-wide mutex.
No vendor handle or borrowed vendor buffer survives that operation. Rust
visitors run after the mutex is released. A visitor can query another waveform,
including one with a different timescale. `Waveform` remains `Send + Sync`.

Opening imports the full public hierarchy. Selected queries follow owned
enumeration routes from that import and check each selected declaration again.
They share common ancestors and skip unrelated subtrees. The routes contain
names and enumeration positions, not vendor handles. They require the unchanged
input described above.

Some bit-only points can certify continuous valid availability from the first
recorded state through the requested tick. A leading interval without a state
remains missing. The adapter then seeks each whole value and walks back through
equal values to find its normalized change time. It accepts only
qualified four-state scalar logic, wire, packed logic and integer layouts.
Selections with projections of a non-scalar base use the ordinary path.
In other bit-only selections, each unqualified history uses ordinary reading
and common normalization. It does not force qualified
histories in the same batch to collect their prefixes. Values remain
Ondas-owned, and all selected reads must succeed before sample callbacks run.
Certification can still process source history inside SST2. It does not promise
constant-time points.

An ordinary cold query reads selected histories from their beginning through the inclusive
end. The adapter merges ordered histories by tick, signal, and source sequence.
It uses a stable sort for a history that is not ordered. This establishes
entering state without a native seek. Memory use grows with that prefix.
`Break` stops delivery to the visitor after native collection has completed.
It does not stop the earlier native collection.

A prepared selection can retain a bounded normalized state snapshot after a
successful read. A compatible repeated point can use that state without another
native call. A successful window can also retain its entering state. A compatible
window read can seek to this validated boundary when all entering values are
known. A cold bit-only window can also obtain this boundary from certified
points. It still checks records and errors after the boundary. Other forward
reads traverse and validate the native prefix, but retain only records from the
saved boundary and earlier failures. Backward or incompatible reads use the full
path. Event selections do not retain these snapshots. Errors and early
termination discard them. This is selected state, not a native history or
vendor-handle cache.

An absent initial state remains missing. A later unavailable recording interval
returns an error. Timed decode errors let the common engine complete earlier
ticks before reporting the failure. Owned results return no partial success.
All owned traversal records are released after completion, `Break`, error, or a
visitor panic.

Use `./dev just conformance-shm` for the full fixture gate and
`./dev just shm-consumer` for a separate application launch without Cargo's
loader environment. For libraries with `SONAME`, the consumer sets the explicit
vendor search path and checks that it resolves the selected library. These
commands require the local reader and installed fixtures.

`ci-shm`, `conformance-shm`, `bench-smoke-shm` and `bench-shm` run once per path
in `XCELIUM_HOMES`, or once for `XCELIUM_HOME` when the list is empty or unset.
They isolate build output and expose the selected runtime library through a
symlink. The sweep fails if any selected SDK fails. It does not use a fixed
release list. See
[local SDK selection](automation.md#proprietary-environments).
