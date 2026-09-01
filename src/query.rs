use std::{marker::PhantomData, ops::ControlFlow};

use crate::{Hierarchy, Result, Signal, Time, TimeRange, Value, ValueRef, Waveform};

/// A reusable, ordered selection of waveform signals.
pub struct Selection<'w> {
    _waveform: PhantomData<&'w mut Waveform>,
}

impl Selection<'_> {
    /// Returns the selected signals in input order, including duplicates.
    pub fn signals(&self) -> &[Signal] {
        unimplemented!("query implementation")
    }

    /// Returns the hierarchy associated with the selected waveform.
    pub fn hierarchy(&self) -> &Hierarchy {
        unimplemented!("query implementation")
    }

    /// Returns an owned sample for each selected signal at `time`.
    pub fn samples(&mut self, _time: Time) -> Result<Vec<Sample>> {
        unimplemented!("query implementation")
    }

    /// Returns an owned trace for each selected signal over `range`.
    pub fn traces(&mut self, _range: TimeRange) -> Result<Vec<Trace>> {
        unimplemented!("query implementation")
    }

    /// Visits a borrowed sample for each signal in selection order at `time`.
    ///
    /// Breaking from the visitor ends the operation successfully.
    pub fn visit_samples<B>(
        &mut self,
        _time: Time,
        _visitor: impl for<'v> FnMut(SampleRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        unimplemented!("query implementation")
    }

    /// Visits entering states, then time-ordered changes, in inclusive `range`.
    ///
    /// Breaking from the visitor ends the operation successfully.
    pub fn scan<B>(
        &mut self,
        _range: TimeRange,
        _visitor: impl for<'v> FnMut(ScanRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        unimplemented!("query implementation")
    }

    /// Visits the strictly increasing union of candidate change times in `range`.
    ///
    /// Candidates may include times with no observed change. Breaking succeeds.
    pub fn scan_candidate_times<B>(
        &mut self,
        _range: TimeRange,
        _visitor: impl FnMut(Time) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        unimplemented!("query implementation")
    }
}

/// An owned value or event sample for a signal at one timestamp.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Sample {
    /// No persistent value is known at or before the sampled time.
    Missing {
        /// The sampled signal.
        signal: Signal,
    },
    /// The final persistent value after all changes at the sampled time.
    Value {
        /// The sampled signal.
        signal: Signal,
        /// The sampled value.
        value: Value,
        /// The time that established this observed value, if known.
        changed_at: Option<Time>,
    },
    /// The number of event occurrences at the sampled time.
    Event {
        /// The sampled event signal.
        signal: Signal,
        /// The exact occurrence count, which may be zero.
        occurrences: u64,
    },
}

/// A borrowed value or event sample for a signal at one timestamp.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum SampleRef<'a> {
    /// No persistent value is known at or before the sampled time.
    Missing {
        /// The sampled signal.
        signal: Signal,
    },
    /// The final persistent value after all changes at the sampled time.
    Value {
        /// The sampled signal.
        signal: Signal,
        /// The borrowed sampled value.
        value: ValueRef<'a>,
        /// The time that established this observed value, if known.
        changed_at: Option<Time>,
    },
    /// The number of event occurrences at the sampled time.
    Event {
        /// The sampled event signal.
        signal: Signal,
        /// The exact occurrence count, which may be zero.
        occurrences: u64,
    },
}

impl Sample {
    /// Returns the sampled signal.
    pub fn signal(&self) -> Signal {
        unimplemented!("query implementation")
    }

    /// Borrows this sample without copying its owned value.
    pub fn as_ref(&self) -> SampleRef<'_> {
        unimplemented!("query implementation")
    }
}

impl SampleRef<'_> {
    /// Returns the sampled signal.
    pub fn signal(self) -> Signal {
        unimplemented!("query implementation")
    }
}

/// A borrowed record emitted by a range scan.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ScanRef<'a> {
    /// A persistent state established before the range start.
    Initial {
        /// The signal whose entering state is reported.
        signal: Signal,
        /// The borrowed entering value.
        value: ValueRef<'a>,
        /// The time that established the value, if known.
        changed_at: Option<Time>,
    },
    /// A value change or event occurrence within the range.
    Change {
        /// The signal that changed or produced the event.
        signal: Signal,
        /// The change or occurrence time.
        time: Time,
        /// The new value, or [`ValueRef::Event`] for one occurrence.
        value: ValueRef<'a>,
    },
}

/// An owned persistent state entering a trace range.
pub struct Initial {
    _private: (),
}

/// An owned value change or event occurrence within a trace range.
pub struct Change {
    _private: (),
}

/// An owned range scan for one signal.
pub struct Trace {
    _private: (),
}

impl Initial {
    /// Returns the entering value.
    pub fn value(&self) -> ValueRef<'_> {
        unimplemented!("query implementation")
    }

    /// Returns the time that established the entering value, if known.
    pub fn changed_at(&self) -> Option<Time> {
        unimplemented!("query implementation")
    }
}

impl Change {
    /// Returns the change or event occurrence time.
    pub fn time(&self) -> Time {
        unimplemented!("query implementation")
    }

    /// Returns the new value, or [`ValueRef::Event`] for an occurrence.
    pub fn value(&self) -> ValueRef<'_> {
        unimplemented!("query implementation")
    }
}

impl Trace {
    /// Returns the traced signal.
    pub fn signal(&self) -> Signal {
        unimplemented!("query implementation")
    }

    /// Returns the inclusive range covered by this trace.
    pub fn range(&self) -> TimeRange {
        unimplemented!("query implementation")
    }

    /// Returns the persistent state established before the range, if any.
    pub fn initial(&self) -> Option<&Initial> {
        unimplemented!("query implementation")
    }

    /// Returns the changes and event occurrences within the range.
    pub fn changes(&self) -> &[Change] {
        unimplemented!("query implementation")
    }
}
