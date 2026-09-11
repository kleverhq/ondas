#![doc = r#"
# Ondas

Read-only, format-independent waveform analysis.

The `fst-native` backend reads FST files and shared in-memory bytes using the
Rust `fst-reader` library. The independent `vcd-native` backend reads VCD directly,
without converting to FST or storing full value histories. The optional `fsdb-lib`
feature adds an FSDB Reader SDK backend of the same name. Other formats have no
reader in this release. File-based
examples use `no_run` because they need a waveform containing the named signals.
The minimum supported Rust version is 1.88.

## API overview

[`Waveform`] owns a source, its [`Metadata`], and an immutable [`Hierarchy`].
[`Scope`] and [`Variable`] describe declarations; [`Signal`] identifies a queryable
history, optionally projected to a static bit slice. Aliased declarations can
share one signal. [`Selection`] reuses an ordered signal list for repeated queries.
[`Sample`] and [`Trace`] own results; [`SampleRef`], [`ScanRef`], and [`ValueRef`]
provide borrowed views.

Use [`open`] or [`open_bytes`] for automatic backend selection, or [`open_with`]
and [`open_bytes_with`] to choose a backend without fallback. [`Format`] describes
source contents, not the reader implementation. Backend types, handles, callbacks,
and format-local indices do not form part of the public model.

## Resolve, sample, and trace

Clone the hierarchy before keeping declaration views across mutable waveform
queries. A [`Signal`] is copyable; a [`Scope`] or [`Variable`] borrows its hierarchy.
The clone lets those views coexist with `&mut Waveform` queries.

```no_run
use ondas::{HierarchyPath, Time, TimeRange};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut wave = ondas::open("dump.fst")?;
let hierarchy = wave.hierarchy().clone();
let path = HierarchyPath::parse(r"tb.\gen.blk[0] .data")?;
let declaration = hierarchy.variable_path(&path)?;
let data = hierarchy.signal_path(&path)?;
let opcode = data.slice(31, 28)?;

let sample = wave.sample(opcode, Time::from_ticks(1_000))?;
println!("{}: {sample:?}", declaration.name());
let trace = wave.trace(
    opcode,
    TimeRange::closed(Time::from_ticks(10_000), Time::from_ticks(20_000)),
)?;
if let Some(initial) = trace.initial() {
    println!("entering: {:?}", initial.value());
}
for change in trace.changes() {
    println!("{}: {:?}", change.time().ticks(), change.value());
}
# Ok(())
# }
```

[`HierarchyPath`] handles exact names and escaping. [`Hierarchy::signal`] first
looks up an exact variable path, then considers one trailing `[msb:lsb]` slice.
For unambiguous slicing, use [`Hierarchy::signal_path`] followed by
[`Signal::slice`]. Slice indices are normalized value positions, not HDL indices.

## Reuse a selection and retain borrowed values

Selection order and duplicates are preserved. A selection mutably borrows its
waveform; query through it until that borrow ends. Callback views cannot escape
the callback, so use [`ValueRef::to_owned`] to retain a value. Views obtained from
owned results instead live as long as the corresponding owner borrow permits.

```no_run
use std::ops::ControlFlow;
use ondas::{ScanRef, Time, TimeRange};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut wave = ondas::open_with("dump.fst", "fst-native")?;
let hierarchy = wave.hierarchy().clone();
let data = hierarchy.signal("tb.dut.data")?;
let low = data.slice(7, 0)?;
let mut selected = wave.select(&[data, low, low])?;
for ticks in [10, 20, 30] {
    let samples = selected.samples(Time::from_ticks(ticks))?;
    assert_eq!(samples.len(), 3);
}

let mut retained = Vec::new();
let outcome = selected.scan(TimeRange::all(), |record| {
    match record {
        ScanRef::Initial { value, .. } | ScanRef::Change { value, .. } => {
            retained.push(value.to_owned());
        }
        _ => {} // Public result enums are non-exhaustive.
    }
    ControlFlow::<()>::Continue(())
})?;
assert!(matches!(outcome, ControlFlow::Continue(())));
for value in &retained {
    println!("{:?}", value.as_ref()); // Borrows owned storage, not a callback buffer.
}
# Ok(())
# }
```

## Time, observations, and failures

[`Time`] is an absolute source tick, not a backend time-table index. Ranges are
closed; reversed bounded ranges are empty, not errors. Delta cycles are not
modeled. Point samples report the final persistent state at a tick; event samples
count occurrences at exactly that tick, including zero. [`Sample::Missing`] means
no persistent state is known at or before the sampled time.

[`Selection::scan`] separates state strictly before the range from actual changes
inside it. [`Selection::scan_candidate_times`] provides a strictly increasing
superset of change times, without initial states or event multiplicity. See those
methods for ordering, projection, and early-stop contracts.

[`PathError`], [`PathFormatError`], [`LookupError`], and [`SliceError`] distinguish
path and metadata resolution from opening and query [`Error`]s. File and backend
failures reported by the reader become errors, not empty-result sentinels. Scans
may already have called a visitor before a later error; owned queries return no
partial result. Reader-specific limitations are described below.

## Reader support and limits

`vcd-native` opens files and shared bytes with a full sequential validation pass,
then replays selected signals for queries. It preserves aliases, exact ranges,
nine-state bits, real signed zero, strings and ordinary repeated events. Source
files must remain unchanged while open. Strings use reversible Latin-1 decoding,
including escapes, NUL and padding. A real declaration with string records is
classified as string storage before handles are returned.

Persistent dump-block records are applied at their recorded tick. Event records
inside `$dumpvars`, `$dumpall`, `$dumpoff` and `$dumpon` are snapshots, not emitted
occurrences. Mixed checkpoint/event ticks do not establish physical multiplicity;
resume records establish observed state, not the physical time of hidden changes.
Unknown significant commands, incompatible aliases and malformed records fail
opening; EVCD strength records and nonzero timezero are unsupported.

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

`fst-native` is a direct, unmodified adapter to `fst-reader` 0.17.0. It reads bits,
reals, strings, and event callbacks, preserving aliases and explicit source ranges.
FST string bytes map reversibly to Unicode U+0000–U+00FF (Latin-1), including NULs
and padding; the adapter does not guess whether a byte sequence is UTF-8.

The underlying reader can panic on some malformed inputs. Those panics are not
intercepted; do not treat this release as a hardened parser for untrusted files.
Ordinary reader errors and detected hierarchy inconsistencies return [`Error`].
Event callbacks at the first recorded tick can include an initialization snapshot,
which the reader does not distinguish from occurrences. Ondas preserves these
callbacks; do not infer exact physical event counts at that initial tick.

Selections reuse validated handles and base-signal grouping, not complete value
histories. Each query traverses selected histories from the beginning through its
end to establish state reliably. Scans retain only the previous value per selected
entry and stop reader callbacks on `Break`; owned traces additionally retain their
output. The decoder also owns its input/decompression buffers, so this is not a
fixed bound on total memory use. Candidate-time scans decode values rather than using a separate activity
index. These are cost characteristics, not different observation semantics.

## Optional FSDB Reader

Enable the additive Cargo feature `fsdb-lib` and set `VERDI_HOME` to a local Verdi
installation when building. The supported target is Linux x86_64 GNU; Verdi
2025+ is the tested SDK baseline. Building requires a C++11 compiler, binutils
and zlib development files. No SDK discovery occurs without the feature.

```no_run
# #[cfg(feature = "fsdb-lib")]
# fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut wave = ondas::open_with("dump.fsdb", "fsdb-lib")?;
let signal = wave.hierarchy().signal("tb.ready")?;
let sample = wave.sample(signal, ondas::Time::from_ticks(10))?;
# Ok(())
# }
```

This backend opens files only. Explicit byte input returns [`Error::UnsupportedInput`];
there is no hidden temporary file or conversion. A disabled `fsdb-lib` name returns
[`Error::UnknownBackend`]. Input files and the linked SDK installation must remain
unchanged and available. Binaries retain the build-time SDK library paths;
removing those libraries can prevent the entire executable from starting, even
for VCD/FST operations. Runtime SDK absence is not handled gracefully.

Known digital storage preserves bits and source logic states; real storage maps
to `f64`, strings use reversible Latin-1, and event records remain occurrences.
Unsupported SDK data types retain their declarations with [`Encoding::Unsupported`].
Integer ticks and scale factors remain exact; floating timestamp formats are
rejected rather than rounded. Only recorded activity is observable: callbacks do
not prove physical event counts during initialization or disabled dumping.

Independent Reader objects and serialized SDK calls preserve `Waveform: Send + Sync`.
The lock is released before Rust visitors, permitting queries on another waveform
inside a callback. Queries load selected histories and traverse from their
beginning; vendor loading has its own memory cost. SDK diagnostics may appear on
stdout/stderr. C++ exceptions become backend errors, but native crashes or aborts
are not contained. SDK permissions and runtime dependencies remain the caller's
responsibility; no vendor files are distributed with Ondas.

## Model boundaries

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
    Packing, Scope, Signal, Variable,
};
/// Owned and borrowed waveform query results.
pub use query::{Change, Initial, Sample, SampleRef, ScanRef, Selection, Trace};
/// Time values, ranges, spans, units, and scales.
pub use time::{Time, TimeRange, TimeSpan, TimeUnit, Timescale};
/// Owned and borrowed waveform signal values.
pub use value::{Bits, BitsRef, Logic, Value, ValueRef};
/// Waveform sources, metadata, formats, and opening functions.
pub use waveform::{Format, Metadata, Waveform, open, open_bytes, open_bytes_with, open_with};

/// A result returned by waveform opening and query operations.
pub type Result<T> = std::result::Result<T, Error>;
