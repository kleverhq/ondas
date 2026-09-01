#![doc = include_str!("../README.md")]
#![doc = r#"

## API overview

Ondas exposes waveform metadata, hierarchy, signal values, and time-based queries
through backend-independent public types. Use [`open`] or [`open_bytes`] for
automatic backend selection, or their `_with` variants to select a backend.
"#]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

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

fn stub<T>() -> T {
    unimplemented!("API skeleton")
}
