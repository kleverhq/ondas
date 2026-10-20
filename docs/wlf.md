# WLF backend

The optional `wlf-lib` backend reads Questa and ModelSim WLF files through a
recent Questa SDK (2020+). It uses the [common model](model.md) for declarations,
samples, traces, scans, projections, aliases and composed queries. The SDK API
and handles stay private.

## Build and input

Enable Cargo feature `wlf-lib`. Set `QUESTA_HOME` to the SDK directory that contains
`include/wlf_api.h` and `linux_x86_64/libwlf.so`. The backend supports Linux x86-64
with the GNU toolchain. The build compiles an Ondas C11 shim into the crate and
links the vendor shared library. It requires the SDK headers, a C compiler and
`readelf`. The SDK library must have no ELF SONAME. Libraries with a SONAME
are not supported.

The executable records the vendor library's absolute path. Keep that installation
available at runtime. A missing library prevents process startup, including an
application that opens another waveform format. Rebuild after moving the SDK or
selecting another SDK version. Changing `QUESTA_HOME` after compilation does not
select another runtime. No separate Ondas shim shared library is needed.

The shim selects the active file with the exported `setCurrentNamespace` function
and clears the SDK's expression lookup pointers at owner boundaries. These
interfaces are absent from the SDK header. SV enum tables also require
a qualified private type-descriptor layout. Qualify each selected SDK with the
vendor gates below. A missing export fails at link time. A changed descriptor
layout or mode behavior requires a new compatibility check. The public SDK
API version alone does not establish compatibility.

Input is file-only. Bytes return `UnsupportedInput` without a temporary file.
Automatic detection uses the case-insensitive `.wlf` extension as a fallback.
Recognized FST and VCD contents take priority over the name. Extensionless WLF
content detection is not established. See the opening functions in Rustdoc for
backend selection and error categories.

## Mapping and limits

The reader preserves logical component names, scope and variable kinds, declared
ranges, constants, directions and verified alias identities. Unlogged objects
have no signal. Unsupported or incompatible value classes remain visible and
fail queries. Type names and aggregate packing stay absent when the SDK does not
establish them.

SV enum values use their declared bit width. The reader copies the complete
native table in declaration order, including variants absent from the history.
Keys preserve leading zeros, signed bit patterns and X/Z. Both table keys and
samples use the same native value-word conversion. Labels keep their source
spelling. No table is inferred from observed values or label positions. Ordinary
Bits reading does not require an enum table. An incompatible table stays absent.

Scalar VHDL logic types keep one bit and their represented logic states. Other
qualified VHDL enum types use their native ordinal storage width of 8, 16 or 32
bits. Each ordinal-to-label pair is checked with the native typed formatter.
Unqualified VHDL types remain unsupported. An array of logic elements has no
scalar enum table. SDK array elements are separate declarations with exact
combined components such as `states[0]`. An expanded array container has no
scalar signal; no aggregate packing is inferred.

For SDK-identified Verilog declarations, an escaped name with terminating
whitespace becomes a logical component without that syntax. Literal backslashes
remain. The variable retains the original spelling in `reader_name`. Names
without this verified syntax remain exact. VHDL names are not parsed as Verilog.
The `vcd2wlf` import can truncate escaped names and split a source scope. These
losses also appear in vendor exports; Ondas cannot reconstruct the source text.

Verified values include logic vectors with all nine represented states, native
binary64 reals, ASCII SystemVerilog strings and named events. Reals preserve
signed zero and NaN payloads. Non-ASCII bytes return `Error::NonAsciiString`,
regardless of UTF-8 validity. The current formatter transport does not support
embedded-NUL contents. The SDK's
string formatter supplies literal contents for the native string type; arbitrary
object display text is not treated as a string.

Queries use exact integer ticks. Multiple delta writes yield the final persistent
state at the tick. Event counts include native EVENT callbacks and exclude
STARTLOG and ENDLOG notifications. Counts reflect the recording and its writer
collapse settings. Initial logging establishes state; it is not an occurrence.
Delayed logging leaves a sample missing until the first recorded value. Persistent
values retain the last recorded state over a logging gap; this does not describe
unrecorded circuit activity. Events do not persist. Queries after the stored end
hold persistent state and return zero event occurrences.

Header metadata preserves the recorded interval, including zero-signal files.
A metadata-only read does not validate later hierarchy or value reads. Qualified
unclosed snapshots return `Malformed`. Other SDK read failures remain `Backend`;
SDK initialization failures return `BackendUnavailable`. A native SDK fault can
terminate the process; Ondas cannot convert it into a Rust error. Active simulation
reading is not supported. Files are never repaired, rewritten or converted implicitly.

## Ownership and verification

One private worker thread owns all WLF SDK objects and calls for the process
lifetime. Requests return owned data. Caller visitors run on the caller thread,
outside native callbacks, and need not be `Send`. Independent files can be
interleaved. File, pack, iterator and owned value resources are released on normal
completion, early break, error and caller panic.

The SDK uses one active time resolution. Before switching resolutions or reading
a new header, the worker closes every native owner. With no open files, the next
header selects the resolution. SDK initialization stays active for the process
lifetime. Normal SDK open makes native array elements available. The SDK can
retain expression lookup pointers into a closed file. The shim clears these
pointers before open and close, and selects the live file before reading or
releasing query resources. This also covers independent owners with the same
resolution. The worker closes its owned handles directly.
Logical sessions keep owned declarations and a small read position. A suspended
query reopens its file and replays from its query start, discarding already returned
completed-tick slots. This preserves event counts and pending caller state without
a history buffer.

Cold bit-only point queries first read a short range shared by the selected bases.
A projected change proved inside this range supplies the exact value and change
time. Unresolved projections share one chronological pass over their base signals.
The reader retains each projected current value and its last change time. A
constant projection can require all preceding source changes. This pass does not
collect a history or repeat native positioning for each previous change.

Cold bit-only windows with a nonzero start first read the state before their start,
then traverse the requested window. Other cold queries traverse from the recorded
beginning. Records skipped by a seek are not decoded or validated. A prepared
selection can reuse one bounded normalized state snapshot for compatible reads;
it does not retain a full history. Event selections keep chronological traversal.

The shim collects completed ticks into a bounded batch. It stops at a tick boundary
after 256 records or 64 KiB of value bytes; one larger complete tick can exceed
these limits. It keeps selected current values and event counts. The worker copies
the batch into one owned response. It delivers valid earlier records before a
later conversion error. An earlier visitor break
takes priority over that error. The SDK can keep its own caches. Caller-owned
traces are separate storage.

Use a worktree-local ignored SDK profile under the [automation policy](automation.md).
Keep SDK files, fixture payloads, credentials and local paths out of source and
images. Each build uses its selected SDK headers and library. SDK and
producer-format combinations need separate conformance checks; an API version
label is not an ABI guarantee. Keep SDK selection, test logs and independent
oracle producer details in ignored local records. See [testing](testing.md) and
[fixtures](fixtures.md) for oracle scope.

Run `./dev just conformance-wlf` and `./dev just ci-wlf` in the selected profile.
Select one SDK with `QUESTA_HOME` or several with `QUESTA_HOMES`. Each selected
SDK gets a separate build directory. See [local SDK selection](automation.md#proprietary-environments).
The vendor gates require installed payloads and fail on missing inputs. The
standalone consumer checks propagated native dependencies and launches outside
Cargo without its loader environment. Public CI remains free of WLF SDK inputs.
