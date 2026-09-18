use super::*;
use crate::{Encoding, Error};

struct State {
    value: Value,
    changed_at: Option<Time>,
}

#[derive(Default)]
pub(super) struct Slot {
    state: Option<State>,
    pending: Option<Value>,
    events: u64,
}

fn update(state: &mut Option<State>, value: ValueRef<'_>, time: Time) -> bool {
    if state
        .as_ref()
        .is_some_and(|previous| previous.value.as_ref().same_value(value))
    {
        return false;
    }
    let changed_at = state.as_ref().map(|_| time);
    *state = Some(State {
        value: value.to_owned(),
        changed_at,
    });
    true
}

fn validate_indices(len: usize, indices: &[usize]) -> Result<()> {
    if let Some(&index) = indices.iter().find(|&&index| index >= len) {
        return Err(Error::InvalidSelectionIndex { index, len });
    }
    Ok(())
}

impl QueryContext<'_> {
    /// Returns the current completed candidate tick.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Visits requested samples in subset order, including repeated indices.
    ///
    /// Indices refer to the original selection, not the driver subset. The
    /// entire subset and sampling time are validated before any callback.
    /// Empty subsets produce no callbacks. Invalid indices return
    /// [`Error::InvalidSelectionIndex`]. Only the current absolute tick and the
    /// tick immediately before it (`t - 1`, when `t > 0`) are valid; other times
    /// return [`Error::InvalidQueryTime`].
    ///
    /// Persistent values and change times follow [`Selection::samples`]; events
    /// are exact-tick counts, including zero. A visitor borrows each sample only
    /// during its callback. Its `Break` stops this subset visit and is returned
    /// unchanged; the caller decides whether to stop the outer query. Errors
    /// propagate unchanged, without rolling back earlier successful callbacks.
    pub fn visit_samples<B>(
        &self,
        time: Time,
        indices: &[usize],
        mut visitor: impl for<'v> FnMut(usize, SampleRef<'v>) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        let current = time == self.time;
        if !current && self.time.ticks().checked_sub(1).map(Time::from_ticks) != Some(time) {
            return Err(Error::InvalidQueryTime {
                requested: time,
                current: self.time,
            });
        }
        validate_indices(self.signals.len(), indices)?;
        for &index in indices {
            let signal = self.signals[index];
            let slot = &self.slots[index];
            let sample = if signal.encoding() == Encoding::Event {
                let occurrences = if current {
                    slot.events
                } else if self.previous_tick == Some(time) {
                    self.previous_events[index]
                } else {
                    0
                };
                SampleRef::Event {
                    signal,
                    occurrences,
                }
            } else {
                let state = slot.state.as_ref();
                let value = if current {
                    slot.pending.as_ref().or(state.map(|state| &state.value))
                } else {
                    state.map(|state| &state.value)
                };
                if let Some(value) = value {
                    let changed_at = state.and_then(|state| {
                        if current && !state.value.as_ref().same_value(value.as_ref()) {
                            Some(self.time)
                        } else {
                            state.changed_at
                        }
                    });
                    SampleRef::Value {
                        signal,
                        value: value.as_ref(),
                        changed_at,
                    }
                } else {
                    SampleRef::Missing { signal }
                }
            };
            if let ControlFlow::Break(value) = visitor(index, sample)? {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

fn project(value: ValueRef<'_>, signal: Signal) -> ValueRef<'_> {
    match value {
        ValueRef::Bits(bits) if signal.is_slice() => {
            let lsb = signal.lsb();
            ValueRef::Bits(bits.slice(lsb + signal.width().expect("bit projection") - 1, lsb))
        }
        value => value,
    }
}

fn initials<B>(
    signals: &[Signal],
    slots: &[Slot],
    visitor: &mut impl for<'v> FnMut(usize, ScanRef<'v>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    for (index, (signal, slot)) in signals.iter().zip(slots).enumerate() {
        if let Some(state) = &slot.state
            && let ControlFlow::Break(value) = visitor(
                index,
                ScanRef::Initial {
                    signal: *signal,
                    value: state.value.as_ref(),
                    changed_at: state.changed_at,
                },
            )
        {
            return ControlFlow::Break(value);
        }
    }
    ControlFlow::Continue(())
}

fn scan_tick<B>(
    signals: &[Signal],
    slots: &[Slot],
    time: Time,
    range: TimeRange,
    emitted_initials: &mut bool,
    visitor: &mut impl for<'v> FnMut(usize, ScanRef<'v>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    if !*emitted_initials && time >= range.start() {
        *emitted_initials = true;
        initials(signals, slots, visitor)?;
    }
    for (index, (&signal, slot)) in signals.iter().zip(slots).enumerate() {
        if let Some(value) = &slot.pending
            && !slot
                .state
                .as_ref()
                .is_some_and(|state| state.value.as_ref().same_value(value.as_ref()))
            && time >= range.start()
        {
            visitor(
                index,
                ScanRef::Change {
                    signal,
                    time,
                    value: value.as_ref(),
                },
            )?;
        }
        let events = slot.events;
        if time >= range.start() && events > 0 {
            visitor(
                index,
                ScanRef::Change {
                    signal,
                    time,
                    value: ValueRef::Event {
                        occurrences: events,
                    },
                },
            )?;
        }
    }
    ControlFlow::Continue(())
}

// Visit a finished tick before committing it, so both entering and final states
// remain available to the same owner. Only selected, bounded state is retained.
fn complete_tick<B>(
    signals: &[Signal],
    slots: &mut [Slot],
    time: Time,
    visitor: &mut impl FnMut(Time, &[Signal], &[Slot]) -> Result<ControlFlow<B>>,
) -> Result<ControlFlow<B>> {
    if let ControlFlow::Break(value) = visitor(time, signals, slots)? {
        return Ok(ControlFlow::Break(value));
    }
    for slot in slots {
        if let Some(value) = slot.pending.take() {
            update(&mut slot.state, value.as_ref(), time);
        }
        slot.events = 0;
    }
    Ok(ControlFlow::Continue(()))
}

impl<'w> Selection<'w> {
    pub(crate) fn new(waveform: &'w mut Waveform, signals: &[Signal]) -> Result<Self> {
        let mut groups = HashMap::<usize, Vec<usize>>::new();
        let mut bases = Vec::new();
        for (position, &signal) in signals.iter().enumerate() {
            let index = waveform.hierarchy().validate(signal)?;
            if signal.encoding() == Encoding::Unsupported {
                return Err(Error::UnsupportedSignal { signal });
            }
            let entries = groups.entry(index).or_default();
            if entries.is_empty() {
                bases.push(signal.base());
            }
            entries.push(position);
        }
        Ok(Self {
            waveform,
            signals: signals.to_vec(),
            bases,
            groups,
        })
    }

    /// Visits candidate times and lets each callback read selected samples.
    ///
    /// `drivers` lists positions in this selection that determine candidate
    /// times. The callback can read any selection entry, not just its drivers,
    /// through a [`QueryContext`].
    ///
    /// # Candidate times
    ///
    /// Candidates are increasing and unique in the inclusive range. A candidate
    /// need not be an actual change: identical writes or changes hidden by a
    /// projection may produce extra callbacks. Callers must confirm their conditions.
    /// Empty drivers or an empty range produce no callbacks. Driver indices are
    /// validated before reading; invalid indices return [`Error::InvalidSelectionIndex`].
    ///
    /// On successful completion without an early stop, every timestamp at which
    /// a full normalized scan of the driver entries in the same range would emit
    /// a [`ScanRef::Change`] is visited, including event aggregates. Initial states
    /// do not contribute timestamps, and driver duplicates do not multiply them.
    /// The exact superset need not match [`Self::scan_candidate_times`] or remain
    /// identical across reader implementations.
    ///
    /// # Reads and memory
    ///
    /// Each context exposes only its completed absolute tick and the tick
    /// immediately before it (`t - 1`, when `t > 0`), even if that preceding tick
    /// lies before the range start. No unfinished tick is published. Only requested samples are delivered to read visitors; a
    /// sequential fallback may decode records for all selected histories and
    /// traverse the prefix once to establish entering state. It never replays
    /// per operand or collects all candidates. Additional query state is bounded
    /// by selection and value sizes, excluding input, reader/index and SDK residency.
    ///
    /// # Stopping and errors
    ///
    /// Callback `Break` stops delivery immediately and is returned unchanged.
    /// Callback/read errors propagate without rolling back earlier observations.
    /// The reader may decode one next-tick record to complete a tick before
    /// invoking the callback. Fresh queries are valid after completion, stop or
    /// error, under the reader's documented malformed-input limitations.
    ///
    /// # Basic use
    ///
    /// ```no_run
    /// # use ondas::{Result, Selection, TimeRange};
    /// # use std::ops::ControlFlow;
    /// # fn example(selection: &mut Selection<'_>) -> Result<()> {
    /// let _ = selection.query(TimeRange::all(), &[0], |context| {
    ///     context.visit_samples(context.time(), &[0], |index, sample| {
    ///         println!("slot {index}: {sample:?}");
    ///         Ok(ControlFlow::<()>::Continue(()))
    ///     })
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Example: read payload only when a condition passes
    ///
    /// The selection has three entries: activity, control and payload. Only
    /// activity drives candidate times. At each candidate the caller checks for
    /// a strict `0 → 1` transition, then checks that control is high at that tick.
    /// Missing values and other logic states do not satisfy this condition.
    ///
    /// The payload visitor runs only for accepted candidates. It copies the
    /// value so the result survives the callback and the waveform itself.
    /// This limits caller reads and copies; a sequential reader may still decode
    /// selected payload while advancing. The example uses tiny in-memory VCD
    /// data so it can run without an external file.
    ///
    /// ```rust
    /// use std::ops::ControlFlow;
    /// use ondas::{Logic, SampleRef, Time, TimeRange, ValueRef};
    ///
    /// fn scalar(sample: SampleRef<'_>) -> Option<Logic> {
    ///     match sample {
    ///         SampleRef::Value { value: ValueRef::Bits(bits), .. } if bits.width() == 1 => bits.bit(0),
    ///         _ => None,
    ///     }
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let input = b"$var wire 1 c activity $end $var wire 1 e control $end
    ///     $var wire 4 d payload $end $enddefinitions $end
    ///     #0 0c 0e b0011 d #1 1c #2 1e 0c #3 b1001 d 1c
    ///     #4 0c 0e #5 1c b1111 d #6 1e 0c #7 b0110 d 1c";
    /// let mut wave = ondas::open_bytes_with("conditional.vcd", input.as_slice().into(), "vcd-native")?;
    /// let signals = ["activity", "control", "payload"].map(|name| wave.hierarchy().signal(name).unwrap());
    /// let mut selection = wave.select(&signals)?;
    /// let mut accepted = Vec::new();
    /// let _ = selection.query(TimeRange::all(), &[0], |context| {
    ///     let time = context.time();
    ///     let Some(previous) = time.ticks().checked_sub(1).map(Time::from_ticks) else {
    ///         return Ok(ControlFlow::<()>::Continue(())); // No predecessor at tick zero.
    ///     };
    ///     let mut before = None;
    ///     let _ = context.visit_samples(previous, &[0], |_, sample| {
    ///         before = scalar(sample);
    ///         Ok(ControlFlow::<()>::Continue(()))
    ///     })?;
    ///     let mut now = [None; 2];
    ///     let _ = context.visit_samples(time, &[0, 1], |slot, sample| {
    ///         now[slot] = scalar(sample);
    ///         Ok(ControlFlow::<()>::Continue(()))
    ///     })?;
    ///     if before == Some(Logic::Zero) && now == [Some(Logic::One), Some(Logic::One)] {
    ///         let _ = context.visit_samples(time, &[2], |_, sample| {
    ///             if let SampleRef::Value { value, .. } = sample {
    ///                 accepted.push((time, value.to_owned())); // Explicit caller ownership.
    ///             } // This caller skips missing payload.
    ///             Ok(ControlFlow::<()>::Continue(()))
    ///         })?;
    ///     }
    ///     Ok(ControlFlow::<()>::Continue(()))
    /// })?;
    /// drop(wave);
    /// let output = accepted.iter().map(|(time, value)| {
    ///     let ValueRef::Bits(bits) = value.as_ref() else { panic!("expected bit payload") };
    ///     (time.ticks(), bits.to_string()) // Borrows retained storage, not callback storage.
    /// }).collect::<Vec<_>>();
    /// assert_eq!(output, [(3, "1001".into()), (7, "0110".into())]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn query<B>(
        &mut self,
        range: TimeRange,
        drivers: &[usize],
        mut visitor: impl FnMut(&QueryContext<'_>) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        validate_indices(self.signals.len(), drivers)?;
        let end = range
            .end()
            .or_else(|| self.waveform.metadata().time_span().map(|span| span.last()))
            .unwrap_or(Time::from_ticks(u64::MAX));
        if drivers.is_empty() || range.is_empty() || range.start() > end {
            return Ok(ControlFlow::Continue(()));
        }
        let mut slots = (0..self.signals.len())
            .map(|_| Slot::default())
            .collect::<Vec<_>>();
        let mut previous_events = vec![0; self.signals.len()];
        let mut previous_tick = None;
        self.read_ticks(end, &mut slots, |time, signals, slots| {
            if time >= range.start()
                && drivers
                    .iter()
                    .any(|&index| slots[index].pending.is_some() || slots[index].events > 0)
            {
                let context = QueryContext {
                    time,
                    signals,
                    slots,
                    previous_tick,
                    previous_events: &previous_events,
                };
                if let ControlFlow::Break(value) = visitor(&context)? {
                    return Ok(ControlFlow::Break(value));
                }
            }
            for (previous, slot) in previous_events.iter_mut().zip(slots) {
                *previous = slot.events;
            }
            previous_tick = Some(time);
            Ok(ControlFlow::Continue(()))
        })
    }

    pub(super) fn sample_visit<B>(
        &mut self,
        time: Time,
        mut visitor: impl for<'v> FnMut(SampleRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        let mut states = (0..self.signals.len())
            .map(|_| None)
            .collect::<Vec<Option<State>>>();
        let mut events = vec![0u64; self.signals.len()];
        let _ = self.scan_each(TimeRange::point(time), |index, record| {
            match record {
                ScanRef::Initial {
                    value, changed_at, ..
                } => {
                    states[index] = Some(State {
                        value: value.to_owned(),
                        changed_at,
                    });
                }
                ScanRef::Change { time, value, .. } => {
                    if let ValueRef::Event { occurrences } = value {
                        events[index] = occurrences;
                    } else {
                        update(&mut states[index], value, time);
                    }
                }
            }
            ControlFlow::<()>::Continue(())
        })?;
        for (index, &signal) in self.signals.iter().enumerate() {
            let sample = if signal.encoding() == Encoding::Event {
                SampleRef::Event {
                    signal,
                    occurrences: events[index],
                }
            } else if let Some(state) = &states[index] {
                SampleRef::Value {
                    signal,
                    value: state.value.as_ref(),
                    changed_at: state.changed_at,
                }
            } else {
                SampleRef::Missing { signal }
            };
            if let ControlFlow::Break(value) = visitor(sample) {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Visits normalized records with their input selection-entry index.
    ///
    /// This is the indexed form of [`Self::scan`], with identical range,
    /// complete-tick, event-count, stopping and error semantics. Every index is
    /// in `0..self.signals().len()` and identifies an input position, not a
    /// backend offset or global signal ID. Aliases and repeated whole signals
    /// or slices each receive their own slot's records. Duplicate entries each
    /// receive the same per-tick event aggregate; internal base-history
    /// deduplication is not observable.
    ///
    /// Initial states appear first in selection order (entries without one are
    /// omitted). Changes follow in nondecreasing time order, with no additional
    /// cross-entry ordering promise within a tick. An empty selection invokes
    /// no callbacks. Records borrow storage only for the callback; use
    /// [`ValueRef::to_owned`] to retain values. `Break` stops delivery immediately
    /// and is returned unchanged; a read error preserves prior callbacks but
    /// does not publish the unfinished tick. No complete history is collected.
    ///
    /// # Basic use
    ///
    /// ```no_run
    /// # use ondas::{Result, Selection, TimeRange};
    /// # use std::ops::ControlFlow;
    /// # fn example(selection: &mut Selection<'_>) -> Result<()> {
    /// let _ = selection.scan_each(TimeRange::all(), |index, record| {
    ///     println!("slot {index}: {record:?}");
    ///     ControlFlow::<()>::Continue(())
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Example: export changes by selection slot
    ///
    /// The selection below contains a bus, an alias of that bus and two slices.
    /// Each has its own slot, even though they share one source history. The
    /// visitor handles initial states separately and renders each borrowed change
    /// immediately. It stops after four changes instead of collecting a trace.
    ///
    /// The small output buffer is only a test sink, bounded by that four-change
    /// limit. A real exporter can write directly to its destination. Sorting the
    /// assertion buffer avoids relying on cross-slot order within a tick.
    ///
    /// ```rust
    /// use std::ops::ControlFlow;
    /// use ondas::{Sample, ScanRef, Time, TimeRange, ValueRef};
    ///
    /// fn render(value: ValueRef<'_>) -> String {
    ///     match value {
    ///         ValueRef::Bits(bits) => format!("bits:{bits}"),
    ///         ValueRef::Real(real) => format!("real:{:016x}", real.to_bits()),
    ///         ValueRef::String(text) => format!("string:{text:?}"),
    ///         ValueRef::Event { occurrences } => format!("events:{occurrences}"),
    ///         _ => format!("{value:?}"),
    ///     }
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let input = b"$var wire 4 ! data $end $var wire 4 ! alias $end
    ///     $enddefinitions $end #0 b0000 ! #2 b1111 ! b0000 !
    ///     #5 b1001 ! #8 b0110 !";
    /// let mut wave = ondas::open_bytes_with("export.vcd", input.as_slice().into(), "vcd-native")?;
    /// let hierarchy = wave.hierarchy().clone();
    /// let declaration = hierarchy.variable("data")?;
    /// assert_eq!(declaration.signedness(), None); // Do not invent missing interpretation.
    /// assert_eq!(declaration.logic_domain(), None);
    /// let data = hierarchy.signal("data")?;
    /// let alias = hierarchy.signal("alias")?;
    /// let high = data.slice(3, 2)?;
    /// let low = data.slice(1, 0)?;
    /// assert_eq!(data, alias); // Distinct declarations can share a history.
    ///
    /// // No candidate traversal is needed for an ordinary point or batch request.
    /// assert!(matches!(wave.sample(data, Time::from_ticks(5))?, Sample::Value { .. }));
    /// let batch = wave.samples(&[data, low], Time::from_ticks(5))?;
    /// assert_eq!(batch.len(), 2);
    ///
    /// let mut selection = wave.select(&[data, alias, high, low])?;
    /// let mut entering_slots = Vec::new();
    /// let mut exported = Vec::new(); // Small bounded test sink; a real sink can write directly.
    /// let outcome = selection.scan_each(TimeRange::from(Time::from_ticks(1)), |slot, record| {
    ///     match record {
    ///         ScanRef::Initial { .. } => entering_slots.push(slot),
    ///         ScanRef::Change { time, value, .. } => {
    ///             let text = render(value); // Borrowed value is consumed only here.
    ///             println!("{slot}\t{}\t{text}", time.ticks());
    ///             exported.push((slot, time.ticks(), text));
    ///             if exported.len() == 4 {
    ///                 return ControlFlow::Break(4);
    ///             }
    ///         }
    ///         _ => {} // Public result enums are non-exhaustive.
    ///     }
    ///     ControlFlow::Continue(())
    /// })?;
    /// assert_eq!(outcome, ControlFlow::Break(4));
    /// assert_eq!(entering_slots, [0, 1, 2, 3]);
    /// exported.sort();
    /// assert_eq!(exported, vec![
    ///     (0, 5, "bits:1001".into()), (1, 5, "bits:1001".into()),
    ///     (2, 5, "bits:10".into()), (3, 5, "bits:01".into()),
    /// ]); // The excursion at tick 2 disappears; tick 8 is not delivered after Break.
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Borrowed records
    ///
    /// A borrowed record cannot escape its callback:
    ///
    /// ```compile_fail,E0521
    /// use ondas::{Result, Selection, TimeRange};
    /// use std::ops::ControlFlow;
    /// fn escape(selection: &mut Selection<'_>) -> Result<()> {
    ///     let mut saved = None;
    ///     selection.scan_each(TimeRange::all(), |_, record| {
    ///         saved = Some(record);
    ///         ControlFlow::<()>::Continue(())
    ///     })?;
    ///     println!("{saved:?}");
    ///     Ok(())
    /// }
    /// ```
    pub fn scan_each<B>(
        &mut self,
        range: TimeRange,
        mut visitor: impl for<'v> FnMut(usize, ScanRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        let end = range
            .end()
            .or_else(|| self.waveform.metadata().time_span().map(|span| span.last()))
            .unwrap_or(Time::from_ticks(u64::MAX));
        if range.is_empty() || range.start() > end || self.signals.is_empty() {
            return Ok(ControlFlow::Continue(()));
        }
        let mut slots = (0..self.signals.len())
            .map(|_| Slot::default())
            .collect::<Vec<_>>();
        let mut emitted_initials = false;
        if let ControlFlow::Break(value) =
            self.read_ticks(end, &mut slots, |time, signals, slots| {
                Ok(scan_tick(
                    signals,
                    slots,
                    time,
                    range,
                    &mut emitted_initials,
                    &mut visitor,
                ))
            })?
        {
            return Ok(ControlFlow::Break(value));
        }
        if !emitted_initials {
            return Ok(initials(&self.signals, &slots, &mut visitor));
        }
        Ok(ControlFlow::Continue(()))
    }

    // Every operation supplies its bounded slots and receives only completed
    // ticks. Reader dispatch, input ownership and one-prefix traversal stay here.
    fn read_ticks<B>(
        &mut self,
        end: Time,
        slots: &mut [Slot],
        mut visitor: impl FnMut(Time, &[Signal], &[Slot]) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        let mut pending_time = None;
        let backend = self.waveform.backend().to_owned();
        #[cfg(test)]
        let probe = streaming_tests::probe(&self.waveform.reader);
        // One entering/final value and event count per slot, never intra-tick writes.
        let result = self
            .waveform
            .reader
            .read(&self.bases, end, |base, time, value| {
                if let Some(previous) = pending_time
                    && previous != time
                {
                    match complete_tick(&self.signals, slots, previous, &mut visitor) {
                        Ok(ControlFlow::Continue(())) => (),
                        outcome => return ControlFlow::Break(outcome),
                    }
                }
                pending_time = Some(time);
                for &index in &self.groups[&base] {
                    let slot = &mut slots[index];
                    if let ValueRef::Event { occurrences } = value {
                        let Some(count) = slot.events.checked_add(occurrences) else {
                            return ControlFlow::Break(Err(Error::Backend {
                                backend: backend.clone(),
                                operation: "count events",
                                message: "event occurrence count exceeds u64::MAX".into(),
                            }));
                        };
                        slot.events = count;
                    } else {
                        slot.pending = Some(project(value, self.signals[index]).to_owned());
                    }
                }
                #[cfg(test)]
                streaming_tests::observe_pending(&probe, slots);
                ControlFlow::Continue(())
            })?;
        if let ControlFlow::Break(outcome) = result {
            return outcome;
        }
        // Only successful EOF completes the final pending tick.
        if let Some(time) = pending_time {
            return complete_tick(&self.signals, slots, time, &mut visitor);
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[cfg(test)]
mod streaming_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_tick_callback_reads_before_and_final_state_and_can_fail() {
        for fail in [false, true] {
            let mut wave = Waveform::memory(
                vec![Encoding::Real, Encoding::Event],
                vec![
                    (0, 0, Value::Real(1.0)),
                    (0, 0, Value::Real(2.0)),
                    (1, 0, Value::Event { occurrences: 1 }),
                    (1, 0, Value::Event { occurrences: 1 }),
                    (0, 3, Value::Real(3.0)),
                ],
                None,
            );
            let signals = wave.hierarchy().signals().collect::<Vec<_>>();
            let mut selection = wave.select(&signals).unwrap();
            let mut slots = [Slot::default(), Slot::default()];
            let mut ticks = Vec::new();
            let result =
                selection.read_ticks(Time::from_ticks(3), &mut slots, |time, entries, slots| {
                    assert_eq!(entries, signals);
                    ticks.push(time.ticks());
                    if time == Time::ZERO {
                        assert!(slots[0].state.is_none());
                        assert!(matches!(slots[0].pending, Some(Value::Real(2.0))));
                        assert_eq!(slots[1].events, 2);
                    } else {
                        assert!(matches!(
                            slots[0].state.as_ref().unwrap().value,
                            Value::Real(2.0)
                        ));
                        assert!(matches!(slots[0].pending, Some(Value::Real(3.0))));
                        assert_eq!(slots[1].events, 0);
                        if fail {
                            return Err(Error::Backend {
                                backend: "memory".into(),
                                operation: "tick visitor",
                                message: "injected consumer failure".into(),
                            });
                        }
                    }
                    Ok(ControlFlow::<()>::Continue(()))
                });
            assert_eq!(ticks, [0, 3]);
            if fail {
                assert!(matches!(
                    result,
                    Err(Error::Backend {
                        operation: "tick visitor",
                        ..
                    })
                ));
                assert!(matches!(
                    slots[0].state.as_ref().unwrap().value,
                    Value::Real(2.0)
                ));
            } else {
                assert_eq!(result.unwrap(), ControlFlow::Continue(()));
                assert!(matches!(
                    slots[0].state.as_ref().unwrap().value,
                    Value::Real(3.0)
                ));
            }
            let mut fresh = [Slot::default(), Slot::default()];
            let _ = selection
                .read_ticks(Time::from_ticks(3), &mut fresh, |_, _, _| {
                    Ok(ControlFlow::<()>::Continue(()))
                })
                .unwrap();
            assert!(matches!(
                fresh[0].state.as_ref().unwrap().value,
                Value::Real(3.0)
            ));
            assert_eq!(
                fresh[0].state.as_ref().unwrap().changed_at,
                Some(Time::from_ticks(3))
            );
        }
    }
}
