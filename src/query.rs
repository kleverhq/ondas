use std::{collections::HashMap, ops::ControlFlow};

use crate::{Hierarchy, Result, Signal, Time, TimeRange, Value, ValueRef, Waveform};

mod engine;

#[cfg(test)]
mod tests;

/// A reusable, ordered selection of waveform signals.
///
/// Created by [`Waveform::select`], this holds a mutable borrow of the waveform
/// and hides prepared backend resources. It changes query reuse, not semantics:
/// one-shot waveform methods are equivalent to using a temporary selection.
///
/// Input order and duplicates are preserved. Each occurrence of a handle gets
/// its corresponding sample, trace, or scan observations, even for equal aliases
/// or repeated slices. Internal deduplication of base histories is not observable.
/// Candidate timestamps are the exception: they form a unique time union.
///
/// Clone [`Self::hierarchy`] before retaining hierarchy views across mutable
/// selection queries. See the crate-level examples for ownership and borrowing.
pub struct Selection<'w> {
    waveform: &'w mut Waveform,
    signals: Vec<Signal>,
    bases: Vec<Signal>,
    groups: HashMap<usize, Vec<usize>>,
}

impl Selection<'_> {
    /// Returns the selected signals in input order, including duplicates.
    pub fn signals(&self) -> &[Signal] {
        &self.signals
    }

    /// Returns the hierarchy associated with the selected waveform.
    pub fn hierarchy(&self) -> &Hierarchy {
        self.waveform.hierarchy()
    }

    /// Returns an owned sample for each selected signal at `time`.
    ///
    /// Results retain selection order and duplicates. See [`Sample`] for final
    /// tick state, missing values, event counts, and sampling after EOF.
    /// On failure, returns [`Error`](crate::Error), not a partial vector.
    pub fn samples(&mut self, time: Time) -> Result<Vec<Sample>> {
        let mut samples = Vec::with_capacity(self.signals.len());
        let _ = self.visit_samples(time, |sample| {
            samples.push(match sample {
                SampleRef::Missing { signal } => Sample::Missing { signal },
                SampleRef::Value {
                    signal,
                    value,
                    changed_at,
                } => Sample::Value {
                    signal,
                    value: value.to_owned(),
                    changed_at,
                },
                SampleRef::Event {
                    signal,
                    occurrences,
                } => Sample::Event {
                    signal,
                    occurrences,
                },
            });
            ControlFlow::<()>::Continue(())
        })?;
        Ok(samples)
    }

    /// Returns an owned trace for each selected signal over `range`.
    ///
    /// Results retain selection order and duplicates and follow [`Self::scan`]
    /// semantics. Empty ranges produce traces with no initial state or changes.
    /// On failure, returns [`Error`](crate::Error), not a partial vector.
    pub fn traces(&mut self, range: TimeRange) -> Result<Vec<Trace>> {
        let mut traces = self
            .signals
            .iter()
            .map(|&signal| Trace {
                signal,
                range,
                initial: None,
                changes: Vec::new(),
            })
            .collect::<Vec<_>>();
        let _ = self.scan_each(range, |index, record| {
            match record {
                ScanRef::Initial {
                    value, changed_at, ..
                } => {
                    traces[index].initial = Some(Initial {
                        value: value.to_owned(),
                        changed_at,
                    })
                }
                ScanRef::Change { time, value, .. } => traces[index].changes.push(Change {
                    time,
                    value: value.to_owned(),
                }),
            }
            ControlFlow::<()>::Continue(())
        })?;
        Ok(traces)
    }

    /// Visits a borrowed sample for each signal in selection order at `time`.
    ///
    /// Includes duplicates and follows [`Sample`] semantics. A callback's borrowed
    /// values cannot outlive that callback; copy a value with [`ValueRef::to_owned`]
    /// to retain it. Returning [`ControlFlow::Break`] ends successfully and returns
    /// that break value inside `Ok`, rather than reporting a query error.
    pub fn visit_samples<B>(
        &mut self,
        time: Time,
        visitor: impl for<'v> FnMut(SampleRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.sample_visit(time, visitor)
    }

    /// Visits entering states, then time-ordered changes, in inclusive `range`.
    ///
    /// # Entering state
    ///
    /// For each selection entry with a known persistent state strictly before
    /// `range.start()`, emits at most one [`ScanRef::Initial`]. All initials come
    /// first in selection order, including duplicate entries. A known
    /// `changed_at` is strictly less than the start. Events have no initial state.
    /// No synthetic change at `start - 1` is created, including at tick zero.
    ///
    /// # Changes and ordering
    ///
    /// Changes and event occurrences inside the closed range follow in
    /// nondecreasing time. Different signals have no defined order within one
    /// tick. Multiple changes to the same signal within one tick preserve their
    /// order and distinct intermediate values; only redundant writes identical
    /// to the preceding known persistent state may be omitted. Without an
    /// initial state, the first record establishing a previously unknown state
    /// is retained. Point sampling instead uses that tick's final state.
    ///
    /// Each [`ValueRef::Event`] change is one occurrence, subject to the FST
    /// [first-tick initialization limitation](crate#reader-support-and-limits).
    /// Occurrences never coalesce or deduplicate, even with identical time and value. Slices emit
    /// only changes of their projected values, not unrelated base-bit activity.
    /// Duplicate selection entries retain their observations. An empty range
    /// invokes no visitor, including for initials.
    ///
    /// # Borrowing, stopping, and errors
    ///
    /// Values can borrow backend buffers and are valid only during the callback;
    /// use [`ValueRef::to_owned`] to retain them. [`ControlFlow::Break`] is a
    /// successful early stop, returned as `Ok(ControlFlow::Break(value))`.
    /// A completed scan returns `Ok(ControlFlow::Continue(()))`.
    ///
    /// This operation has partial-observation semantics: a late backend failure
    /// returns [`Error`](crate::Error) after earlier visitor calls may have run.
    /// It does not roll those observations back. Owned [`Self::samples`] and
    /// [`Self::traces`] instead return no partial result on error.
    pub fn scan<B>(
        &mut self,
        range: TimeRange,
        mut visitor: impl for<'v> FnMut(ScanRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.scan_each(range, |_, record| visitor(record))
    }

    /// Visits a strictly increasing superset of selected change times in `range`.
    ///
    /// Every timestamp that could be emitted as [`ScanRef::Change`] by a full
    /// [`Self::scan`] of this selection and range must occur. Additional candidate
    /// times are allowed; for a slice these may include changes to other bits of
    /// its base signal. This permits activity indexes without decoding values.
    ///
    /// Initial states do not contribute timestamps. The union identifies neither
    /// the changing signal nor event multiplicity: repeated handles and multiple
    /// occurrences at one tick still yield that tick only once. Bounds are
    /// inclusive; an empty range produces no calls.
    ///
    /// [`ControlFlow::Break`] stops successfully with the visitor's break value
    /// inside `Ok`; normal completion returns `Ok(ControlFlow::Continue(()))`.
    pub fn scan_candidate_times<B>(
        &mut self,
        range: TimeRange,
        mut visitor: impl FnMut(Time) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        // ponytail: decode selected values; use a cheaper activity index if benchmarks justify it.
        let mut last = None;
        self.scan(range, |record| {
            if let ScanRef::Change { time, .. } = record
                && last != Some(time)
            {
                last = Some(time);
                return visitor(time);
            }
            ControlFlow::Continue(())
        })
    }
}

/// An owned value or event sample for a signal at one timestamp.
///
/// A persistent signal returns its final state after all of its changes at the
/// sampled tick. `changed_at`, when known, is the tick that established the
/// observed state and is no later than the sampled time. For a slice, it is the
/// last projected-value change, not activity in other base bits; an unreliable
/// timestamp is `None`.
///
/// [`Self::Missing`] means no persistent value is known at or before that time,
/// not a read error or an HDL unknown logic value. Events have no persistent
/// state and never use `Missing`: [`Self::Event`] counts occurrences at exactly
/// the requested tick, including zero. The FST reader can expose initialization
/// callbacks at the first recorded tick; see the
/// [reader limits](crate#reader-support-and-limits) before interpreting that count.
///
/// Query times are not restricted by [`Metadata::time_span`](crate::Metadata::time_span).
/// After EOF, the last known persistent value is held; event counts are zero.
/// Each result carries its selected [`Signal`], including a slice handle, so
/// batch results remain self-contained. Owned queries return no partial result
/// on a file or backend error.
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

/// A borrowed sample with the same observation semantics as [`Sample`].
///
/// A view from [`Selection::visit_samples`] is valid only during its callback.
/// A view from [`Sample::as_ref`] instead borrows that owned sample.
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
        self.as_ref().signal()
    }

    /// Borrows this sample without copying its owned value.
    pub fn as_ref(&self) -> SampleRef<'_> {
        match self {
            Self::Missing { signal } => SampleRef::Missing { signal: *signal },
            Self::Value {
                signal,
                value,
                changed_at,
            } => SampleRef::Value {
                signal: *signal,
                value: value.as_ref(),
                changed_at: *changed_at,
            },
            Self::Event {
                signal,
                occurrences,
            } => SampleRef::Event {
                signal: *signal,
                occurrences: *occurrences,
            },
        }
    }
}

impl SampleRef<'_> {
    /// Returns the sampled signal.
    pub fn signal(self) -> Signal {
        match self {
            Self::Missing { signal } | Self::Value { signal, .. } | Self::Event { signal, .. } => {
                signal
            }
        }
    }
}

/// A borrowed record emitted by a range scan.
///
/// [`Selection::scan`] defines initial-state ordering, same-tick changes, event
/// multiplicity, duplicates, and partial observations. Values supplied by a scan
/// are callback-scoped; use [`ValueRef::to_owned`] to retain their contents.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ScanRef<'a> {
    /// A persistent state established before the range start.
    Initial {
        /// The signal whose entering state is reported.
        signal: Signal,
        /// The borrowed entering value.
        value: ValueRef<'a>,
        /// The time that established the value, if known; strictly before range start.
        ///
        /// For a slice this describes the projected value, not unrelated base activity.
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

/// An owned persistent state established strictly before a trace range.
///
/// This is separate from actual changes in the range, not a synthetic timestamp.
/// Event signals have no entering state.
pub struct Initial {
    value: Value,
    changed_at: Option<Time>,
}

/// An owned value change or event occurrence within a trace range.
///
/// Each event record represents one occurrence, even if its time and value match
/// another record. Distinct intermediate persistent values within a tick remain
/// separate changes in their original same-signal order.
pub struct Change {
    time: Time,
    value: Value,
}

/// An owned range scan for one signal.
///
/// Follows [`Selection::scan`] semantics: a state strictly before the range is
/// stored separately from changes inside the closed bounds. An empty range has
/// no initial state or changes. No changes in a nonempty range is a successful
/// empty change list; an entering state may still be present.
///
/// Owned trace queries return an error without a partial trace on read failure.
/// Views obtained from this trace's [`Initial`] and [`Change`] objects borrow
/// their owned storage, not a callback buffer.
pub struct Trace {
    signal: Signal,
    range: TimeRange,
    initial: Option<Initial>,
    changes: Vec<Change>,
}

impl Initial {
    /// Returns the entering value.
    pub fn value(&self) -> ValueRef<'_> {
        self.value.as_ref()
    }

    /// Returns the time that established the entering value, if known.
    ///
    /// Always strictly before the trace range start. For a slice this is the
    /// projected value's change time, or `None` if it cannot be determined reliably.
    pub fn changed_at(&self) -> Option<Time> {
        self.changed_at
    }
}

impl Change {
    /// Returns the change or event occurrence time.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Returns the new value, or [`ValueRef::Event`] for an occurrence.
    pub fn value(&self) -> ValueRef<'_> {
        self.value.as_ref()
    }
}

impl Trace {
    /// Returns the traced signal.
    pub fn signal(&self) -> Signal {
        self.signal
    }

    /// Returns the inclusive range covered by this trace.
    pub fn range(&self) -> TimeRange {
        self.range
    }

    /// Returns the persistent state established strictly before the range, if any.
    ///
    /// Returns `None` for an empty range, an event signal, or no known entering state.
    pub fn initial(&self) -> Option<&Initial> {
        self.initial.as_ref()
    }

    /// Returns the changes and event occurrences within the closed range.
    ///
    /// Times are nondecreasing. Same-signal changes within a tick retain their
    /// order and distinct intermediate values. Redundant identical persistent
    /// writes may be omitted; event occurrences are never coalesced.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }
}
