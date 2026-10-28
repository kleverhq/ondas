use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    ops::ControlFlow,
    path::Path,
    sync::Arc,
};

mod composition;

use crate::{
    Error, Hierarchy, Result, Sample, ScanRef, Selection, Signal, Time, TimeRange, TimeSpan,
    Timescale, Trace,
    backends::{Input, Reader, fst, ghw, vcd},
};

/// Opens a waveform source with an automatically selected available backend.
///
/// FST uses `fst-lib`. VCD uses `vcd-native`. GHW uses `ghw-native`.
/// FSDB, WLF and WDB files use the optional `fsdb-lib`, `wlf-lib` and `wdb-lib` features.
/// SHM directories use the optional `shm-lib` feature. Content recognition takes
/// priority over the filename. A recognized extension is a fallback hint.
/// A recognized format without a reader returns [`Error::NoBackend`].
/// Unrecognized input returns
/// [`Error::UnknownFormat`]. Each opened waveform uses one backend.
///
/// File and decoder errors are returned where the decoder reports them. See
/// the crate-level reader limitations for malformed inputs that can panic.
pub fn open(path: impl AsRef<Path>) -> Result<Waveform> {
    open_file(path.as_ref(), None)
}

/// Reads owned source metadata without retaining a waveform or hierarchy.
///
/// FSDB, SHM, WLF and WDB read native source metadata without materializing declarations.
/// FST, VCD and GHW use their regular readers, so this is not a fast path for those
/// formats. Format detection matches [`open`], as do metadata values for sources
/// that open successfully. Unlike [`open`], it does not validate a vendor
/// declaration tree. Use [`open`] for validated hierarchy or signal access.
///
/// This function accepts filesystem paths, including SHM directories, not bytes.
pub fn read_metadata(path: impl AsRef<Path>) -> Result<Metadata> {
    let path = path.as_ref();
    let name = path.to_string_lossy().into_owned();
    if std::fs::metadata(path)?.is_dir() {
        let format = directory_format(path)?;
        #[cfg(feature = "shm-lib")]
        if format == Format::Shm {
            return crate::backends::shm::read_metadata(path, name);
        }
        return Err(Error::NoBackend { format });
    }
    let file = File::open(path)?;
    let vcd_file = file.try_clone().ok();
    let mut input: Box<dyn Input> = Box::new(BufReader::new(file));
    let format = detect_format(&mut *input, &name, Some(path))?;
    match format {
        Format::Fst => fst::Reader::open(input, name).map(|(_, _, metadata)| metadata),
        Format::Vcd => vcd::Reader::open(input, name, vcd_file).map(|(_, _, metadata)| metadata),
        Format::Ghw => ghw::Reader::open(input, name).map(|(_, _, metadata)| metadata),
        #[cfg(feature = "fsdb-lib")]
        Format::Fsdb => crate::backends::fsdb::read_metadata(path, name),
        #[cfg(feature = "shm-lib")]
        Format::Shm => crate::backends::shm::read_metadata(path, name),
        #[cfg(feature = "wlf-lib")]
        Format::Wlf => crate::backends::wlf::read_metadata(path, name),
        #[cfg(feature = "wdb-lib")]
        Format::Wdb => crate::backends::wdb::read_metadata(path, name),
        #[cfg(not(all(
            feature = "fsdb-lib",
            feature = "shm-lib",
            feature = "wlf-lib",
            feature = "wdb-lib"
        )))]
        _ => Err(Error::NoBackend { format }),
    }
}

/// Opens a waveform source with only the named backend, without fallback.
///
/// Available names are `fst-lib`, `vcd-native`, `ghw-native`, and feature-enabled
/// `fsdb-lib`, `shm-lib`, `wlf-lib` and `wdb-lib`. SHM input must be a database directory.
/// A disabled optional backend is an unknown name. Unknown names return
/// [`Error::UnknownBackend`]. A format unsupported by the selected backend returns
/// [`Error::BackendDoesNotSupport`]. See [`open`] for detection and reader limits.
pub fn open_with(path: impl AsRef<Path>, backend: &str) -> Result<Waveform> {
    open_file(path.as_ref(), Some(backend))
}

/// Opens shared in-memory bytes with an automatically selected backend.
///
/// `name` is the logical [`Metadata::source_name`] and a format-detection hint.
/// The input stays alive through the shared ownership of `bytes`. Detection,
/// errors, and reader limitations are the same as [`open`]. FST, VCD and GHW readers
/// support file and byte input. `fsdb-lib`, `wlf-lib` and `wdb-lib` read files only. `shm-lib`
/// reads directories only. An enabled FSDB, SHM, WLF or WDB filename hint returns
/// [`Error::UnsupportedInput`].
/// The SDK's content probe is only available for file paths.
pub fn open_bytes(name: impl Into<String>, bytes: Arc<[u8]>) -> Result<Waveform> {
    open_input(name.into(), Box::new(Cursor::new(bytes)), None, None, None)
}

/// Opens shared in-memory bytes with only the named backend, without fallback.
///
/// Combines [`open_bytes`]' input ownership with [`open_with`]'s explicit reader
/// selection and error categories. Selecting enabled `fsdb-lib`, `shm-lib`,
/// `wlf-lib` or `wdb-lib` returns [`Error::UnsupportedInput`] without creating a temporary file.
pub fn open_bytes_with(
    name: impl Into<String>,
    bytes: Arc<[u8]>,
    backend: &str,
) -> Result<Waveform> {
    open_input(
        name.into(),
        Box::new(Cursor::new(bytes)),
        Some(backend),
        None,
        None,
    )
}

fn check_backend(backend: Option<&str>) -> Result<()> {
    if let Some(backend) = backend
        && !matches!(backend, "fst-lib" | "vcd-native" | "ghw-native")
        && !(cfg!(feature = "fsdb-lib") && backend == "fsdb-lib")
        && !(cfg!(feature = "shm-lib") && backend == "shm-lib")
        && !(cfg!(feature = "wlf-lib") && backend == "wlf-lib")
        && !(cfg!(feature = "wdb-lib") && backend == "wdb-lib")
    {
        return Err(Error::UnknownBackend {
            backend: backend.into(),
        });
    }
    Ok(())
}

fn open_file(path: &Path, backend: Option<&str>) -> Result<Waveform> {
    check_backend(backend)?;
    if std::fs::metadata(path)?.is_dir() {
        let format = if backend == Some("shm-lib") {
            Format::Shm
        } else {
            directory_format(path)?
        };
        if let Some(backend) = backend
            && backend != "shm-lib"
        {
            return Err(Error::BackendDoesNotSupport {
                backend: backend.into(),
                format,
            });
        }
        #[cfg(feature = "shm-lib")]
        {
            let (reader, hierarchy, metadata) =
                crate::backends::shm::Reader::open(path, path.to_string_lossy().into_owned())?;
            return Ok(Waveform {
                compositions: composition::Compositions::default(),
                reader: Reader::Shm(Box::new(reader)),
                hierarchy,
                metadata,
            });
        }
        #[cfg(not(feature = "shm-lib"))]
        return Err(Error::NoBackend { format });
    }
    let file = File::open(path)?;
    let vcd_file = file.try_clone().ok();
    open_input(
        path.to_string_lossy().into_owned(),
        Box::new(BufReader::new(file)),
        backend,
        Some(path),
        vcd_file,
    )
}

fn open_input(
    name: String,
    mut input: Box<dyn Input>,
    backend: Option<&str>,
    _path: Option<&Path>,
    vcd_file: Option<File>,
) -> Result<Waveform> {
    check_backend(backend)?;
    #[cfg(feature = "wdb-lib")]
    if backend == Some("wdb-lib") && _path.is_none() {
        return Err(Error::UnsupportedInput {
            backend: "wdb-lib".into(),
            input: crate::InputKind::Bytes,
        });
    }

    #[cfg(feature = "wlf-lib")]
    if backend == Some("wlf-lib") && _path.is_none() {
        return Err(Error::UnsupportedInput {
            backend: "wlf-lib".into(),
            input: crate::InputKind::Bytes,
        });
    }
    #[cfg(feature = "fsdb-lib")]
    if backend == Some("fsdb-lib") && _path.is_none() {
        return Err(Error::UnsupportedInput {
            backend: "fsdb-lib".into(),
            input: crate::InputKind::Bytes,
        });
    }
    #[cfg(feature = "shm-lib")]
    if backend == Some("shm-lib") && _path.is_none() {
        return Err(Error::UnsupportedInput {
            backend: "shm-lib".into(),
            input: crate::InputKind::Bytes,
        });
    }
    let format = detect_format(&mut *input, &name, _path)?;
    let (reader, hierarchy, metadata) = match (format, backend) {
        (Format::Fst, None | Some("fst-lib")) => {
            let (reader, hierarchy, metadata) = fst::Reader::open(input, name)?;
            (Reader::Fst(Box::new(reader)), hierarchy, metadata)
        }
        (Format::Vcd, None | Some("vcd-native")) => {
            let (reader, hierarchy, metadata) = vcd::Reader::open(input, name, vcd_file)?;
            (Reader::Vcd(Box::new(reader)), hierarchy, metadata)
        }
        (Format::Ghw, None | Some("ghw-native")) => {
            let (reader, hierarchy, metadata) = ghw::Reader::open(input, name)?;
            (Reader::Ghw(Box::new(reader)), hierarchy, metadata)
        }
        #[cfg(feature = "fsdb-lib")]
        (Format::Fsdb, None | Some("fsdb-lib")) => {
            let path = _path.ok_or_else(|| Error::UnsupportedInput {
                backend: "fsdb-lib".into(),
                input: crate::InputKind::Bytes,
            })?;
            let (reader, hierarchy, metadata) = crate::backends::fsdb::Reader::open(path, name)?;
            (Reader::Fsdb(Box::new(reader)), hierarchy, metadata)
        }
        #[cfg(feature = "shm-lib")]
        (Format::Shm, None | Some("shm-lib")) => {
            let path = _path.ok_or_else(|| Error::UnsupportedInput {
                backend: "shm-lib".into(),
                input: crate::InputKind::Bytes,
            })?;
            let (reader, hierarchy, metadata) = crate::backends::shm::Reader::open(path, name)?;
            (Reader::Shm(Box::new(reader)), hierarchy, metadata)
        }
        #[cfg(feature = "wlf-lib")]
        (Format::Wlf, None | Some("wlf-lib")) => {
            let path = _path.ok_or_else(|| Error::UnsupportedInput {
                backend: "wlf-lib".into(),
                input: crate::InputKind::Bytes,
            })?;
            let (reader, hierarchy, metadata) = crate::backends::wlf::Reader::open(path, name)?;
            (Reader::Wlf(Box::new(reader)), hierarchy, metadata)
        }
        #[cfg(feature = "wdb-lib")]
        (Format::Wdb, None | Some("wdb-lib")) => {
            let path = _path.ok_or_else(|| Error::UnsupportedInput {
                backend: "wdb-lib".into(),
                input: crate::InputKind::Bytes,
            })?;
            let (reader, hierarchy, metadata) = crate::backends::wdb::Reader::open(path, name)?;
            (Reader::Wdb(Box::new(reader)), hierarchy, metadata)
        }
        (_, Some(backend)) => {
            return Err(Error::BackendDoesNotSupport {
                backend: backend.into(),
                format,
            });
        }
        #[cfg(not(all(
            feature = "fsdb-lib",
            feature = "shm-lib",
            feature = "wlf-lib",
            feature = "wdb-lib"
        )))]
        (_, None) => return Err(Error::NoBackend { format }),
    };
    Ok(Waveform {
        compositions: composition::Compositions::default(),
        reader,
        hierarchy,
        metadata,
    })
}

fn directory_format(path: &Path) -> Result<Format> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("shm"))
    {
        return Ok(Format::Shm);
    }
    // Confirmed SST2 DSN/TRN headers, independent of the directory suffix.
    let mut designs = 0;
    let mut transitions = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let component = entry.path();
        let expected = match component.extension().and_then(|s| s.to_str()) {
            Some("dsn") => [0xa1, 0x95, 0x11, 0xbf, 0, 0, 0, 1],
            Some("trn") => [0xa1, 0x95, 0x11, 0xbf, 0, 0, 0, 4],
            _ => continue,
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        let mut header = [0; 8];
        if File::open(component)?.read_exact(&mut header).is_ok() && header == expected {
            if expected[7] == 1 {
                designs += 1;
            } else {
                transitions += 1;
            }
        }
    }
    if designs == 1 && transitions > 0 {
        Ok(Format::Shm)
    } else {
        Err(Error::UnknownFormat)
    }
}

fn detect_format(input: &mut dyn Input, name: &str, _path: Option<&Path>) -> Result<Format> {
    let prefix = input.fill_buf()?;
    // The uncompressed FST header section is 329 bytes (fst-reader 0.17).
    // Its first zero byte alone also matches FSDB and is not a signature.
    let mut detected =
        if prefix.starts_with(&[0, 0, 0, 0, 0, 0, 0, 1, 73]) || prefix.first() == Some(&254) {
            Some(Format::Fst)
        } else if prefix.starts_with(b"Xilinx WAVE DATABASE ") {
            Some(Format::Wdb)
        } else if prefix.starts_with(b"GHDLwave") {
            Some(Format::Ghw)
        } else {
            None
        };
    if detected.is_none() {
        if prefix.starts_with(b"\xef\xbb\xbf") {
            input.consume(3);
        }
        loop {
            let prefix = input.fill_buf()?;
            if prefix.is_empty() {
                break;
            }
            if let Some(&byte) = prefix.iter().find(|b| !b.is_ascii_whitespace()) {
                if byte == b'$' {
                    detected = Some(Format::Vcd);
                }
                break;
            }
            let length = prefix.len();
            input.consume(length);
        }
    }
    input.rewind()?;
    #[cfg(feature = "fsdb-lib")]
    if detected.is_none()
        && let Some(path) = _path
        && crate::backends::fsdb::probe(path)?
    {
        detected = Some(Format::Fsdb);
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if let Some(format) = detected {
        Ok(format)
    } else {
        match extension.as_str() {
            "fst" => Ok(Format::Fst),
            "vcd" => Ok(Format::Vcd),
            "ghw" => Ok(Format::Ghw),
            "fsdb" => Ok(Format::Fsdb),
            "wlf" => Ok(Format::Wlf),
            "wdb" => Ok(Format::Wdb),
            "shm" => Ok(Format::Shm),
            _ => Err(Error::UnknownFormat),
        }
    }
}

/// A source format, distinct from the reader implementation.
///
/// A variant does not imply that its reader is compiled in or available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// Value Change Dump.
    Vcd,
    /// Fast Signal Trace.
    Fst,
    /// GHDL Waveform.
    Ghw,
    /// Fast Signal Database.
    Fsdb,
    /// Wave Log Format.
    Wlf,
    /// Waveform database.
    Wdb,
    /// Simulation History Manager database directory.
    Shm,
}

/// An opened, read-only waveform and its query interface.
///
/// Holds one private reader and exposes format-independent types. Queries need
/// mutable access for reader state. A [`Selection`] reuses a validated ordered
/// signal list. One-shot methods have the same semantics as temporary selections.
/// Input order and duplicates remain visible. Base-signal deduplication is private.
///
/// Handles from another source return [`Error::InvalidSignal`]. Querying an
/// unsupported value class returns [`Error::UnsupportedSignal`]. Ordinary decoder
/// errors are returned. Ondas does not intercept upstream panics. See crate docs.
///
/// # Example: resolve a declaration, sample a slice and read a trace
///
/// This example requires a file containing the named signal. Clone the hierarchy
/// before keeping a [`crate::Variable`] or [`crate::Scope`] view across mutable
/// waveform queries. These views borrow their hierarchy. [`Signal`] is copyable.
/// The clone lets the declaration view coexist with `&mut Waveform`.
///
/// [`crate::HierarchyPath`] handles exact names and escaping. After resolving the
/// signal, `slice(31, 28)` selects normalized value positions, not HDL indices.
/// The sample reports the value at one tick. The trace separates state before
/// the requested range from changes inside it.
///
/// ```no_run
/// use ondas::{HierarchyPath, Time, TimeRange};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut wave = ondas::open("dump.fst")?;
/// let hierarchy = wave.hierarchy().clone();
/// let path = HierarchyPath::parse(r"tb.\gen.blk[0] .data")?;
/// let declaration = hierarchy.variable_path(&path)?;
/// let data = hierarchy.signal_path(&path)?;
/// let opcode = data.slice(31, 28)?;
///
/// let sample = wave.sample(opcode, Time::from_ticks(1_000))?;
/// println!("{}: {sample:?}", declaration.name());
/// let trace = wave.trace(
///     opcode,
///     TimeRange::closed(Time::from_ticks(10_000), Time::from_ticks(20_000)),
/// )?;
/// if let Some(initial) = trace.initial() {
///     println!("entering: {:?}", initial.value());
/// }
/// for change in trace.changes() {
///     println!("{}: {:?}", change.time().ticks(), change.value());
/// }
/// # Ok(())
/// # }
/// ```
pub struct Waveform {
    pub(crate) reader: Reader,
    hierarchy: Hierarchy,
    metadata: Metadata,
    compositions: composition::Compositions,
}

impl Waveform {
    /// Returns the detected source format.
    pub fn format(&self) -> Format {
        match self.reader {
            Reader::Vcd(_) => Format::Vcd,
            Reader::Fst(_) => Format::Fst,
            Reader::Ghw(_) => Format::Ghw,
            #[cfg(feature = "fsdb-lib")]
            Reader::Fsdb(_) => Format::Fsdb,
            #[cfg(feature = "shm-lib")]
            Reader::Shm(_) => Format::Shm,
            #[cfg(feature = "wlf-lib")]
            Reader::Wlf(_) => Format::Wlf,
            #[cfg(feature = "wdb-lib")]
            Reader::Wdb(_) => Format::Wdb,
            #[cfg(test)]
            Reader::Memory { .. } | Reader::Generated(_) => Format::Fst,
        }
    }

    /// Returns the selected implementation's stable lower-kebab-case name.
    ///
    /// `fst-lib` identifies the `fst-reader` adapter, independently of the
    /// source format. Use this name for diagnostics or explicit opening.
    pub fn backend(&self) -> &str {
        match self.reader {
            Reader::Fst(_) => "fst-lib",
            Reader::Vcd(_) => "vcd-native",
            Reader::Ghw(_) => "ghw-native",
            #[cfg(feature = "fsdb-lib")]
            Reader::Fsdb(_) => "fsdb-lib",
            #[cfg(feature = "shm-lib")]
            Reader::Shm(_) => "shm-lib",
            #[cfg(feature = "wlf-lib")]
            Reader::Wlf(_) => "wlf-lib",
            #[cfg(feature = "wdb-lib")]
            Reader::Wdb(_) => "wdb-lib",
            #[cfg(test)]
            Reader::Memory { .. } => "memory",
            #[cfg(test)]
            Reader::Generated(_) => "generated",
        }
    }

    /// Returns source metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the immutable hierarchy.
    ///
    /// Clone it before retaining borrowed scopes or variables across mutable
    /// waveform queries. Handles from that clone still refer to this source.
    pub fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }

    /// Prepares an ordered signal list for repeated queries.
    ///
    /// Preserves duplicates and separate slices of a base signal. Invalid or
    /// unsupported handles fail before reading values. The returned selection
    /// holds a mutable waveform borrow, not a cache of every selected history.
    pub fn select(&mut self, signals: &[Signal]) -> Result<Selection<'_>> {
        Selection::new(self, signals)
    }

    /// Samples one signal at an absolute tick.
    ///
    /// See [`Sample`] for final-tick state, missing values, events, slices, and
    /// sampling after EOF. Equivalent to a one-entry selection, without a partial
    /// result on error.
    pub fn sample(&mut self, signal: Signal, time: Time) -> Result<Sample> {
        Ok(self
            .samples(&[signal], time)?
            .pop()
            .expect("one sample for one signal"))
    }

    /// Samples signals in input order, retaining duplicates.
    ///
    /// Equivalent to [`Selection::samples`]. Returns no partial vector on error.
    pub fn samples(&mut self, signals: &[Signal], time: Time) -> Result<Vec<Sample>> {
        self.select(signals)?.samples(time)
    }

    /// Samples selected signals at several explicitly requested ticks.
    ///
    /// Equivalent to [`Selection::samples_at`] on a temporary selection. Both
    /// time and signal order retain duplicates. Handles and encodings are
    /// validated even when `times` is empty. Returns no partial batch on error.
    pub fn samples_at(
        &mut self,
        signals: &[Signal],
        times: &[Time],
    ) -> Result<Vec<(Time, Vec<Sample>)>> {
        self.select(signals)?.samples_at(times)
    }

    /// Traces one signal over inclusive bounds.
    ///
    /// See [`Selection::scan`] for entering states, projections, same-tick order,
    /// and event multiplicity. Returns no partial trace on error.
    pub fn trace(&mut self, signal: Signal, range: TimeRange) -> Result<Trace> {
        Ok(self
            .traces(&[signal], range)?
            .pop()
            .expect("one trace for one signal"))
    }

    /// Traces signals in input order, retaining duplicates.
    ///
    /// Equivalent to [`Selection::traces`]. Returns no partial vector on error.
    pub fn traces(&mut self, signals: &[Signal], range: TimeRange) -> Result<Vec<Trace>> {
        self.select(signals)?.traces(range)
    }

    /// Visits entering states and changes over inclusive bounds.
    ///
    /// Equivalent to [`Selection::scan`], including duplicate entries, slice
    /// filtering, and same-signal ordering. Values are valid only during the callback.
    /// A break stops successfully. A late reader error may follow earlier observations.
    pub fn scan<B>(
        &mut self,
        signals: &[Signal],
        range: TimeRange,
        visitor: impl for<'v> FnMut(ScanRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.select(signals)?.scan(range, visitor)
    }

    /// Visits unique, increasing candidate change times over inclusive bounds.
    ///
    /// Equivalent to [`Selection::scan_candidate_times`]. Initial-state times
    /// are excluded. Extra candidates are allowed. A break stops successfully.
    pub fn scan_candidate_times<B>(
        &mut self,
        signals: &[Signal],
        range: TimeRange,
        visitor: impl FnMut(Time) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.select(signals)?.scan_candidate_times(range, visitor)
    }

    #[cfg(test)]
    pub(crate) fn memory(
        encodings: Vec<crate::Encoding>,
        mut records: Vec<(usize, u64, crate::Value)>,
        fail_after: Option<usize>,
    ) -> Self {
        records.sort_by_key(|(_, time, _)| *time);
        let span = records.first().zip(records.last()).map(|(first, last)| {
            TimeSpan::new(Time::from_ticks(first.1), Time::from_ticks(last.1))
        });
        Self {
            compositions: composition::Compositions::default(),
            hierarchy: Hierarchy::new(vec![], vec![], encodings),
            metadata: Metadata {
                source_name: "memory".into(),
                timescale: None,
                time_span: span,
                writer: None,
                date: None,
                comments: vec![],
            },
            reader: Reader::Memory {
                records: records
                    .into_iter()
                    .map(|(id, t, v)| (id, Time::from_ticks(t), v))
                    .collect(),
                fail_after,
                failure: None,
            },
        }
    }
}

/// Format-independent source metadata.
pub struct Metadata {
    pub(crate) source_name: String,
    pub(crate) timescale: Option<Timescale>,
    pub(crate) time_span: Option<TimeSpan>,
    pub(crate) writer: Option<String>,
    pub(crate) date: Option<String>,
    pub(crate) comments: Vec<String>,
}

impl Metadata {
    /// Returns the source path or caller-provided logical name.
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns the exact tick duration when reliably available.
    ///
    /// `None` means the source does not establish a supported physical scale.
    /// Keep raw [`Time`] ticks in that case; do not assume a unit for physical
    /// conversion. In particular, GHW does not store its time resolution.
    pub fn timescale(&self) -> Option<Timescale> {
        self.timescale
    }

    /// Returns the first and last recorded ticks when reliably available.
    ///
    /// `None` means an empty source or unavailable metadata. These bounds do not
    /// restrict queries. Samples after EOF hold the last persistent value.
    /// Event counts are zero.
    pub fn time_span(&self) -> Option<TimeSpan> {
        self.time_span
    }

    /// Returns the source-declared writer, if present.
    pub fn writer(&self) -> Option<&str> {
        self.writer.as_deref()
    }

    /// Returns the source-declared date, if present.
    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }

    /// Iterates over source comments in stored order.
    pub fn comments(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.comments.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod semantics_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &[u8]) -> Arc<[u8]> {
        Arc::from(text)
    }

    #[test]
    fn shm_routing_distinguishes_directories_and_bytes() {
        if cfg!(feature = "shm-lib") {
            assert!(matches!(
                open_bytes_with("unrecognized", bytes(b""), "shm-lib"),
                Err(Error::UnsupportedInput {
                    input: crate::InputKind::Bytes,
                    ..
                })
            ));
            assert!(matches!(
                open_bytes("dump.SHM", bytes(b"")),
                Err(Error::UnsupportedInput {
                    input: crate::InputKind::Bytes,
                    ..
                })
            ));
        } else {
            assert!(matches!(
                open_with("", "shm-lib"),
                Err(Error::UnknownBackend { .. })
            ));
            assert!(matches!(
                open_bytes("dump.SHM", bytes(b"")),
                Err(Error::NoBackend {
                    format: Format::Shm
                })
            ));
        }
        let wave = open_bytes(
            "dump.shm",
            bytes(b"$var wire 1 ! a $end $enddefinitions $end #0 1!"),
        )
        .unwrap();
        assert_eq!(wave.format(), Format::Vcd);

        let root = std::env::temp_dir().join(format!("ondas-shm-routing-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        // Own this newly created directory; remove it even if an assertion fails.
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        assert!(matches!(directory_format(&root), Err(Error::UnknownFormat)));
        std::fs::write(root.join("wave.dsn"), [0xa1, 0x95, 0x11, 0xbf, 0, 0, 0, 1]).unwrap();
        std::fs::write(root.join("wave.trn"), [0xa1, 0x95, 0x11, 0xbf, 0, 0, 0, 4]).unwrap();
        assert_eq!(directory_format(&root).unwrap(), Format::Shm);
        assert!(matches!(
            open_with(&root, "vcd-native"),
            Err(Error::BackendDoesNotSupport {
                format: Format::Shm,
                ..
            })
        ));
        #[cfg(not(feature = "shm-lib"))]
        assert!(matches!(
            open(&root),
            Err(Error::NoBackend {
                format: Format::Shm
            })
        ));
        std::fs::write(root.join("wave.trn"), b"invalid").unwrap();
        assert!(matches!(directory_format(&root), Err(Error::UnknownFormat)));
    }

    #[test]
    fn detection_prefers_content_and_falls_back_to_case_insensitive_extensions() {
        for (name, content, format) in [
            ("dump.FST", &b" \n$version test $end"[..], Format::Vcd),
            ("dump.vcd", &b"GHDLwave"[..], Format::Ghw),
            ("dump.VCD", &b""[..], Format::Vcd),
            ("dump.GHW", &b""[..], Format::Ghw),
            ("dump.FSDB", &b""[..], Format::Fsdb),
            ("dump.WLF", &b""[..], Format::Wlf),
            ("dump.WDB", &b""[..], Format::Wdb),
            ("neutral", &b"Xilinx WAVE DATABASE "[..], Format::Wdb),
            ("wrong.vcd", &b"Xilinx WAVE DATABASE "[..], Format::Wdb),
        ] {
            let result = open_bytes(name, bytes(content));
            if matches!(format, Format::Vcd | Format::Ghw) {
                assert!(
                    matches!(
                        result,
                        Err(Error::Malformed {
                            format: actual,
                            ..
                        }) if actual == format
                    ),
                    "{name}"
                );
            } else if (cfg!(feature = "fsdb-lib") && format == Format::Fsdb)
                || (cfg!(feature = "wlf-lib") && format == Format::Wlf)
                || (cfg!(feature = "wdb-lib") && format == Format::Wdb)
            {
                assert!(matches!(
                    result,
                    Err(Error::UnsupportedInput {
                        input: crate::InputKind::Bytes,
                        ..
                    })
                ));
            } else {
                assert!(
                    matches!(result, Err(Error::NoBackend { format: actual }) if actual == format),
                    "{name}"
                );
            }
            assert!(
                matches!(open_bytes_with(name, bytes(content), "fst-lib"), Err(Error::BackendDoesNotSupport { backend, format: actual }) if backend == "fst-lib" && actual == format),
                "{name}"
            );
        }
        // Invalid explicit selection is rejected before attempting filesystem I/O.
        assert!(
            matches!(open_with("", "unknown-reader"), Err(Error::UnknownBackend { backend }) if backend == "unknown-reader")
        );
        assert!(matches!(open_with("", "fst-lib"), Err(Error::Io(_))));
    }

    #[test]
    fn metadata_preserves_optional_fields_and_comment_order() {
        let mut wave = Waveform::memory(vec![], vec![], None);
        let metadata = wave.metadata();
        assert_eq!(metadata.source_name(), "memory");
        assert_eq!(metadata.timescale(), None);
        assert_eq!(metadata.time_span(), None);
        assert_eq!(metadata.writer(), None);
        assert_eq!(metadata.date(), None);
        assert_eq!(metadata.comments().len(), 0);
        wave.metadata = Metadata {
            source_name: "logical name".into(),
            timescale: Some(Timescale::new(10, crate::TimeUnit::Picosecond)),
            time_span: Some(TimeSpan::new(Time::from_ticks(7), Time::from_ticks(13))),
            writer: Some("writer".into()),
            date: Some("source date".into()),
            comments: vec!["second".into(), "".into(), "first".into(), "second".into()],
        };
        let metadata = wave.metadata();
        assert_eq!(metadata.source_name(), "logical name");
        assert_eq!(metadata.writer(), Some("writer"));
        assert_eq!(metadata.date(), Some("source date"));
        assert_eq!(metadata.timescale().unwrap().factor(), 10);
        assert_eq!(
            metadata.timescale().unwrap().unit(),
            crate::TimeUnit::Picosecond
        );
        assert_eq!(metadata.time_span().unwrap().first(), Time::from_ticks(7));
        assert_eq!(metadata.time_span().unwrap().last(), Time::from_ticks(13));
        let mut comments = metadata.comments();
        assert_eq!(comments.len(), 4);
        assert_eq!(comments.next(), Some("second"));
        assert_eq!(comments.len(), 3);
        assert_eq!(comments.collect::<Vec<_>>(), ["", "first", "second"]);
    }

    #[test]
    fn opening_errors_distinguish_detection_selection_and_io() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Waveform>();
        send_sync::<Hierarchy>();
        assert!(matches!(open(""), Err(Error::Io(_))));
        assert!(matches!(
            open_bytes("unknown", bytes(b"not a waveform")),
            Err(Error::UnknownFormat)
        ));
        assert!(matches!(
            open_bytes("dump.vcd", bytes(b"$timescale 1 ns $end")),
            Err(Error::Malformed {
                format: Format::Vcd,
                ..
            })
        ));
        assert!(matches!(
            open_bytes_with("dump.vcd", bytes(b"$scope module tb $end"), "fst-lib"),
            Err(Error::BackendDoesNotSupport {
                format: Format::Vcd,
                ..
            })
        ));
        for backend in ["not-a-reader", "fst-native"] {
            assert!(matches!(
                open_bytes_with("dump.fst", bytes(b""), backend),
                Err(Error::UnknownBackend { backend: actual }) if actual == backend
            ));
            assert!(matches!(
                open_with("", backend),
                Err(Error::UnknownBackend { backend: actual }) if actual == backend
            ));
        }
        assert!(matches!(
            open_bytes("empty.fst", bytes(b"")),
            Err(Error::Malformed {
                format: Format::Fst,
                ..
            })
        ));
    }
}
