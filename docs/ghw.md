# Native GHW reader

`ghw-native` is the built-in Rust reader for GHDL Waveform files. Automatic
opening and explicit selection use this reader. It accepts files and shared
`Arc<[u8]>` input. It needs no Cargo feature, native library, converter or temporary
file. The public API follows the [common model](model.md).

## Format boundary

GHW has no separate normative format standard. The executable reference is the
GHDL producer and reader at revision
[`2c451de76b07b73fd338c91d68fd5cefcb152512`](https://github.com/ghdl/ghdl/tree/2c451de76b07b73fd338c91d68fd5cefcb152512).
The Rust implementation uses the wire grammar and independent fixture evidence.
It does not link GHDL or use wellen at runtime.

The reader accepts uncompressed versions 0 and 1, with word/offset descriptors
4/1. Fixed fields and binary64 values use the declared
byte order. Signed and unsigned LEB128 fields have checked widths. Strings use
the source prefix encoding. Each string octet maps reversibly to Unicode U+0000
to U+00FF. The reader does not guess UTF-8 from byte contents.

Opening reads strings, types and hierarchy, then validates the complete body.
This pass establishes the first and last recorded ticks. `read_metadata` uses
the same full opening path and drops the reader. It is not a header-only operation.
Directory and tail sections are validated when present. Recorded footer offsets
do not drive query seeking. Clean EOF after a complete section is accepted;
partial fields, value lists and section terminators fail.

GHW does not encode time resolution. Ondas preserves integer ticks and returns
`None` for timescale. A conventional unit does not establish a physical duration.
The consumer needs a known scale before it can convert ticks to physical time. Missing writer, date and
definition-name information remains absent.

## Declaration and value mapping

| Source object | Ondas representation |
| --- | --- |
| Integer and physical scalar | Signed 32- or 64-bit `Bits`, according to source storage |
| Real scalar | `Real`, preserving binary64 bits |
| Bit, boolean and std_ulogic scalar | One bit; two or nine states from explicit source type markers |
| Other enumeration | Fixed-width ordinal bits with declaration labels and encoded keys |
| Integer-indexed one-dimensional bit or std_ulogic vector | One whole `Bits` signal with the source range |
| Other array, including boolean and character arrays | Unpacked array scopes and readable element leaves |
| Record | Unpacked record scope and readable fields |
| Process | Leaf process scope |

Constrained enum metadata keeps original base ordinals. Subsets do not renumber
their labels. Character leaves retain their declared labels, including Latin-1
octets. They do not become string values. Physical unit tables remain private;
queries return the source integer value without scaling.

Array layouts support multiple dimensions, enum and negative indices, mixed
directions, nested bounds, arrays of records and records of arrays. Unbounded
record aliases use their resolved field constraints. A null vector retains a
variable with no signal or bit range. A null non-vector array retains an empty
array scope. No value is invented for either object.

Names retain exact source case and punctuation, including VHDL extended
identifier delimiters. Use `HierarchyPath::from_components` for exact components.
Generate-for scopes retain their source iterator suffix. The reader does not
apply Verilog name normalization or infer signed vector types from type names.
Absence of changes does not make a declaration constant.

Whole signal identity uses the ordered source scalar IDs and common encoding.
Exact aliases share a signal. Reversed, subset, superset and overlapping recipes
remain distinct when their ordered recipes differ. Scalar-ID holes are resolved
through active ranks. They do not cause a repeated search through unused IDs.
Declaration metadata remains independent of shared signal identity.
The active source table resolves scalar type aliases once for body decoding.
Declaration types keep their original metadata.

## Query cost and storage

Opening can build an index of validated scalar payload positions and ticks.
The index shares source scalars across overlapping recipes. It keeps only the
last source payload at each tick. Files must remain unchanged while open.
Small selections read selected positions in chronological order through the
required prefix. Broad selections use sequential replay. A late window must
still establish earlier selected state. A narrow vector slice still requires
its whole source recipe.

Each query builds one sorted list of source scalar dependencies. Its size is
proportional to the selected recipe positions. Repeated positions for one source
scalar share first-observation tracking. A whole value remains missing until
all its recipe positions have been observed. Each touched whole value is
delivered once per change list during sequential replay. Indexed reads combine
the final source payloads from all lists at one tick before assembly.
The common query engine observes the final tick state and compares projections
before emitting changes. Equal ticks can span snapshot and cycle sections.

Broad replay maps each active source rank to its selected dependency range once
per query. This table avoids a source-ID search for every scalar payload. Its
size is proportional to the number of active source scalars. Small selections
keep sparse dependency lookup, including when the payload index is unavailable.

Explicit completed-tick notifications allow callback termination before values
at the next tick, including an unrelated-only suffix. Scans, candidates and
composed queries use the common engine. Callback values borrow current buffers;
owned traces retain their requested output.

The reader retains hierarchy, type layouts, ordered recipes and input buffers.
The optional payload index has a 64 MiB limit, including vector capacities.
Each nonempty source chain reserves at least eight entries at first. This avoids
repeated small allocations for short histories. Histories with few entries use
more capacity and can reach the index limit earlier.
If it reaches this limit or cannot allocate storage, opening still validates
the body and queries use sequential replay. Each indexed entry holds a source
tick and byte offset, not a decoded value. Index construction adds work to
opening. A fixed-size buffer batches wire reads and nearby indexed seeks retain
buffered bytes. Queries retain selected current values and common query state.
There is no retained value history, query checkpoint or continuation cache.
Benchmark warm cases distinguish a reused waveform from retained full values.

## Errors and limits

Detected invalid fields, ordinals, IDs, time order and incomplete sections return
`Error::Malformed` with a byte offset. I/O failures return `Error::Io`.
Unsupported versions, descriptors, type tags, hierarchy tags and layouts return
`Error::UnsupportedFeature` with a byte offset. This does not establish malformed data. Gzip and bzip2 outer
containers are unsupported. The reader does not decompress through a subprocess.

`ghw0002-ghdl-issue-538` omits required bounds for its unbounded inner vector.
The next TYP entry appears where a scalar range is required. It returns
`Error::Malformed`. The format boundary does not include arbitrary historical
producer grammars, access/file types or repair of broken recordings.
Derived empty layouts are limited to
1,000,000 scopes. This checked limit prevents a compact empty-element array from
creating an unbounded hierarchy. It returns an explicit unsupported-layout error.

## Evidence

[`tests/ghw_native.rs`](../tests/ghw_native.rs) checks the public API with independent
literal bytes. Private wire tests cover checked integers, tiny buffers, both byte
orders and versions, real payloads, deep constraints, aliases and section errors.
Test counters check selected storage, shared scalar decoding and early stopping.

The [fixture suite](fixtures.md) executes every GHW artifact in file/bytes modes.
All 102 have independent profiles from the pinned native GHDL reader: 98 positive
and four hash-bound malformed recordings. Three negative inputs end inside a
cycle; the fourth lacks nested bounds. Positive profiles remain sparse. They
assert only listed declarations and complete selected windows.

[`ghw_workload_checks.rs`](../tests/support/ghw_workload_checks.rs) checks source
bit order, issue-99 neighbors, nested raw/resolved vectors, numeric storage,
processes, nulls and exact names. It checks shared benchmark consumers against
authored values and independent complete windows. The named type coverage also
includes multidimensional arrays, p32, queryable i64, enum subsets and unbounded
record aliases. These extend the incomplete paths in pinned
[wellen 0.25.6](https://github.com/ekiwi/wellen/tree/34f40595c4330bcac364397679cdc3d5ae6b5c0c/src/ghw).
They do not imply support for every VHDL construct.

The [benchmark target](benchmarking.md) checks execution and ownership boundaries.
This implementation makes no performance comparison claim.
