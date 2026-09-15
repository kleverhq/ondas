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

/// Selective reads at one completed candidate tick of [`Selection::query`].
///
/// Only the current tick and its checked predecessor are readable; the latter
/// may precede the query range. At tick zero no predecessor exists. Reads may be
/// repeated in any order and return final, never provisional, states. Ordinary
/// conditions and event/edge interpretation remain caller code.
///
/// The context is not an eager caller snapshot or an arbitrary-history handle.
/// A sequential reader may decode records while advancing, but only requested
/// samples are delivered. Borrowed samples are valid only for their visitor;
/// copy a [`ValueRef`] to retain its contents.
///
/// A context cannot escape its candidate callback:
///
/// ```compile_fail,E0521
/// use ondas::{Result, Selection, TimeRange};
/// use std::ops::ControlFlow;
/// fn escape(selection: &mut Selection<'_>) -> Result<()> {
///     let mut saved = None;
///     selection.query(TimeRange::all(), &[0], |context| {
///         saved = Some(context);
///         Ok(ControlFlow::<()>::Continue(()))
///     })?;
///     println!("{:?}", saved.unwrap().time());
///     Ok(())
/// }
/// ```
///
/// A sample cannot escape even into its surrounding candidate callback:
///
/// ```compile_fail,E0521
/// use ondas::{Result, Selection, TimeRange};
/// use std::ops::ControlFlow;
/// fn escape(selection: &mut Selection<'_>) -> Result<()> {
///     selection.query(TimeRange::all(), &[0], |context| {
///         let mut saved = None;
///         context.visit_samples(context.time(), &[0], |_, sample| {
///             saved = Some(sample);
///             Ok(ControlFlow::<()>::Continue(()))
///         })?;
///         println!("{:?}", saved.unwrap());
///         Ok(ControlFlow::<()>::Continue(()))
///     })?;
///     Ok(())
/// }
/// ```
pub struct QueryContext<'a> {
    time: Time,
    signals: &'a [Signal],
    slots: &'a [engine::Slot],
    previous_tick: Option<Time>,
    previous_events: &'a [u64],
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
    /// For input-position indices that distinguish repeated entries, use
    /// [`Self::scan_each`]. This convenience form uses the same execution path.
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
    /// tick. Each persistent selection entry emits at most one net change per
    /// tick: its final recorded value, compared with its entering state using
    /// [`ValueRef`] representation identity. A same-tick excursion returning to
    /// that state disappears and does not advance `changed_at`. Without a known
    /// entering state, the final value establishes one, including HDL unknown.
    /// Samples, scans and traces use these same final tick states.
    ///
    /// Each [`ValueRef::Event`] change aggregates a positive `occurrences` count
    /// for one entry and tick, subject to the FST
    /// [first-tick initialization limitation](crate#reader-support-and-limits).
    /// Counts preserve reader observations, not ordering within the tick or
    /// events omitted by the producer. Overflow returns an error, never wraps.
    /// Repeated selection entries each receive the same count. Slices emit
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
    /// It does not roll those observations back. Only completed ticks are
    /// published; a failure discards the unfinished pending tick. A sequential
    /// reader may need one record of the next tick to complete the preceding
    /// tick before calling the visitor. Owned [`Self::samples`] and
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
/// timestamp is `None`. Same-tick excursions returning to the entering value
/// do not advance this timestamp.
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
    /// A value change or per-tick event aggregate within the range.
    Change {
        /// The signal that changed or produced the event.
        signal: Signal,
        /// The change or occurrence time.
        time: Time,
        /// The new persistent value, or a positive [`ValueRef::Event`] count.
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

/// An owned value change or per-tick event aggregate within a trace range.
///
/// Each event record carries a positive count for one tick. Persistent changes
/// contain only the final recorded state of
/// a tick when it differs from the entering state, or first establishes a state.
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

    /// Returns the new persistent value, or a positive [`ValueRef::Event`] count.
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
    /// Times are nondecreasing. Each persistent signal has at most one net
    /// change per tick, representing its final recorded state. Redundant persistent
    /// writes are omitted; event occurrences are aggregated into one positive
    /// count per tick, without carrying state between ticks.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }
}
