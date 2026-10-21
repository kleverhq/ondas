use crate::{BitRange, Format, HierarchyPath, Signal, Time};

/// An error that occurs while parsing a hierarchy path.
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

/// An error that occurs while formatting a hierarchy path for Verilog.
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

/// An error that occurs while creating a signal bit slice.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SliceError {
    /// The HDL range width differs from the current signal width.
    #[error("HDL range {range:?} does not match signal width {width}")]
    HdlRangeWidthMismatch {
        /// The supplied HDL range.
        range: BitRange,
        /// The current signal width.
        width: u32,
    },
    /// The requested HDL indices extend beyond the supplied range.
    #[error("HDL slice [{msb}:{lsb}] is outside {range:?}")]
    HdlOutOfBounds {
        /// The supplied HDL range.
        range: BitRange,
        /// The requested left index.
        msb: i64,
        /// The requested right index.
        lsb: i64,
    },
    /// The requested HDL indices have the opposite direction.
    #[error("HDL slice [{msb}:{lsb}] has the opposite direction to {range:?}")]
    HdlDirectionMismatch {
        /// The supplied HDL range.
        range: BitRange,
        /// The requested left index.
        msb: i64,
        /// The requested right index.
        lsb: i64,
    },
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

/// An invalid request to construct a static bit composition.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CompositionError {
    /// No operands were supplied.
    #[error("a composition needs at least one part")]
    Empty,
    /// A part has zero width.
    #[error("composition part {part} has zero width")]
    EmptyPart {
        /// The zero-based operand position.
        part: usize,
        /// The rejected signal.
        signal: Signal,
    },
    /// A part is not a bit-vector signal.
    #[error("composition part {part} is not a bit vector")]
    NotBits {
        /// The zero-based operand position.
        part: usize,
        /// The rejected signal.
        signal: Signal,
    },
    /// The total width cannot fit in u32.
    #[error("composition width exceeds u32::MAX")]
    WidthOverflow,
}

/// An error that occurs while resolving hierarchy metadata.
///
/// Metadata lookup can find a missing or ambiguous declaration, or one without a
/// signal. These are normal lookup outcomes, not backend failures. Invalid
/// selectors return separate path and slice errors. They do not become
/// [`Error::Backend`].
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

/// An error that occurs while opening or querying a waveform.
///
/// Error categories help you choose a recovery action. For I/O errors, check the
/// path, permissions, or filesystem. An unavailable backend may need a runtime
/// library or license. Unsupported formats and input kinds need a different reader.
/// [`LookupError`] and [`SliceError`] separately describe metadata resolution and
/// projection errors.
///
/// # Failure contract
///
/// Failures reported by a backend return this type. Ondas does not intercept
/// upstream reader panics. See the [reader limits](crate#reader-details).
/// Empty results do not represent errors. If no persistent state is known, the
/// result is [`Sample::Missing`](crate::Sample::Missing). If no event is observed,
/// the occurrence count is zero. If no changes occur, the trace change list is empty. Invalid
/// handles and unsupported values remain errors.
///
/// A scan may have delivered observations before a late backend error. Owned
/// sample and trace queries return no partial result.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The static bit composition request is invalid.
    #[error(transparent)]
    InvalidComposition(#[from] CompositionError),
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

    /// A decoded string sample contains a byte outside ASCII (0 through 127).
    ///
    /// This is a value-support limit, not evidence of a malformed source.
    /// The failure can occur during opening or a query. Earlier callback
    /// observations remain valid; owned queries return no partial result.
    #[error("non-ASCII string at {time:?} in {format:?} waveform read by {backend}")]
    NonAsciiString {
        /// The source format.
        format: Format,
        /// The reader that decoded the value.
        backend: String,
        /// The source tick of the rejected value.
        time: Time,
    },

    /// A driver or readable subset index is outside its selection.
    #[error("selection index {index} is outside 0..{len}")]
    InvalidSelectionIndex {
        /// The rejected input position.
        index: usize,
        /// The number of entries in the selection.
        len: usize,
    },

    /// A query-context read is outside the current and preceding tick window.
    #[error(
        "cannot read {requested:?} in a query at {current:?}; only the current tick and its checked predecessor are available"
    )]
    InvalidQueryTime {
        /// The requested sample time.
        requested: Time,
        /// The current candidate time.
        current: Time,
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
