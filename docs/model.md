# Library model

Ondas provides waveform observations through a read-only Rust API. The API is
independent of the source format. [Rustdoc](https://docs.rs/ondas) defines the public
contracts. This document explains the implementation boundaries.
The [architecture diagram](images/architecture.drawio.svg)
shows the public entities and private read/query path.

## Scope

The model covers metadata, hierarchy, value histories, static bit projections,
point and range observations, static bit compositions, and repeated queries over
selected signals.
Writing, conversion, live ingestion, transactions, assertions, coverage, design
connectivity and asynchronous queries are outside its scope. A waveform is not a
complete HDL design database.

## Formats and readers

A format is a source representation. A backend is a reader implementation. One
reader can support several formats, and one format can have several readers.
Opening assigns one backend to the waveform. Automatic selection uses a fixed
priority among compiled, available readers for the recognized format. Explicit
selection never falls back to another reader. Cargo features add implementations.
They do not select mutually exclusive modes.

Backend names are stable lower-kebab-case strings. Concrete types, vendor
handles, callbacks, indices and reader-specific errors stay private. Selection
does not require a public backend enum, a generic waveform type or a plugin ABI.

Readers remain independent even when they share a dependency: changing one
format's reader must not require changes to another's. Native readers and
third-party adapters can coexist. Reader choices belong in [vcd-native](vcd-native.md),
[fst-lib](fst-lib.md), [GHW](ghw.md), [fsdb-lib](fsdb-lib.md), [shm-lib](shm-lib.md)
and [WLF](wlf.md), not in the common
model.

## Identity and ownership

A hierarchy is immutable and can be cloned. Scopes and variables borrow it. A
variable is a declaration. A signal identifies a history. Confirmed
aliases share a signal without losing their separate declaration metadata.
Scope and variable equality identifies an object in one source hierarchy.
Equal paths or metadata do not make objects equal. Alias declarations remain
distinct even when they share a signal. A hierarchy clone keeps object identity;
an independent open creates different objects, including when it reads the same
file. Parent and child relationships refer to objects, so scopes with the same
path keep their own children.
If the reader cannot establish an alias relationship, each declaration has a
separate signal. Different handles do not prove independent histories. Some
declarations have no queryable history. Known signedness and logic domain belong
to each declaration. They do not belong to the shared signal or stored bits.
Interpretation metadata remains absent when it is unavailable or does not apply.
Observed values do not establish this metadata.

Bit-vector equality compares width and each logic state. Owned values and
borrowed views use the same rule, independent of storage. Leading zeros are part
of the width. Unknown and don't-care states compare as distinct states, not
wildcards. Numerical interpretation and width extension remain caller policy.

Enum metadata is a mapping from whole stored bits to reader-provided labels.
Each exposed key is a `BitsRef` with the whole signal width and the same bit
positions as queried values. Callers can inspect logic states without formatting
text. `to_owned()` retains an independent `Bits` value.
The reader leaves the mapping absent when it cannot establish that relation. It
does not infer padding, bit reversal or a domain from the keys. Labels remain
separate from string samples.

String samples preserve ASCII bytes exactly where the source transport supports
them. Non-ASCII values return a typed error. Decoding transport escapes does not
choose a text encoding. This restriction does not apply to names, enum labels or
diagnostics.

A signal handle identifies a source history, a computed bit composition or a
static bit projection, not a path or raw reader index. Validation includes waveform identity so a foreign
handle cannot accidentally select a local signal.

Hierarchy paths contain exact components. Escaping affects display, not identity.
Brackets, dots and whitespace inside source names are not traversal syntax.
Public hierarchy contracts define lookup spelling and slice-selector precedence.
A single variable lookup scans declarations without allocating a path index.
Subsequent lookups build a full index shared by hierarchy clones. Opening and
traversal do not allocate this index. Consumers that use repeated lookups pay this
cost once.

Logical names exclude recognized Verilog escape syntax. A literal backslash in
the identifier remains part of the component. Readers use their declaration
metadata and verified spelling rules to separate syntax from data. They do not
remove characters only because the name looks escaped. Scopes and variables
record an escape marker when the reader retains that syntax. A reader that
already supplies logical names can lack the original spelling.

`reader_name` is optional diagnostic text before name adaptation. It can contain
escape syntax or printed ranges. Consumers use `name` and `path` for addressing;
they do not parse reader spelling. A conversion can lose source names or metadata
before Ondas opens the result. The reader cannot reconstruct missing information.

## Time and observations

Time uses absolute source ticks. A timescale has an exact integer factor and unit.
An unknown duration remains absent. A convention does not establish a physical
scale. Keep raw ticks when the duration is unknown; physical conversion needs a
known scale. Reader-local time indices and timestamp tables stay private. The model has no
separate delta-cycle coordinate.

Persistent values establish state. Events have occurrence counts but no persistent
state. Point and range queries use final recorded states at each source tick.
A persistent slot has at most one net change per tick. If its value changes and
returns to its entering representation, its change time does not advance. The
first recorded state is observable even when the value is HDL unknown. Entering
state is separate from changes in the range. Never invent a
`start - 1` timestamp. Missing state, empty history and query failure are distinct.

A bit projection reports changes to its observed value, not activity in discarded
bits. Declaration ranges retain HDL indices and direction; projections use
normalized value positions. Sharing base-signal reads must not change projection
identity or semantics.

## Static bit compositions

A composition joins ordered bit signals from one waveform. Its parts occupy
contiguous positions, from most significant to least significant, and retain
their internal bit order. Parts can be slices or earlier compositions. Repeated
parts retain each placement. The waveform owns immutable definitions and reuses
a handle for an identical ordered definition. Different definitions do not become
equal because their observed values match. A one-part request keeps its handle.

A computed signal creates no declaration, path or alias. Source hierarchy and
interpretation metadata stay unchanged. Automatic reconstruction requires proof
of both source membership and bit mapping. Similar names, adjacent ranges and
equal values do not provide that proof.

If any part has an observation, the composition retains its bits and fills
Missing parts with X. An observed all-X part counts as observed. If all parts are
Missing, the result is Missing. Slices select from the assembled value. Thus a
part outside a result slice can establish that slice's observation. Preserve
these dependencies through nested compositions; replacing a result slice with
source slices can change availability. Direct source reads still distinguish
Missing and recorded X. Errors do not become X.

The common engine evaluates selected definitions in dependency order after each
source tick is complete. It compares computed values and projections with their
own entering state. Part timestamps alone do not establish a computed change
time. State depends on all reachable source and computed operands, their
definitions and value widths, not history length. A narrow selected result can
retain wide or deeply nested dependencies.
Definition storage lasts until waveform drop. Reader and SDK memory remain
separate. Computed selections use chronological state establishment; source-only
point and prefix shortcuts do not establish their change times.

Declaration ranges preserve original signed indices. The checked range helper
maps the right endpoint to normalized position zero and validates width, bounds
and direction. It accepts an explicitly chosen coordinate range; it does not
prove that the range belongs to a declaration. Use the selected declaration's
metadata for source HDL indices. Missing range metadata stays absent.

## Queries and storage

A waveform owns source access and query state. A selection holds ordered signal
handles and reusable reader preparation. One-shot and prepared queries have the
same meaning. Only their cost differs. A composed query separates drivers that
advance candidate time from entries the caller can read. One context owns access
to the completed candidate tick and the preceding tick. The caller defines the
conditions and output policy. Non-driver updates between candidates still contribute to
sampled state. Exact-tick event counts do not persist across gaps.

Owned observations can outlive queries. Callback views borrow query data.
Copying a view produces owned data. A borrow cannot outlive its owner or
callback. Packed bits are an implementation choice, not a public string-storage
contract.

Candidate timestamps identify possible activity. They can include extra ticks
and do not represent a decoded history.
Public query contracts define their ordering and allowances, along with event
multiplicity, range bounds and callback termination.

Explicit-time batches use the same completed states as point reads, including
quiet ticks. The library schedules the requested times together. Output retains
both time order and selection positions, including duplicates. The owned batch
is complete or an error; decoding selected data may fail at intermediate ticks.
The current batch path reads the source prefix through the latest requested
tick. It does not use point-read seek shortcuts or cached replay.
Its schedule grows with the time list, while retained working values depend on
the selection and composition dependencies. Returned values and reader/SDK
storage are separate costs.

A saved trace owns entering state and normalized changes. A lookup borrows that
storage without reading the source. Finite coverage includes both requested
bounds; a reversed range covers nothing. An unbounded trace supports ordinary
EOF sampling from its start, including when the start is after recorded EOF.
A known entering state remains available even without in-window changes. Outside
coverage is a typed error, distinct from an in-coverage Missing value. Events
remain exact-tick counts. The first persistent observation does not establish a
known transition time; its record time and the sample's change time are separate.

## Compatibility boundary

Reader optimizations may change traversal, indexing or internal storage. They must
preserve the public rules for representation identity, final tick state, event multiplicity,
selection positions, supported observation times, lifetimes or error/stop
propagation. Readers may return different extra candidates within the documented
contract. Consumers must still confirm their conditions. Optional metadata and
change-time precision may improve only when evidence supports the change. Additional query
state remains distinct from input, decoder/index, SDK and caller-output residency.

Conditions, numerical interpretation and output policy remain caller code. There
is no public backend plugin interface, raw/final mode, expression runtime or
implicit promise of arbitrary-time sampling inside a callback context.

## Adapter responsibilities

Shared code owns path identity, projections, selection ordering and normalized
observations. Adapters decode sources, manage resources, convert metadata and
translate reader failures. The shared sequential traversal keeps an entering
state, a pending final value and an event count per selected entry. It visits a
completed tick before committing that state. It finalizes only entries touched
at that tick and traverses the prefix once for all operands. This additional
state excludes memory used by the input, decoder, index and SDK. Variable-sized
values contribute their own size. Test pure
conversions locally and adapters against real artifacts.

Keep known metadata and represent absent or unsupported information explicitly.
Do not invent ranges, replace missing values with zero or turn failures into empty
results. Unknown vendor declaration kinds use namespaced strings rather than a
closed cross-language enum.

Errors distinguish lookup mistakes, invalid handles, unsupported encodings,
unrecognized or malformed sources, unavailable readers and operational failures.
A callback may observe partial results before failure; owned queries return a
result only on success.

Add internal abstractions only for existing readers or tests. Hypothetical
strategy flags, capability APIs, downcasts, expression languages and backend
frameworks do not belong here. See [testing](testing.md) for correctness checks
and [benchmarking](benchmarking.md) for performance comparisons.
