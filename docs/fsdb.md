# FSDB Reader backend

`fsdb-lib` is an optional adapter to the Synopsys FSDB Reader SDK. Enable the Cargo
feature of the same name alongside the built-in VCD/FST readers. It implements
the [common waveform model](model.md), not a public vendor API. Reader selection,
input limitations and examples belong in the crate Rustdoc.

## Build and deployment boundary

The supported source-build target is `x86_64-unknown-linux-gnu`, with Verdi
2025+ as the tested SDK baseline. `VERDI_HOME` selects the installed Verdi
root in the filesystem where Cargo runs. A C++11 compiler, binutils `readelf`,
zlib development files and the corresponding C++ runtime are required. The build
uses `share/FsdbReader` headers and the SDK's `linux64` library directory
(`LINUX64` is also recognized). Missing inputs fail the feature-enabled build;
feature-disabled builds do not discover or link the SDK.

Only Ondas's own C++ source, private C ABI header, dependency-link anchors and Rust
adapter are distributed. SDK headers, libraries, manuals and fixtures are not
copied into the crate. Use and redistribution permissions remain the user's
responsibility; absence of a runtime license checkout is not a grant of rights.

The static shim links the installed Reader libraries dynamically. These SDK
libraries have no ELF `SONAME`; a generated linker script retains their absolute
paths in the final consumer's `DT_NEEDED` entries. A private two-address object
keeps `libnsys` and zlib from being discarded by lld's `--as-needed`: the Reader
library does not declare those dependencies itself. No vendor symbol contents
are read through these link anchors. SDK variants with `SONAME` are rejected
rather than silently depending on an unspecified loader search path.

This is deliberately a local source-build deployment model. The installed SDK
must remain available at the build-time path for the executable's lifetime.
Moving the SDK requires rebuilding. Removing it can prevent process startup
before Rust runs, including when the application intends to read only VCD/FST.
There is no graceful runtime-absence guarantee. An RPATH applied only to this
crate's own targets would not satisfy downstream Rust-library consumers.

Use an ignored local devcontainer profile to mount the full installation
read-only and set `VERDI_HOME` inside that container. Keep vendor build outputs
separate from ordinary public outputs. Mount fixture providers read-only too;
never add vendor files to an image or Docker build context. See
[automation](automation.md) for the local configuration boundary.

## Opening and hierarchy

The SDK opens paths, not streams. Ondas passes actual Unix path bytes, including
non-UTF-8 names, while `source_name` is a display string. Embedded NUL is rejected.
Bytes input is unsupported: no hidden temporary files, memory-file tricks or
FSDB-to-VCD conversion are used. Keep the source file unchanged while open.

Content recognition precedes filename extensions. The ordinary FST header marker
includes its section length, because a single initial zero byte also occurs in
FSDB. The enabled Reader probes otherwise unrecognized file contents through its
own API; case-insensitive extensions provide the fallback. Explicit selection
never switches backends after an opening failure.

Each waveform owns an `ffrOpenNonSharedObj` Reader. Tree callbacks populate owned
hierarchy descriptors before queries begin. Repeated identical declarations in
appended trees are coalesced; aliases at distinct paths retain their declarations
and share the SDK identity. Scope repeats may supply previously absent definition
names, but contradictory definitions fail. Record/struct members retain their
containing scopes; memory indices in declaration names are not query projections.

The adapter preserves explicit vector ranges, including `[0:0]`, and maps known
scope/variable kinds to canonical names. Unknown kinds remain namespaced.
Unsupported value declarations remain visible and fail query validation before
visiting values. Not every composite or user-defined SDK data type has a common
value decoder; transaction, assertion and object-database semantics are outside
the common model.

## Values, time and traversal

The C++ boundary normalizes SDK storage into the existing Bits, Real, String and
Event representations. Digital vectors preserve every bit and `x/z`; recognized
VHDL logic storage preserves all nine states. Float/double storage is copied
without alignment assumptions, with shortreal promoted to `f64`. String bytes
map reversibly to Unicode U+0000–U+00FF rather than guessing UTF-8. Event records
are occurrences, not persistent bit values. Reader callbacks alone must not be
interpreted as proof of physical event multiplicity during initialization or
periods where dumping was disabled.

Times remain unsigned absolute integer ticks. Floating timestamp formats are
rejected, not rounded. Scale factors are parsed as exact positive integers and
units, without floating-point conversion. Metadata bounds come from the file,
not from selected histories.

Queries select and load base identities, create the SDK chronological cursor and
walk from the beginning through the inclusive end. The shared query engine owns
entering-state, deduplication, projection and sample/trace semantics. The adapter
does not sort, deduplicate or cache complete histories. Equal-tick cross-signal
order is not a public guarantee. Vendor loading may allocate substantial memory
for selected histories even when the Rust visitor stops early.

## Ownership, threads and failures

SDK types and pointers stay behind the private C ABI. C++ exceptions are caught
at that boundary. Tree-callback failures are retained and reported after the SDK
returns; arbitrary Rust visitors are never invoked from C++.

All SDK operations, including open and destruction, are serialized within this
adapter. Non-shared objects avoid the SDK's shared-open identity semantics. The
Reader guide recommends this open API for multithreaded applications (Verdi
FSDB Reader 2025+, `ffrOpenNonSharedObj`, pages 104–105). The Rust owner can
move across threads; mutable query access owns its cursor exclusively. This lock
does not coordinate unrelated SDK users elsewhere in the process.

Transient values are copied into reusable Rust buffers while locked; the lock is
released before calling a Rust visitor. Reentrant callbacks can therefore query
a different waveform. Cursor cleanup runs on success, `Break`, error and Rust
panic, releasing traversal before unloading values and closing the file.

Indistinguishable SDK opening failures become backend errors, not invented
corruption diagnoses. The adapter does not suppress process-wide stdout/stderr:
SDK banners and warnings can remain visible. Native crashes, aborts and malformed
input robustness are not contained by C++ exception handling; this is not a
sandbox for untrusted files.

## Verification

`just conformance-fsdb` uses the real SDK and the common oracle runner in file
mode. Every selected public FSDB is mandatory; available private FSDBs are also
checked. `ONDAS_REQUIRE_PRIVATE_FIXTURES=1` requires private inputs instead of
permitting absence skips. Present-but-invalid inputs always fail. The private
lock and fixture policy are defined in [fixtures](fixtures.md).

Focused tests cover duplicate selections, slices, boundaries, early termination,
reentrant queries, panic cleanup, independent opens, thread movement, and path
handling. Feature-disabled tests check availability and unchanged native readers.
An independent downstream executable, launched directly without Cargo's loader
environment, checks deployment on development Rust and MSRV. Conformance asserts
only supplied oracle observations; unexplored value classes and omitted edge
cases must not be inferred from a green corpus count. Missing coverage is a
request to the fixture producer, never an invitation to record this adapter's
output as its own oracle. See [testing](testing.md).
