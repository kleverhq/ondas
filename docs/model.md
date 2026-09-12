# Library model

Ondas exposes waveform observations through a read-only, format-independent Rust
API. [Rustdoc](https://docs.rs/ondas) defines the public contracts; this document
explains the implementation boundaries. The [architecture diagram](images/architecture.drawio.svg)
shows the public entities and private read/query path.

## Scope

The model covers metadata, hierarchy, value histories, static bit projections,
point and range observations, and repeated queries over selected signals.
Writing, conversion, live ingestion, transactions, assertions, coverage, design
connectivity and asynchronous queries are outside its scope. A waveform is not a
complete HDL design database.

## Formats and readers

A format is a source representation; a backend is a reader implementation. One
reader can support several formats, and one format can have several readers.
Opening binds the waveform to one backend. Automatic selection uses deterministic
priority among compiled, available readers for the recognized format. Explicit
selection never falls back to another reader. Cargo features add implementations
rather than selecting mutually exclusive modes.

Backend names are stable lower-kebab-case strings. Concrete types, vendor
handles, callbacks, indices and reader-specific errors stay private. Selection
does not require a public backend enum, a generic waveform type or a plugin ABI.

Readers remain independent even when they share a dependency: changing one
format's reader must not require changes to another's. Native readers and
third-party adapters can coexist. Reader choices belong in [vcd-native](vcd-native.md),
[fst-native](fst-native.md), [GHW](ghw.md), [fsdb-lib](fsdb-lib.md) and [WLF](wlf.md), not in the common
model.

## Identity and ownership

A hierarchy is immutable and cloneable; scopes and variables borrow it. A
variable is a declaration, while a signal identifies a history. Aliases can
share a history without losing their separate declaration metadata. Some
declarations have no queryable history.

A signal handle identifies a whole history or a static bit projection, not a
path or raw reader index. Validation includes waveform identity so a foreign
handle cannot accidentally select a local signal.

Hierarchy paths contain exact components. Escaping affects display, not identity.
Brackets, dots and whitespace inside source names are not traversal syntax.
Public hierarchy contracts define lookup spelling and slice-selector precedence.

## Time and observations

Time uses absolute source ticks and an exact integer timescale factor/unit.
Reader-local time indices and timestamp tables stay private. The model has no
separate delta-cycle coordinate.

Persistent values establish state; events have multiplicity but no persistent
state. Point and range queries answer different questions about changes within
one tick. Entering state is separate from changes in the range: never invent a
`start - 1` timestamp. Missing state, empty history and query failure are distinct.

A bit projection reports changes to its observed value, not activity in discarded
bits. Declaration ranges retain HDL indices and direction; projections use
normalized value positions. Sharing base-signal reads must not change projection
identity or semantics.

## Queries and storage

A waveform owns source access and query state. A selection holds ordered signal
handles and reusable reader preparation. One-shot and prepared queries have the
same meaning; only their cost differs.

Owned observations can outlive queries. Callback views borrow reader buffers;
copying a view produces owned data, and a borrow cannot outlive its owner or
callback. Packed bits are an implementation choice, not a public string-storage
contract.

Candidate timestamps form a conservative activity index, not a decoded history.
Public query contracts define their ordering and allowances, along with event
multiplicity, range bounds and callback termination.

## Adapter responsibilities

Shared code owns path identity, projections, selection ordering and normalized
observations. Adapters decode sources, manage resources, convert metadata and
translate reader failures. Test pure conversions locally and adapters against
real artifacts.

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
