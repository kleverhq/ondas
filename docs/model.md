# Library Model

Ondas presents the common observable part of waveform data through a read-only,
format-independent Rust API. The public types and their detailed contracts live
in [rustdoc](https://docs.rs/ondas); this document owns the conceptual boundaries
used to implement them.

## Scope

The model covers source metadata, hierarchy declarations, queryable value
histories, static bit projections, point observations, range observations, and
repeated queries over a selected signal set.

Writing or converting waveforms, live ingestion, transactions, assertions,
coverage, design connectivity, and asynchronous queries are outside this model.
A waveform database is not a complete HDL design database.

## Formats and Implementations

A format identifies the source representation. A backend identifies the reader
implementation. These are separate dimensions: a reader can support several
formats, and a format can have several readers.

Opening binds a waveform to one backend. Automatic selection considers the
recognized format and compiled, runtime-available readers in deterministic
priority order. Explicit selection exercises only the named reader, without
fallback. Cargo features add implementations rather than selecting mutually
exclusive modes.

Backend identities are stable lower-kebab-case strings. Concrete backend types,
vendor handles, callbacks, indices, and library-specific errors stay private.
Neither a public backend enum nor a generic waveform parameter is needed to let
callers select a reader. Backend selection is not a public plugin ABI.

Backend integrations are independent. Sharing an underlying reader library does
not couple formats: replacing or adding a reader for one format must not require
changing readers for another. A native implementation and a third-party adapter
can coexist behind the same public contracts.

Concrete reader choices and integration constraints belong in the format documents:
[VCD](vcd.md), [FST](fst.md), [GHW](ghw.md), [FSDB](fsdb.md), and [WLF](wlf.md).
The common model does not prescribe a reader library for any format.

## Identity and Ownership

A hierarchy is immutable and cloneable. Scopes and variables are borrowed views
of that hierarchy. A variable is a declaration; a signal is a queryable history.
Several alias declarations can name the same history, while a declaration can
also have no queryable history.

A signal handle identifies a whole history or a static bit projection of it. It
is not a declaration path or a raw backend index. Handle validation must retain
waveform identity, so an index from another waveform cannot accidentally select a
local signal. Aliases share whole-signal identity without losing their separate
declaration metadata.

Hierarchy paths are sequences of exact components. String escaping is a
presentation concern, not identity. Source-language brackets, dots, or whitespace
inside names must not be reinterpreted as traversal structure. Lookup syntax and
slice-selector precedence belong to the public hierarchy contracts.

## Time and Observations

Time is expressed in absolute source ticks. Exact timescales retain an integer
factor and unit rather than floating-point seconds. Backend-local time indices
and global timestamp tables do not become public API.

Persistent values and event occurrences are different kinds of observation.
Persistent histories establish state; events have multiplicity but no persistent
state. A point observation and a range history answer different questions about
changes within the same tick. Separate delta-cycle identifiers are not part of
the model.

Entering state is represented separately from changes in a queried range. The
implementation must not invent a timestamp such as `start - 1`. Missing state,
an empty history, and a query failure are distinct outcomes.

Bit projections describe observed values, not merely activity in the underlying
storage. A change to a discarded bit must not become a projected value change.
Declaration ranges retain HDL indices and direction; projections use normalized
value positions. Shared loading of several projections is an optimization, not
a change to their public identities or observation semantics.

## Queries and Storage

A waveform owns source access and query state. A selection holds an ordered set
of signal handles together with reusable backend preparation. One-shot and
selection queries have the same meaning; preparation only changes cost.

Owned observations serve simple callers and long-term storage. Borrowed views
allow callback-based processing without copying every backend buffer. Copying a
view creates an owned value; borrowed data must never outlive its owner or
callback lifetime. Packed bit representations remain an implementation choice,
not a string-based public storage contract.

Candidate timestamps provide an activity index rather than decoded value
history. Their conservative nature permits cheaper queries without changing the
meaning of samples or traces. Exact ordering, multiplicity, range bounds, and
callback termination rules are specified on the public query types and methods.

## Adaptation Boundary

Shared library code owns path identity, projection semantics, selection ordering,
and normalized observations. Reader adaptation owns source decoding, resource
lifetime, metadata conversion, and translation of reader failures into Ondas
errors. Pure conversions can be tested locally; actual adapters require real
artifacts.

Preserve metadata when it is known, and represent absence or unsupported value
classes explicitly. Do not invent declaration ranges, convert missing values to
zero, or hide read failures behind empty results. Unknown vendor declaration
kinds use namespaced strings instead of forcing a closed cross-language enum.

Errors distinguish lookup mistakes, invalid handles, unsupported encodings,
unrecognized or malformed sources, unavailable readers, and operational reader
failures. These categories let callers choose different recovery actions.
Partial callback observation is distinct from an owned result returned only on
success.

Keep internal boundaries proportional to concrete readers and tests. Do not add
strategy flags, capability APIs, format downcasts, expression languages, or a
backend framework for hypothetical extensions. Correctness is exercised through
the public API using the approach in [testing](testing.md); performance decisions
use [controlled measurements](benchmarking.md).
