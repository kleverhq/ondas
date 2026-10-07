# Library model

Ondas provides waveform observations through a read-only Rust API. The API is
independent of the source format. [Rustdoc](https://docs.rs/ondas) defines the public
contracts. This document explains the implementation boundaries.
The [architecture diagram](images/architecture.drawio.svg)
shows the public entities and private read/query path.

## Scope

The model covers metadata, hierarchy, value histories, static bit projections,
point and range observations, and repeated queries over selected signals.
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
[fst-lib](fst-lib.md), [GHW](ghw.md), [fsdb-lib](fsdb-lib.md) and [WLF](wlf.md), not in the common
model.

## Identity and ownership

A hierarchy is immutable and can be cloned. Scopes and variables borrow it. A
variable is a declaration. A signal identifies a history. Aliases can
share a history without losing their separate declaration metadata. Some
declarations have no queryable history. Known signedness and logic domain belong
to each declaration. They do not belong to the shared signal or stored bits.
Interpretation metadata remains absent when it is unavailable or does not apply.
Observed values do not establish this metadata.

A signal handle identifies a whole history or a static bit projection, not a
path or raw reader index. Validation includes waveform identity so a foreign
handle cannot accidentally select a local signal.

Hierarchy paths contain exact components. Escaping affects display, not identity.
Brackets, dots and whitespace inside source names are not traversal syntax.
Public hierarchy contracts define lookup spelling and slice-selector precedence.
A single variable lookup scans declarations without allocating a path index.
Subsequent lookups build a full index shared by hierarchy clones. Opening and
traversal do not allocate this index. Consumers that use repeated lookups pay this
cost once.

For VCD and FST, leading Verilog escape markers are separate from logical name
identity. Scopes and variables record whether the source name was escaped.
Consumers can preserve identifier spelling without adding the marker to the name. FSDB keeps
leading SDK backslashes in identity and separately retains the SDK declaration
spelling before extracting printed ranges.

## Time and observations

Time uses absolute source ticks. A timescale has an exact integer factor and unit.
Reader-local time indices and timestamp tables stay private. The model has no
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
