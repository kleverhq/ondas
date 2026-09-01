use std::{ops::ControlFlow, path::Path, sync::Arc};

use crate::{
    Hierarchy, Result, Sample, ScanRef, Selection, Signal, Time, TimeRange, TimeSpan, Timescale,
    Trace, stub,
};

/// Opens a waveform file using an automatically selected available backend.
pub fn open(_path: impl AsRef<Path>) -> Result<Waveform> {
    unimplemented!("backend implementation")
}

/// Opens a waveform file using only the named backend, without fallback.
pub fn open_with(_path: impl AsRef<Path>, _backend: &str) -> Result<Waveform> {
    unimplemented!("backend implementation")
}

/// Opens an in-memory waveform using an automatically selected available backend.
pub fn open_bytes(_name: impl Into<String>, _bytes: Arc<[u8]>) -> Result<Waveform> {
    unimplemented!("backend implementation")
}

/// Opens an in-memory waveform using only the named backend, without fallback.
pub fn open_bytes_with(
    _name: impl Into<String>,
    _bytes: Arc<[u8]>,
    _backend: &str,
) -> Result<Waveform> {
    unimplemented!("backend implementation")
}

/// A recognized waveform source format.
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

/// An opened, read-only waveform source and its query interface.
pub struct Waveform {
    _private: (),
}

impl Waveform {
    /// Returns the detected source format.
    pub fn format(&self) -> Format {
        unimplemented!("backend implementation")
    }

    /// Returns the selected backend's stable lower-kebab-case name.
    pub fn backend(&self) -> &str {
        unimplemented!("backend implementation")
    }

    /// Returns the source metadata.
    pub fn metadata(&self) -> &Metadata {
        unimplemented!("backend implementation")
    }

    /// Returns the immutable source hierarchy.
    pub fn hierarchy(&self) -> &Hierarchy {
        unimplemented!("backend implementation")
    }

    /// Prepares an ordered signal set for repeated queries.
    pub fn select(&mut self, _signals: &[Signal]) -> Result<Selection<'_>> {
        unimplemented!("backend implementation")
    }

    /// Samples one signal at an absolute tick.
    pub fn sample(&mut self, _signal: Signal, _time: Time) -> Result<Sample> {
        unimplemented!("backend implementation")
    }

    /// Samples signals at an absolute tick, preserving input order and duplicates.
    pub fn samples(&mut self, _signals: &[Signal], _time: Time) -> Result<Vec<Sample>> {
        unimplemented!("backend implementation")
    }

    /// Returns an owned trace for one signal over an inclusive range.
    pub fn trace(&mut self, _signal: Signal, _range: TimeRange) -> Result<Trace> {
        unimplemented!("backend implementation")
    }

    /// Returns owned traces in input order over an inclusive range.
    pub fn traces(&mut self, _signals: &[Signal], _range: TimeRange) -> Result<Vec<Trace>> {
        unimplemented!("backend implementation")
    }

    /// Visits entering states and changes for signals over an inclusive range.
    ///
    /// Returning [`ControlFlow::Break`] stops successfully; a later backend error may follow
    /// earlier visitor calls.
    pub fn scan<B>(
        &mut self,
        _signals: &[Signal],
        _range: TimeRange,
        _visitor: impl for<'v> FnMut(ScanRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        unimplemented!("backend implementation")
    }

    /// Visits strictly increasing candidate change times over an inclusive range.
    ///
    /// Candidates include every possible change time but may include extra times. Returning
    /// [`ControlFlow::Break`] stops successfully.
    pub fn scan_candidate_times<B>(
        &mut self,
        _signals: &[Signal],
        _range: TimeRange,
        _visitor: impl FnMut(Time) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        unimplemented!("backend implementation")
    }
}

/// Format-independent metadata associated with a waveform source.
pub struct Metadata {
    _private: (),
}

impl Metadata {
    /// Returns the source's path or caller-provided logical name.
    pub fn source_name(&self) -> &str {
        unimplemented!("backend implementation")
    }

    /// Returns the exact duration of one tick when reliably available.
    pub fn timescale(&self) -> Option<Timescale> {
        unimplemented!("backend implementation")
    }

    /// Returns the first and last recorded ticks when reliably available.
    pub fn time_span(&self) -> Option<TimeSpan> {
        unimplemented!("backend implementation")
    }

    /// Returns the source-declared writer, if present.
    pub fn writer(&self) -> Option<&str> {
        unimplemented!("backend implementation")
    }

    /// Returns the source-declared date, if present.
    pub fn date(&self) -> Option<&str> {
        unimplemented!("backend implementation")
    }

    /// Iterates over source comments in their stored order.
    pub fn comments(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        stub::<std::iter::Empty<&str>>()
    }
}
