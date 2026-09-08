use crate::{Format, HierarchyPath, Signal};

/// An error encountered while parsing a hierarchy path.
///
/// Syntax diagnostics are separate from file and backend failures. The offset is
/// a byte position in the input, not a character index.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid hierarchy path at byte {offset}: {message}")]
pub struct PathError {
    /// The byte offset at which parsing failed.
    pub offset: usize,
    /// A description of the invalid syntax.
    pub message: String,
}

/// An error encountered while formatting a hierarchy path for Verilog.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum PathFormatError {
    /// A component has no lossless Verilog representation.
    #[error("path component cannot be represented in Verilog: {component:?}")]
    NotRepresentable {
        /// The component that cannot be represented.
        component: String,
    },
}

/// An error encountered while creating a signal bit slice.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SliceError {
    /// The signal is not a bit vector.
    #[error("only bit-vector signals can be sliced")]
    NotBits,

    /// The requested bounds are reversed (`msb < lsb`).
    #[error("invalid slice [{msb}:{lsb}]")]
    InvalidRange {
        /// The requested most-significant bit.
        msb: u32,
        /// The requested least-significant bit.
        lsb: u32,
    },

    /// The requested slice extends beyond the current signal or projection width.
    #[error("slice [{msb}:{lsb}] is outside signal width {width}")]
    OutOfBounds {
        /// The width of the signal.
        width: u32,
        /// The requested most-significant bit.
        msb: u32,
        /// The requested least-significant bit.
        lsb: u32,
    },
}

/// An error encountered while resolving hierarchy metadata.
///
/// Missing, ambiguous, and signal-less declarations are normal metadata
/// resolution outcomes, not backend failures. Invalid selectors retain separate
/// path and slice diagnostics rather than becoming [`Error::Backend`].
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum LookupError {
    /// The supplied hierarchy path is invalid.
    #[error(transparent)]
    InvalidPath(
        /// The path parsing error.
        #[from]
        PathError,
    ),

    /// No hierarchy item matches the path.
    #[error("hierarchy item was not found: {path}")]
    NotFound {
        /// The unresolved path.
        path: HierarchyPath,
    },

    /// More than one hierarchy item matches the path.
    #[error("hierarchy lookup is ambiguous ({matches} matches): {path}")]
    Ambiguous {
        /// The ambiguous path.
        path: HierarchyPath,
        /// The number of matching items.
        matches: usize,
    },

    /// The resolved variable has no queryable waveform signal.
    #[error("variable has no waveform signal: {path}")]
    NoSignal {
        /// The variable path.
        path: HierarchyPath,
    },

    /// The selector contains an invalid signal slice.
    #[error(transparent)]
    InvalidSlice(
        /// The slice error.
        #[from]
        SliceError,
    ),
}

/// An error encountered while opening or querying a waveform.
///
/// Categories distinguish recovery actions: I/O may require checking the path,
/// permissions, or filesystem; unavailable backends may require a runtime library
/// or license; unsupported formats and input kinds require a different reader.
/// [`LookupError`] and [`SliceError`] separately describe metadata resolution and
/// projection errors.
///
/// # Failure contract
///
/// Failures reported by a backend return this type. Upstream reader panics are
/// not intercepted; see the [reader limits](crate#reader-support-and-limits).
/// Empty results are not error sentinels: no known persistent state is [`Sample::Missing`](crate::Sample::Missing),
/// no event is an occurrence count of zero, and no changes is an empty trace
/// change list. Invalid handles and unsupported values remain errors.
///
/// A scan may have delivered observations before a late backend error. Owned
/// sample and trace queries return no partial result.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An input/output operation failed.
    #[error("I/O error: {0}")]
    Io(
        /// The underlying input/output error.
        #[from]
        std::io::Error,
    ),

    /// The waveform format could not be detected.
    #[error("unknown waveform format")]
    UnknownFormat,

    /// The requested backend name is unknown.
    #[error("unknown Ondas backend: {backend}")]
    UnknownBackend {
        /// The requested backend name.
        backend: String,
    },

    /// No available backend can read the waveform format.
    #[error("no available backend for {format:?}")]
    NoBackend {
        /// The detected waveform format.
        format: Format,
    },

    /// The selected backend is known but unavailable at runtime.
    ///
    /// For example, a required dependency, license, or vendor library is missing.
    #[error("backend {backend} is unavailable: {message}")]
    BackendUnavailable {
        /// The backend name.
        backend: String,
        /// The reason the backend is unavailable.
        message: String,
    },

    /// The explicitly selected backend cannot read the detected waveform format.
    #[error("backend {backend} does not support {format:?}")]
    BackendDoesNotSupport {
        /// The backend name.
        backend: String,
        /// The unsupported waveform format.
        format: Format,
    },

    /// The selected backend cannot read the supplied input kind.
    #[error("backend {backend} does not support {input:?} input")]
    UnsupportedInput {
        /// The backend name.
        backend: String,
        /// The unsupported input kind.
        input: InputKind,
    },

    /// The recognized waveform data is malformed.
    #[error("malformed {format:?} waveform read by {backend}: {message}")]
    Malformed {
        /// The waveform format.
        format: Format,
        /// The backend that read the waveform.
        backend: String,
        /// A description of the malformed data.
        message: String,
    },

    /// The signal belongs to a different hierarchy or waveform.
    #[error("signal does not belong to this waveform")]
    InvalidSignal {
        /// The invalid signal handle.
        signal: Signal,
    },

    /// The hierarchy exposes the signal, but its value class is unsupported for queries.
    #[error("signal encoding is not supported: {signal:?}")]
    UnsupportedSignal {
        /// The unsupported signal handle.
        signal: Signal,
    },

    /// The underlying reader failed in a way not covered by a more specific category.
    #[error("backend {backend} failed during {operation}: {message}")]
    Backend {
        /// The backend name.
        backend: String,
        /// The operation that failed.
        operation: &'static str,
        /// A description of the failure.
        message: String,
    },
}

/// The kind of input supplied to a waveform backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputKind {
    /// A filesystem path.
    File,
    /// An in-memory byte buffer.
    Bytes,
}
