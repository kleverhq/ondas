use std::{
    fs::File,
    io::{BufReader, Cursor},
    ops::ControlFlow,
    path::Path,
    sync::Arc,
};

use crate::{
    Error, Hierarchy, Result, Sample, ScanRef, Selection, Signal, Time, TimeRange, TimeSpan,
    Timescale, Trace,
    backends::{Input, Reader, fst, vcd},
};

/// Opens a waveform file with an automatically selected available backend.
///
/// FST uses `fst-native`; VCD uses `vcd-native`. Content recognition takes
/// precedence over the filename; a recognized extension is a fallback hint. A recognized format without a
/// reader returns [`Error::NoBackend`]; unrecognized input returns
/// [`Error::UnknownFormat`]. Each opened waveform uses one backend.
///
/// File and decoder errors are returned where the decoder reports them. See
/// the crate-level reader limitations for malformed inputs that can panic.
pub fn open(path: impl AsRef<Path>) -> Result<Waveform> {
    open_file(path.as_ref(), None)
}

/// Opens a waveform file with only the named backend, without fallback.
///
/// Available names are `fst-native` and `vcd-native`. Unknown names return
/// [`Error::UnknownBackend`]; a format unsupported by the selected backend returns
/// [`Error::BackendDoesNotSupport`]. See [`open`] for detection and reader limits.
pub fn open_with(path: impl AsRef<Path>, backend: &str) -> Result<Waveform> {
    open_file(path.as_ref(), Some(backend))
}

/// Opens shared in-memory bytes with an automatically selected backend.
///
/// `name` is the logical [`Metadata::source_name`] and a format-detection hint.
/// The input stays alive through the shared ownership of `bytes`. Detection,
/// errors, and reader limitations are the same as [`open`]. Both native readers
/// support file and byte input; other readers need not support both input kinds.
pub fn open_bytes(name: impl Into<String>, bytes: Arc<[u8]>) -> Result<Waveform> {
    open_input(name.into(), Box::new(Cursor::new(bytes)), None)
}

/// Opens shared in-memory bytes with only the named backend, without fallback.
///
/// Combines [`open_bytes`]' input ownership with [`open_with`]'s explicit reader
/// selection and error categories.
pub fn open_bytes_with(
    name: impl Into<String>,
    bytes: Arc<[u8]>,
    backend: &str,
) -> Result<Waveform> {
    open_input(name.into(), Box::new(Cursor::new(bytes)), Some(backend))
}

fn check_backend(backend: Option<&str>) -> Result<()> {
    if let Some(backend) = backend
        && !matches!(backend, "fst-native" | "vcd-native")
    {
        return Err(Error::UnknownBackend {
            backend: backend.into(),
        });
    }
    Ok(())
}

fn open_file(path: &Path, backend: Option<&str>) -> Result<Waveform> {
    check_backend(backend)?;
    open_input(
        path.to_string_lossy().into_owned(),
        Box::new(BufReader::new(File::open(path)?)),
        backend,
    )
}

fn open_input(name: String, mut input: Box<dyn Input>, backend: Option<&str>) -> Result<Waveform> {
    check_backend(backend)?;
    let prefix = input.fill_buf()?;
    let mut detected = if matches!(prefix.first(), Some(0 | 254)) {
        Some(Format::Fst)
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
    let extension = Path::new(&name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let format = if let Some(format) = detected {
        format
    } else {
        match extension.as_str() {
            "fst" => Format::Fst,
            "vcd" => Format::Vcd,
            "ghw" => Format::Ghw,
            "fsdb" => Format::Fsdb,
            "wlf" => Format::Wlf,
            _ => return Err(Error::UnknownFormat),
        }
    };
    let (reader, hierarchy, metadata) = match (format, backend) {
        (Format::Fst, None | Some("fst-native")) => {
            let (reader, hierarchy, metadata) = fst::Reader::open(input, name)?;
            (Reader::Fst(Box::new(reader)), hierarchy, metadata)
        }
        (Format::Vcd, None | Some("vcd-native")) => {
            let (reader, hierarchy, metadata) = vcd::Reader::open(input, name)?;
            (Reader::Vcd(Box::new(reader)), hierarchy, metadata)
        }
        (_, Some(backend)) => {
            return Err(Error::BackendDoesNotSupport {
                backend: backend.into(),
                format,
            });
        }
        (_, None) => return Err(Error::NoBackend { format }),
    };
    Ok(Waveform {
        reader,
        hierarchy,
        metadata,
    })
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
}

/// An opened, read-only waveform and its query interface.
///
/// Holds one private reader and exposes format-independent types. Queries need
/// mutable access for reader state. A [`Selection`] reuses a validated ordered
/// signal list; one-shot methods have the same semantics as temporary selections.
/// Input order and duplicates remain visible; base-signal deduplication is private.
///
/// Handles from another source return [`Error::InvalidSignal`]. Querying an
/// unsupported value class returns [`Error::UnsupportedSignal`]. Ordinary decoder
/// errors are returned, but upstream panics are not intercepted; see crate docs.
pub struct Waveform {
    pub(crate) reader: Reader,
    hierarchy: Hierarchy,
    metadata: Metadata,
}

impl Waveform {
    /// Returns the detected source format.
    pub fn format(&self) -> Format {
        match self.reader {
            Reader::Vcd(_) => Format::Vcd,
            _ => Format::Fst,
        }
    }

    /// Returns the selected implementation's stable lower-kebab-case name.
    ///
    /// `fst-native` identifies the direct Rust FST reader, independently of the
    /// source format. Use this name for diagnostics or explicit opening.
    pub fn backend(&self) -> &str {
        match self.reader {
            Reader::Fst(_) => "fst-native",
            Reader::Vcd(_) => "vcd-native",
            #[cfg(test)]
            Reader::Memory { .. } => "memory",
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
    /// filtering, and same-signal ordering. Values are callback-scoped. A break
    /// stops successfully; a late reader error may follow earlier observations.
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
    /// are excluded; extra candidates are allowed. A break stops successfully.
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
    pub fn timescale(&self) -> Option<Timescale> {
        self.timescale
    }

    /// Returns the first and last recorded ticks when reliably available.
    ///
    /// `None` means an empty source or unavailable metadata. These bounds do not
    /// restrict queries: samples after EOF hold the last persistent value, while
    /// event counts are zero.
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
mod tests {
    use super::*;

    fn bytes(text: &[u8]) -> Arc<[u8]> {
        Arc::from(text)
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
        ] {
            let result = open_bytes(name, bytes(content));
            if format == Format::Vcd {
                assert!(
                    matches!(
                        result,
                        Err(Error::Malformed {
                            format: Format::Vcd,
                            ..
                        })
                    ),
                    "{name}"
                );
            } else {
                assert!(
                    matches!(result, Err(Error::NoBackend { format: actual }) if actual == format),
                    "{name}"
                );
            }
            assert!(
                matches!(open_bytes_with(name, bytes(content), "fst-native"), Err(Error::BackendDoesNotSupport { backend, format: actual }) if backend == "fst-native" && actual == format),
                "{name}"
            );
        }
        // Invalid explicit selection is rejected before attempting filesystem I/O.
        assert!(
            matches!(open_with("", "unknown-reader"), Err(Error::UnknownBackend { backend }) if backend == "unknown-reader")
        );
        assert!(matches!(open_with("", "fst-native"), Err(Error::Io(_))));
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
            open_bytes_with("dump.vcd", bytes(b"$scope module tb $end"), "fst-native"),
            Err(Error::BackendDoesNotSupport {
                format: Format::Vcd,
                ..
            })
        ));
        assert!(matches!(
            open_bytes_with("dump.fst", bytes(b""), "not-a-reader"),
            Err(Error::UnknownBackend { .. })
        ));
        assert!(matches!(
            open_bytes("empty.fst", bytes(b"")),
            Err(Error::Malformed {
                format: Format::Fst,
                ..
            })
        ));
    }
}
