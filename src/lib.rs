#![doc = r#"
# Ondas

Ondas is a Rust library for reading waveform files and analyzing signal values.
Use it to browse a design's hierarchy, sample signals at a given time, or process
changes over a time range. The same API works across supported formats; the
library does not write or convert waveforms.

## Supported formats

A backend is the reader that opens a source and retrieves its recorded values.
Ondas uses one backend per opened waveform. Backend types, reader-specific
handles, native callbacks and format-local indices stay private.

| Format | Backend | Input | Availability |
|---|---|---|---|
| FST | `fst-lib` | Files and shared in-memory bytes | Included; uses `fst-reader` 0.17.0 |
| VCD | `vcd-native` | Files and shared in-memory bytes | Included; reads VCD directly without conversion |
| FSDB | `fsdb-lib` | Files only | Optional Cargo feature; requires a local FSDB Reader SDK |

Other formats have no reader in this release. The minimum supported Rust version
is 1.88. See [reader details](#reader-details) for format limits and FSDB setup.
In particular, the FST reader can panic on malformed input; it is not a hardened
parser for untrusted files.

## How the API fits together

Open a source, resolve its signals, then query their values. The public types
are available directly under `ondas`, for example `ondas::Waveform`; the source
modules are private.

| Type | What it represents |
|---|---|
| [`Waveform`] | An open source, its [`Metadata`], its hierarchy and its query interface. |
| [`Hierarchy`] | Immutable declarations. [`Scope`] groups declarations; [`Variable`] describes one declaration. |
| [`Signal`] | A handle to a signal's history, optionally sliced to fixed bit positions. Aliased declarations can share a signal. |
| [`Selection`] | An ordered signal list reused across queries. It mutably borrows its waveform and preserves order and duplicates. |
| [`Sample`], [`Trace`] | Owned query results: an observation at one tick, or state and changes over a range. |
| [`SampleRef`], [`ScanRef`], [`ValueRef`] | Borrowed views. Callback views cannot escape their callback; use [`ValueRef::to_owned`] when a value must be retained. |

Use [`open`] or [`open_bytes`] for automatic backend selection. Use [`open_with`]
or [`open_bytes_with`] to select a backend explicitly, without fallback.
[`Format`] describes source contents, not the reader implementation.

## Read a signal

This example opens a small VCD from memory, resolves `ready` by name and samples
it. The first recorded value is at tick 5, so the earlier sample has no known
state. It runs without an external waveform file.

```rust
use ondas::{Encoding, Sample, Time};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let data = b"$var wire 1 ! ready $end $enddefinitions $end #5 1!";
let mut wave = ondas::open_bytes_with("tiny.vcd", data.as_slice().into(), "vcd-native")?;
let ready = wave.hierarchy().signal("ready")?;
assert_eq!(ready.encoding(), Encoding::Bits { width: 1 });
assert!(matches!(wave.sample(ready, Time::from_ticks(4))?, Sample::Missing { .. }));
assert!(matches!(wave.sample(ready, Time::from_ticks(5))?, Sample::Value { .. }));
# Ok(())
# }
```

For a file, start with `ondas::open("dump.fst")?` instead. The [`Waveform`] example
also shows escaped hierarchy names, a bit slice and a trace over a bounded range.
File-based examples use `no_run` because they require the named signals.

[`HierarchyPath`] represents exact names and escaping. [`Hierarchy::signal`]
first looks up an exact variable path, then considers one trailing `[msb:lsb]`
slice. To separate name lookup from slicing, use [`Hierarchy::signal_path`]
followed by [`Signal::slice`]. Slice indices are normalized value positions,
not HDL indices.

## Choose a query

| Task | Operation |
|---|---|
| Read one signal at one tick | [`Waveform::sample`] |
| Read several signals at one tick | [`Waveform::samples`] or [`Selection::samples`] |
| Keep the state and changes over a range | [`Waveform::trace`], [`Waveform::traces`] or [`Selection::traces`] |
| Process changes without collecting a trace | [`Selection::scan`] or [`Selection::scan_each`] |
| Visit possible change times | [`Selection::scan_candidate_times`] |
| Check conditions at activity ticks and read only the requested samples | [`Selection::query`] |

Use an ordinary point or batch query when the observation time is already known.
A selection is useful when the same signal list is queried repeatedly.

For streamed output, [`Selection::scan_each`] identifies each record by its
position in the selection. Its example exports a bus, an alias and two slices,
then stops early.

For conditional sampling, [`Selection::query`] separates driver signals, which
determine candidate times, from the other signals the caller may read. Each
[`QueryContext`] can read its completed tick and the preceding tick, not arbitrary
history. Its example checks a transition and control value before copying a
payload. Conditions remain ordinary Rust code; the query does not collect a
candidate list or require reborrowing the waveform inside a callback.

## Time, values and errors

- [`Time`] counts absolute source ticks, not indices into a backend time table.
  Ranges are closed. A reversed bounded range is empty, not an error.
- A point sample reports the final persistent state at its tick. Delta cycles
  are not modeled. [`Sample::Missing`] means no persistent state is known at or
  before the requested time.
- An event sample counts reader-observed occurrences at exactly that tick,
  including zero. Scans and traces carry one positive aggregate per selection
  entry and tick. Aggregation cannot recover omitted events or expose intra-tick
  ordering.
- A scan separates state strictly before the range from changes inside it.
  Candidate-time scans return a strictly increasing superset of change times;
  initials and event multiplicity do not add candidate times. See
  [`Selection::scan`] and [`Selection::scan_candidate_times`] for the full contracts.
- [`PathError`], [`PathFormatError`], [`LookupError`] and [`SliceError`] describe
  path and metadata resolution errors. Opening and query failures use [`Error`].
  Reader-reported failures return errors, not empty-result sentinels.
- Scans and callback queries may have delivered observations before a later
  failure. Owned samples and traces return no partial collection.

## Reader details

### VCD

<details>
<summary>Validation, value representation and event limits</summary>

`vcd-native` validates the source with a full sequential pass when opening, then
replays selected signals for queries. It does not store full value histories.
Source files must remain unchanged while open.

- The reader preserves aliases, exact ranges, nine-state bits, real signed zero,
  strings and ordinary repeated events. Strings use reversible Latin-1 decoding,
  including escapes, NUL and padding. A real declaration with string records is
  classified as string storage before signal handles are returned.
- Persistent dump-block records apply at their recorded tick. Event records in
  `$dumpvars`, `$dumpall`, `$dumpoff` and `$dumpon` are snapshots, not occurrences.
  Mixed checkpoint/event ticks do not establish physical multiplicity. Resume
  records establish observed state, not the physical time of hidden changes.
- Unknown significant commands, incompatible aliases and malformed records fail
  opening. EVCD strength records and nonzero timezero are unsupported.

</details>

### FST

<details>
<summary>Reader behavior, declaration metadata and query costs</summary>

`fst-lib` is a direct, unmodified adapter to `fst-reader` 0.17.0. It reads bits,
reals, strings and event callbacks, preserving aliases and explicit source ranges.

- String bytes map reversibly to Unicode U+0000 to U+00FF (Latin-1), including NULs
  and padding. The adapter does not guess whether a byte sequence is UTF-8.
- Explicit FST SystemVerilog types can supply [`Variable::logic_domain`]. Explicit
  VHDL type attributes can also supply [`Variable::signedness`]. Legacy storage
  kinds and type-name text do not establish either field. The current VCD and
  FSDB adapters leave both fields unavailable.
- Some malformed inputs can make the underlying reader panic. Ondas does not
  intercept those panics. Ordinary reader errors and detected hierarchy
  inconsistencies return [`Error`].
- At the first recorded tick, event callbacks can include an initialization
  snapshot that the reader cannot distinguish from occurrences. Ondas preserves
  those callbacks; they do not establish exact physical event counts at that tick.

Selections reuse validated handles and base-signal grouping, not complete value
histories. Each query traverses selected histories from their beginning through
its end to establish state. Scans retain entering and pending final values per
selected entry and stop reader callbacks on `Break`; owned traces also retain
their output. The decoder owns input and decompression buffers, so this is not a
fixed bound on total memory use. Candidate-time scans decode values rather than
using a separate activity index. These costs do not change observation semantics.

</details>

### FSDB

<details>
<summary>SDK setup, supported values and runtime requirements</summary>

Enable the additive Cargo feature `fsdb-lib` and set `VERDI_HOME` to a local
Verdi installation when building.

- Use Verdi 2021 or newer on Linux x86_64 GNU. Building requires a C++11 compiler,
  binutils and zlib development files. Older SDKs can reject newer FSDB format
  versions; use a newer Reader for those files.
- Without the feature, Ondas does not discover an SDK and the backend name
  `fsdb-lib` returns [`Error::UnknownBackend`].
- The backend opens files only. Explicit byte input returns
  [`Error::UnsupportedInput`]; it creates no temporary file or conversion.

```no_run
# #[cfg(feature = "fsdb-lib")]
# fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut wave = ondas::open_with("dump.fsdb", "fsdb-lib")?;
let signal = wave.hierarchy().signal("tb.ready")?;
let sample = wave.sample(signal, ondas::Time::from_ticks(10))?;
# Ok(())
# }
```

Input files and the linked SDK installation must remain unchanged and available.
Binaries retain the build-time SDK library paths. Removing those libraries can
prevent the entire executable from starting, even for VCD/FST operations;
runtime SDK absence is not handled gracefully.

- Known digital storage preserves bits and source logic states. Real storage maps
  to `f64`; NUL-terminated strings use reversible Latin-1. Embedded-NUL string
  data and transaction events are unsupported.
- Normal HDL event records remain occurrences. No-change event initialization
  markers are not triggers. Unknown event records and event queries on
  SDK-reported dump-off files return errors. Callbacks describe recorded activity,
  not physical event counts during initialization or disabled dumping.
- Unsupported SDK data types retain their declarations with [`Encoding::Unsupported`].
  Integer ticks and scale factors remain exact. Floating timestamp formats are
  rejected rather than rounded.

Independent Reader objects and serialized SDK calls preserve `Waveform: Send + Sync`.
The lock is released before Rust visitors, so a callback can query another
waveform. Queries load selected histories and traverse from their beginning;
vendor loading has its own memory cost. SDK diagnostics may appear on stdout/stderr.
C++ exceptions become backend errors, but native crashes or aborts are not
contained. SDK permissions and runtime dependencies remain the caller's
responsibility. Ondas distributes no vendor files.

</details>

## Scope of the library

The API does not model writing or conversion, live ingestion, transactions,
assertions, coverage, design connectivity, async operations, relative hierarchy
paths, dynamic or multidimensional projections, or an expression language. There
is no public backend trait or plugin ABI, global timestamp table, strategy
selection, query capability flags, or format-specific downcasting.
"#]
#![warn(missing_docs)]
#![deny(unsafe_code)]

mod backends;
mod error;
mod hierarchy;
mod query;
mod time;
mod value;
mod waveform;

/// Errors and input kinds used by path, hierarchy, slicing, and waveform operations.
pub use error::{Error, InputKind, LookupError, PathError, PathFormatError, SliceError};
/// Types describing waveform hierarchy and queryable signals.
pub use hierarchy::{
    BitRange, Direction, Encoding, Enumeration, EnumerationVariant, Hierarchy, HierarchyPath, Item,
    LogicDomain, Packing, Scope, Signal, Signedness, Variable,
};
/// Owned and borrowed waveform query results.
pub use query::{Change, Initial, QueryContext, Sample, SampleRef, ScanRef, Selection, Trace};
/// Time values, ranges, spans, units, and scales.
pub use time::{Time, TimeRange, TimeSpan, TimeUnit, Timescale};
/// Owned and borrowed waveform signal values.
pub use value::{Bits, BitsRef, Logic, Value, ValueRef};
/// Waveform sources, metadata, formats, and opening functions.
pub use waveform::{Format, Metadata, Waveform, open, open_bytes, open_bytes_with, open_with};

/// A result returned by waveform opening and query operations.
pub type Result<T> = std::result::Result<T, Error>;
