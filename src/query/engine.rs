use super::*;
use crate::{Encoding, Error};

#[derive(Clone)]
struct State {
    value: Value,
    changed_at: Option<Time>,
}

#[derive(Clone, Default)]
pub(super) struct Slot {
    state: Option<State>,
    pending: Option<Value>,
    changed: bool,
    events: u64,
}

#[derive(Clone, Copy)]
enum Position {
    Vcd(crate::backends::vcd::Position),
    #[cfg(feature = "fsdb-lib")]
    Fsdb(Time),
}

impl Position {
    fn time(self) -> Time {
        match self {
            Self::Vcd(position) => Time::from_ticks(position.time),
            #[cfg(feature = "fsdb-lib")]
            Self::Fsdb(time) => time,
        }
    }
}

pub(super) struct Replay {
    position: Position,
    start: Time,
    end: Time,
    time: Option<Time>,
    slots: Vec<Slot>,
}

// One query-boundary snapshot, not a time index or value history.
const CHECKPOINT_BYTES: usize = 4 * 1024 * 1024;

fn checkpoint_bytes(slots: &[Slot]) -> usize {
    slots.iter().fold(
        std::mem::size_of::<Replay>() + std::mem::size_of_val(slots),
        |bytes, slot| {
            [
                slot.state.as_ref().map(|state| &state.value),
                slot.pending.as_ref(),
            ]
            .into_iter()
            .flatten()
            .fold(bytes, |bytes, value| {
                bytes.saturating_add(match value.as_ref() {
                    ValueRef::Bits(bits) => bits.width() as usize,
                    ValueRef::String(text) => text.len(),
                    _ => 0,
                })
            })
        },
    )
}

fn update(state: &mut Option<State>, value: Value, time: Time) -> bool {
    if state
        .as_ref()
        .is_some_and(|previous| previous.value.as_ref().same_value(value.as_ref()))
    {
        return false;
    }
    let changed_at = state.as_ref().map(|_| time);
    *state = Some(State { value, changed_at });
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
            let slot_index = self.slot_indices[index];
            let slot = &self.slots[slot_index];
            let sample = if signal.encoding() == Encoding::Event {
                let occurrences = if current {
                    slot.events
                } else if self.previous_tick == Some(time) {
                    self.previous_events[slot_index]
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
                        if current && slot.changed {
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
    slot_indices: &[usize],
    visitor: &mut impl for<'v> FnMut(usize, ScanRef<'v>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    for (index, signal) in signals.iter().enumerate() {
        let slot = &slots[slot_indices[index]];
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
    slot_indices: &[usize],
    time: Time,
    range: TimeRange,
    emitted_initials: &mut bool,
    visitor: &mut impl for<'v> FnMut(usize, ScanRef<'v>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    if !*emitted_initials && time >= range.start() {
        *emitted_initials = true;
        initials(signals, slots, slot_indices, visitor)?;
    }
    for (index, &signal) in signals.iter().enumerate() {
        let slot = &slots[slot_indices[index]];
        if let Some(value) = &slot.pending
            && slot.changed
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
// remain available to the same owner. Only slots touched at this tick need work.
fn complete_tick<B>(
    signals: &[Signal],
    slots: &mut [Slot],
    active_slots: &mut Vec<usize>,
    time: Time,
    visit_from: Option<Time>,
    visitor: &mut impl FnMut(Time, &[Signal], &[Slot]) -> Result<ControlFlow<B>>,
) -> Result<ControlFlow<B>> {
    for &index in active_slots.iter() {
        #[cfg(test)]
        streaming_tests::comparison_visit();
        let slot = &mut slots[index];
        slot.changed = slot.pending.as_ref().is_some_and(|value| {
            !slot
                .state
                .as_ref()
                .is_some_and(|state| state.value.as_ref().same_value(value.as_ref()))
        });
    }
    if visit_from.is_none_or(|start| time >= start)
        && let ControlFlow::Break(value) = visitor(time, signals, slots)?
    {
        return Ok(ControlFlow::Break(value));
    }
    for index in active_slots.drain(..) {
        #[cfg(test)]
        streaming_tests::commit_visit();
        let slot = &mut slots[index];
        if let Some(value) = slot.pending.take()
            && slot.changed
        {
            slot.state = Some(State {
                value,
                changed_at: slot.state.as_ref().map(|_| time),
            });
        }
        slot.events = 0;
        slot.changed = false;
    }
    Ok(ControlFlow::Continue(()))
}

impl<'w> Selection<'w> {
    pub(crate) fn new(waveform: &'w mut Waveform, signals: &[Signal]) -> Result<Self> {
        let mut groups = HashMap::<usize, Vec<usize>>::new();
        let mut bases = Vec::new();
        let mut first_slots = HashMap::new();
        let mut slot_indices = Vec::with_capacity(signals.len());
        let mut retained_signals = Vec::new();
        for &signal in signals {
            let index = waveform.hierarchy().validate(signal)?;
            if signal.encoding() == Encoding::Unsupported {
                return Err(Error::UnsupportedSignal { signal });
            }
            let position = retained_signals.len();
            let first = *first_slots.entry(signal).or_insert(position);
            slot_indices.push(first);
            if first != position {
                continue;
            }
            retained_signals.push(signal);
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
            slot_indices,
            retained_signals,
            replay: None,
            checkpoint: None,
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
        let mut slots = (0..self.retained_signals.len())
            .map(|_| Slot::default())
            .collect::<Vec<_>>();
        let mut previous_events = vec![0; self.retained_signals.len()];
        let mut previous_tick = None;
        let slot_indices = self.slot_indices.clone();
        // A session also needs the preceding tick's events.
        self.read_ticks_from(
            range.start().ticks().checked_sub(1).map(Time::from_ticks),
            end,
            &mut slots,
            |time, signals, slots| {
                if time >= range.start()
                    && drivers.iter().any(|&index| {
                        let slot = &slots[slot_indices[index]];
                        slot.pending.is_some() || slot.events > 0
                    })
                {
                    let context = QueryContext {
                        time,
                        signals,
                        slots,
                        slot_indices: &slot_indices,
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
            },
        )
    }

    #[cfg(feature = "fsdb-lib")]
    fn fsdb_point_states(&mut self, time: Time) -> Result<Option<Vec<Option<State>>>> {
        if self.signals.is_empty()
            || !self
                .bases
                .iter()
                .all(|signal| matches!(signal.encoding(), Encoding::Bits { .. }))
            || !matches!(self.waveform.reader, crate::backends::Reader::Fsdb(_))
        {
            return Ok(None);
        }
        let next = time.ticks().checked_add(1).map(Time::from_ticks);
        if let Some(replay) = self.replay.as_ref().filter(|replay| {
            replay.time.is_none() && replay.end == time && Some(replay.start) == next
        }) {
            return Ok(Some(
                self.slot_indices
                    .iter()
                    .map(|&index| replay.slots[index].state.clone())
                    .collect(),
            ));
        }
        // Keep existing chronological replay when it already supplies an exact
        // entering state. Otherwise seek cold/backwards bit-only point reads.
        let eligible = |replay: &Replay| replay.start <= time && replay.end <= time;
        if self.replay.as_ref().is_some_and(eligible)
            || self.checkpoint.as_ref().is_some_and(eligible)
        {
            return Ok(None);
        }
        self.replay = None;
        self.checkpoint = None;
        let crate::backends::Reader::Fsdb(reader) = &mut self.waveform.reader else {
            unreachable!()
        };
        let samples = reader.sample_bits(&self.bases, &self.retained_signals, time)?;
        let slots: Vec<_> = samples
            .into_iter()
            .map(|sample| Slot {
                state: match sample {
                    Sample::Value {
                        value, changed_at, ..
                    } => Some(State { value, changed_at }),
                    Sample::Missing { .. } => None,
                    Sample::Event { .. } => unreachable!("bit-only point selection"),
                },
                ..Slot::default()
            })
            .collect();
        let states = self
            .slot_indices
            .iter()
            .map(|&index| slots[index].state.clone())
            .collect();
        if let Some(next) = next.filter(|_| checkpoint_bytes(&slots) <= CHECKPOINT_BYTES) {
            // This is state AFTER time, not entering time. Only later scans may
            // reuse it; same-tick point reads are handled explicitly above.
            self.replay = Some(Replay {
                position: Position::Fsdb(next),
                start: next,
                end: time,
                time: None,
                slots,
            });
        }
        Ok(Some(states))
    }

    pub(super) fn sample_visit<B>(
        &mut self,
        time: Time,
        mut visitor: impl for<'v> FnMut(SampleRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        #[cfg(feature = "fsdb-lib")]
        let (point_sampled, mut states) = {
            let states = self.fsdb_point_states(time)?;
            (
                states.is_some(),
                states.unwrap_or_else(|| vec![None; self.signals.len()]),
            )
        };
        #[cfg(not(feature = "fsdb-lib"))]
        let (point_sampled, mut states) = (false, vec![None; self.signals.len()]);
        let mut events = vec![0u64; self.signals.len()];
        if !point_sampled {
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
                            update(&mut states[index], value.to_owned(), time);
                        }
                    }
                }
                ControlFlow::<()>::Continue(())
            })?;
        }
        // User callbacks also run after traversal has completed. Keep caches
        // provisional until they return, so unwinding cannot retain them.
        let replay = self.replay.take();
        let checkpoint = self.checkpoint.take();
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
        self.replay = replay;
        self.checkpoint = checkpoint;
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
        let mut slots = (0..self.retained_signals.len())
            .map(|_| Slot::default())
            .collect::<Vec<_>>();
        let mut emitted_initials = false;
        let slot_indices = self.slot_indices.clone();
        if let ControlFlow::Break(value) = self.read_ticks_from(
            Some(range.start()),
            end,
            &mut slots,
            |time, signals, slots| {
                Ok(scan_tick(
                    signals,
                    slots,
                    &slot_indices,
                    time,
                    range,
                    &mut emitted_initials,
                    &mut visitor,
                ))
            },
        )? {
            return Ok(ControlFlow::Break(value));
        }
        if !emitted_initials {
            let replay = self.replay.take();
            let checkpoint = self.checkpoint.take();
            let result = initials(&self.signals, &slots, &slot_indices, &mut visitor);
            if result.is_continue() {
                self.replay = replay;
                self.checkpoint = checkpoint;
            }
            return Ok(result);
        }
        Ok(ControlFlow::Continue(()))
    }

    // Every operation supplies its bounded slots and receives only completed
    // ticks. Reader dispatch, input ownership and one-prefix traversal stay here.
    #[cfg(test)]
    fn read_ticks<B>(
        &mut self,
        end: Time,
        slots: &mut [Slot],
        visitor: impl FnMut(Time, &[Signal], &[Slot]) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        self.read_ticks_from(None, end, slots, visitor)
    }

    fn read_ticks_from<B>(
        &mut self,
        start: Option<Time>,
        end: Time,
        slots: &mut [Slot],
        mut visitor: impl FnMut(Time, &[Signal], &[Slot]) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        let eligible =
            |replay: &Replay| start.is_some_and(|start| replay.start <= start) && replay.end <= end;
        #[cfg(feature = "fsdb-lib")]
        if let Some(start) = start.filter(|start| *start > Time::ZERO)
            && !self.replay.as_ref().is_some_and(eligible)
            && !self.checkpoint.as_ref().is_some_and(eligible)
        {
            // Seek the entering state, then traverse only the bounded window.
            // Mixed values and events retain the chronological reference path.
            self.fsdb_point_states(Time::from_ticks(start.ticks() - 1))?;
        }
        let mut checkpoint = self.checkpoint.take();
        let retained = self.replay.take().filter(&eligible);
        let replay = retained.as_ref().or_else(|| {
            checkpoint
                .as_ref()
                .filter(|checkpoint| eligible(checkpoint))
        });
        let mut checkpoint_considered = checkpoint
            .as_ref()
            .is_some_and(|checkpoint| Some(checkpoint.start) == start);
        let replay_start = replay.map(|replay| replay.start).unwrap_or(Time::ZERO);
        let mut pending_time = replay.and_then(|replay| replay.time);
        let position = replay.map(|replay| replay.position);
        if let Some(replay) = replay {
            slots.clone_from_slice(&replay.slots);
        }
        #[cfg(test)]
        streaming_tests::setup_visits(slots.len());
        let mut active_slots = slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                (slot.pending.is_some() || slot.events > 0).then_some(index)
            })
            .collect::<Vec<_>>();
        drop(retained);
        let backend = self.waveform.backend().to_owned();
        #[cfg(test)]
        let probe = streaming_tests::probe(&self.waveform.reader);
        // One entering/final value and event count per slot, never intra-tick writes.
        let mut consume = |base, time, value: ValueRef<'_>, position: Option<Position>| {
            if let Some(previous) = pending_time
                && previous != time
            {
                match complete_tick(
                    &self.signals,
                    slots,
                    &mut active_slots,
                    previous,
                    start,
                    &mut visitor,
                ) {
                    Ok(ControlFlow::Continue(())) => (),
                    outcome => return ControlFlow::Break(outcome),
                }
            }
            if !checkpoint_considered
                && let (Some(start), Some(position)) = (start, position)
                && time >= start
                && pending_time.is_none_or(|previous| previous < start)
            {
                checkpoint_considered = true;
                // The previous tick has committed; this record is still unread
                // at the saved position. Oversized selections simply replay.
                checkpoint = (checkpoint_bytes(slots) <= CHECKPOINT_BYTES).then(|| Replay {
                    position,
                    start,
                    end: position.time(),
                    time: None,
                    slots: slots.to_vec(),
                });
            }
            pending_time = Some(time);
            for &index in &self.groups[&base] {
                let slot = &mut slots[index];
                if slot.pending.is_none()
                    && slot.events == 0
                    && !matches!(value, ValueRef::Event { occurrences: 0 })
                {
                    active_slots.push(index);
                }
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
                    slot.pending = Some(project(value, self.retained_signals[index]).to_owned());
                }
            }
            debug_assert!(active_slots.len() <= slots.len());
            #[cfg(test)]
            streaming_tests::observe_pending(&probe, slots);
            ControlFlow::Continue(())
        };
        let result = match &mut self.waveform.reader {
            crate::backends::Reader::Vcd(reader) => {
                let position = position.map(|position| match position {
                    Position::Vcd(position) => position,
                    #[cfg(feature = "fsdb-lib")]
                    Position::Fsdb(_) => unreachable!("reader is fixed for a selection"),
                });
                reader.read_from(&self.bases, end, position, |base, time, value, position| {
                    consume(base, time, value, Some(Position::Vcd(position)))
                })
            }
            #[cfg(feature = "fsdb-lib")]
            crate::backends::Reader::Fsdb(reader) => {
                let from = match position {
                    Some(Position::Fsdb(time)) => time,
                    None => Time::ZERO,
                    Some(Position::Vcd(_)) => unreachable!("reader is fixed for a selection"),
                };
                // A boundary checkpoint does not retain the preceding tick's
                // event counts. Event selections keep the full reference path.
                let cacheable = !self.bases.iter().any(|s| s.encoding() == Encoding::Event);
                reader.read_from(&self.bases, from, end, |base, time, value| {
                    consume(base, time, value, cacheable.then_some(Position::Fsdb(time)))
                })
            }
            reader => reader.read(&self.bases, end, |base, time, value| {
                consume(base, time, value, None)
            }),
        }?;
        if let ControlFlow::Break(outcome) = result {
            return outcome;
        }
        // Preserve a successful position even when selected signals are missing.
        let replay = match &self.waveform.reader {
            crate::backends::Reader::Vcd(reader) => reader.position().map(|position| Replay {
                position: Position::Vcd(position),
                start: pending_time.unwrap_or(replay_start),
                end,
                time: pending_time,
                slots: slots.to_vec(),
            }),
            #[cfg(feature = "fsdb-lib")]
            crate::backends::Reader::Fsdb(_)
                if !self.bases.iter().any(|s| s.encoding() == Encoding::Event) =>
            {
                end.ticks().checked_add(1).map(|next| Replay {
                    position: Position::Fsdb(Time::from_ticks(next)),
                    start: pending_time.unwrap_or(replay_start),
                    end,
                    time: pending_time,
                    slots: slots.to_vec(),
                })
            }
            _ => None,
        };
        // Only successful EOF completes the final pending tick.
        let result = if let Some(time) = pending_time {
            complete_tick(
                &self.signals,
                slots,
                &mut active_slots,
                time,
                start,
                &mut visitor,
            )?
        } else {
            ControlFlow::Continue(())
        };
        if result.is_continue() {
            self.replay = replay;
            self.checkpoint = checkpoint;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod streaming_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "fsdb-lib")]
    #[test]
    #[ignore = "requires real FSDB runtime and locked public fixtures"]
    fn fsdb_checkpoint_preserves_projected_state_and_invalidates() {
        let root = std::path::PathBuf::from(std::env::var_os("ONDAS_FIXTURES").unwrap());
        let path = root.join("kleverhq.ondas-fixtures/fsdb0010-history-short/waveform.fsdb");
        let mut wave = crate::open_with(path, "fsdb-lib").unwrap();
        let clock = wave.hierarchy().signal("top.clock").unwrap();
        let word = wave.hierarchy().signal("top.word_00").unwrap();
        let signals = [
            clock,
            word,
            word.slice(0, 0).unwrap(),
            word.slice(31, 16).unwrap(),
            word,
        ];
        let mut selection = wave.select(&signals).unwrap();
        fn count(selection: &Selection<'_>) -> usize {
            let crate::backends::Reader::Fsdb(reader) = &selection.waveform.reader else {
                unreachable!()
            };
            reader.records_read
        }
        let time = Time::from_ticks(4000);
        // Keep an earlier replay to exercise the chronological checkpoint path,
        // independently of the cold bit-only seek.
        let _ = selection
            .scan(TimeRange::point(Time::ZERO), |_| {
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        let _ = selection
            .scan(TimeRange::point(time), |_| ControlFlow::<()>::Continue(()))
            .unwrap();
        let expected = format!("{:?}", selection.samples(time).unwrap());
        let cold = count(&selection);
        assert!(cold > 4000);
        assert!(selection.checkpoint.is_some());
        assert_eq!(format!("{:?}", selection.samples(time).unwrap()), expected);
        assert!(
            count(&selection) - cold < 10,
            "checkpoint must avoid the source prefix"
        );
        // A stable projection retains unknown initial changed_at, not the seek tick.
        let sample = selection.samples(time).unwrap();
        assert!(matches!(
            sample[3],
            Sample::Value {
                changed_at: None,
                ..
            }
        ));
        // Earlier bounds fall back; later access reconstructs the same checkpoint.
        selection.samples(Time::from_ticks(3)).unwrap();
        assert_eq!(format!("{:?}", selection.samples(time).unwrap()), expected);
        let _ = selection
            .scan(TimeRange::all(), |_| ControlFlow::Break(()))
            .unwrap();
        assert!(selection.checkpoint.is_none());
        let _ = selection
            .scan(TimeRange::point(Time::ZERO), |_| {
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        let before = count(&selection);
        let _ = selection
            .scan(TimeRange::point(time), |_| ControlFlow::<()>::Continue(()))
            .unwrap();
        assert_eq!(format!("{:?}", selection.samples(time).unwrap()), expected);
        assert!(count(&selection) - before > 4000);
        let error = selection.query(TimeRange::all(), &[0], |_| -> Result<ControlFlow<()>> {
            Err(Error::Backend {
                backend: "test".into(),
                operation: "query",
                message: "stop".into(),
            })
        });
        assert!(error.is_err());
        assert!(selection.checkpoint.is_none());
        for sample_callback in [true, false] {
            let _ = selection
                .scan(TimeRange::point(time), |_| ControlFlow::<()>::Continue(()))
                .unwrap();
            selection.samples(time).unwrap();
            assert!(selection.checkpoint.is_some());
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if sample_callback {
                    let _ = selection.visit_samples(time, |_| -> ControlFlow<()> {
                        panic!("sample callback after traversal");
                    });
                } else {
                    let _ = selection.scan(
                        TimeRange::point(Time::from_ticks(5000)),
                        |_| -> ControlFlow<()> { panic!("quiet-window initial after traversal") },
                    );
                }
            }));
            assert!(panic.is_err());
            assert!(selection.checkpoint.is_none());
            assert!(selection.replay.is_none());
        }
    }

    #[cfg(feature = "fsdb-lib")]
    #[test]
    #[ignore = "requires real FSDB runtime and locked public fixtures"]
    fn fsdb_cold_window_matches_chronological_reference() {
        let root = std::path::PathBuf::from(std::env::var_os("ONDAS_FIXTURES").unwrap());
        let path = root.join("kleverhq.ondas-fixtures/fsdb0010-history-short/waveform.fsdb");
        let mut wave = crate::open_with(path, "fsdb-lib").unwrap();
        let clock = wave.hierarchy().signal("top.clock").unwrap();
        let word = wave.hierarchy().signal("top.word_00").unwrap();
        let signals = [clock, word, word.slice(31, 16).unwrap(), word];
        let range = TimeRange::closed(Time::from_ticks(4000), Time::from_ticks(4001));
        let mut chronological = wave.select(&signals).unwrap();
        let _ = chronological
            .scan(TimeRange::point(Time::ZERO), |_| {
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        let mut expected = Vec::new();
        let _ = chronological
            .scan_each(range, |index, record| {
                expected.push(format!("{index}:{record:?}"));
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        drop(chronological);
        let crate::backends::Reader::Fsdb(reader) = &wave.reader else {
            unreachable!()
        };
        let before = (reader.records_read, reader.point_queries);
        let mut cold = wave.select(&signals).unwrap();
        let mut actual = Vec::new();
        let _ = cold
            .scan_each(range, |index, record| {
                // The authored source toggles clock every tick and word_00 every
                // 16 ticks; these checks do not derive expectations from Ondas.
                match (index, record) {
                    (
                        0,
                        ScanRef::Initial {
                            value: ValueRef::Bits(bits),
                            changed_at,
                            ..
                        },
                    ) => {
                        assert_eq!(bits.to_string(), "1");
                        assert_eq!(changed_at, Some(Time::from_ticks(3999)));
                    }
                    (
                        1 | 3,
                        ScanRef::Initial {
                            value: ValueRef::Bits(bits),
                            changed_at,
                            ..
                        },
                    ) => {
                        assert_eq!(bits.to_string(), format!("{:032b}", 249));
                        assert_eq!(changed_at, Some(Time::from_ticks(3984)));
                    }
                    (
                        2,
                        ScanRef::Initial {
                            value: ValueRef::Bits(bits),
                            changed_at: None,
                            ..
                        },
                    ) => {
                        assert_eq!(bits.to_string(), "0000000000000000");
                    }
                    (
                        1 | 3,
                        ScanRef::Change {
                            time,
                            value: ValueRef::Bits(bits),
                            ..
                        },
                    ) => {
                        assert_eq!(time, Time::from_ticks(4000));
                        assert_eq!(bits.to_string(), format!("{:032b}", 250));
                    }
                    (
                        0,
                        ScanRef::Change {
                            time,
                            value: ValueRef::Bits(bits),
                            ..
                        },
                    ) => {
                        assert_eq!(
                            bits.to_string(),
                            if time.ticks() % 2 == 0 { "0" } else { "1" }
                        );
                    }
                    _ => panic!("unexpected independent window record {index}: {record:?}"),
                }
                actual.push(format!("{index}:{record:?}"));
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(actual, expected);
        let crate::backends::Reader::Fsdb(reader) = &cold.waveform.reader else {
            unreachable!()
        };
        assert!(
            reader.point_queries > before.1,
            "cold window must seek entering state"
        );
        assert!(
            reader.records_read - before.0 < 100,
            "cold window must not replay the prefix: {} records",
            reader.records_read - before.0
        );
    }

    #[cfg(feature = "fsdb-lib")]
    #[test]
    #[ignore = "requires real FSDB runtime and locked public fixtures"]
    fn fsdb_cold_points_match_chronological_reference() {
        let root = std::path::PathBuf::from(std::env::var_os("ONDAS_FIXTURES").unwrap());
        for fixture in [
            "fsdb0010-history-short",
            "fsdb0017-typed-records",
            "fsdb0019-typed-values",
            "fsdb0020-nine-state-ranges",
        ] {
            let mut wave = crate::open(
                root.join("kleverhq.ondas-fixtures")
                    .join(fixture)
                    .join("waveform.fsdb"),
            )
            .unwrap();
            let mut signals = Vec::new();
            for variable in wave.hierarchy().variables() {
                if let Some(signal) = variable.signal()
                    && let Encoding::Bits { width } = signal.encoding()
                {
                    signals.push(signal);
                    signals.push(signal.slice(0, 0).unwrap());
                    signals.push(signal.slice(width - 1, width - 1).unwrap());
                    signals.push(signal);
                }
            }
            assert!(!signals.is_empty());
            for tick in [0, 1, 3, 4, 5, 16, 2048, 4000, 4001, u64::MAX] {
                let time = Time::from_ticks(tick);
                let mut expected: Vec<_> = signals
                    .iter()
                    .map(|&signal| Sample::Missing { signal })
                    .collect();
                let before = match &wave.reader {
                    crate::backends::Reader::Fsdb(reader) => reader.point_queries,
                    _ => unreachable!(),
                };
                let _ = wave
                    .select(&signals)
                    .unwrap()
                    .scan_each(TimeRange::closed(Time::ZERO, time), |index, record| {
                        let (value, changed_at) = match record {
                            ScanRef::Initial {
                                value, changed_at, ..
                            } => (value, changed_at),
                            ScanRef::Change { value, time, .. } => (
                                value,
                                matches!(expected[index], Sample::Value { .. }).then_some(time),
                            ),
                        };
                        expected[index] = Sample::Value {
                            signal: signals[index],
                            value: value.to_owned(),
                            changed_at,
                        };
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                let chronological_points = match &wave.reader {
                    crate::backends::Reader::Fsdb(reader) => reader.point_queries,
                    _ => unreachable!(),
                };
                assert_eq!(chronological_points, before, "reference must not seek");
                let actual = wave.samples(&signals, time).unwrap();
                assert_eq!(
                    format!("{actual:?}"),
                    format!("{expected:?}"),
                    "{fixture} tick {tick}"
                );
                if tick > 0 && tick < 6001 {
                    let crate::backends::Reader::Fsdb(reader) = &wave.reader else {
                        unreachable!()
                    };
                    assert!(
                        reader.point_queries > chronological_points,
                        "cold point must seek"
                    );
                }
            }
            if fixture == "fsdb0017-typed-records" {
                let glitch = wave.hierarchy().signal("top.glitch").unwrap();
                let sample = wave.sample(glitch, Time::from_ticks(1024)).unwrap();
                assert!(
                    matches!(sample, Sample::Value { changed_at: None, ref value, .. }
                    if matches!(value.as_ref(), ValueRef::Bits(bits) if bits.to_string() == "0")),
                    "independent fixture history returns to the initial zero at tick 1024"
                );
            }
            let range = TimeRange::closed(Time::ZERO, Time::from_ticks(6001));
            let mut expected = Vec::new();
            let _ = wave
                .select(&signals)
                .unwrap()
                .scan_each(range, |index, record| {
                    expected.push(format!("{index}:{record:?}"));
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            let mut selection = wave.select(&signals).unwrap();
            selection.samples(Time::from_ticks(6000)).unwrap();
            selection.samples(Time::from_ticks(6001)).unwrap();
            let mut actual = Vec::new();
            let _ = selection
                .scan_each(range, |index, record| {
                    actual.push(format!("{index}:{record:?}"));
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert_eq!(
                actual, expected,
                "quiet point cache must not be reused as earlier entering state"
            );
        }
    }

    #[cfg(feature = "fsdb-lib")]
    #[test]
    #[ignore = "requires real FSDB runtime and locked public fixtures"]
    fn fsdb_replay_reuses_completed_point_reads() {
        let root = std::path::PathBuf::from(std::env::var_os("ONDAS_FIXTURES").unwrap());
        let path = root.join("kleverhq.ondas-fixtures/fsdb0010-history-short/waveform.fsdb");
        let mut wave = crate::open_with(&path, "fsdb-lib").unwrap();
        let clock = wave.hierarchy().signal("top.clock").unwrap();
        let word = wave.hierarchy().signal("top.word_00").unwrap();
        let signals = [
            clock,
            word.slice(0, 0).unwrap(),
            word.slice(31, 16).unwrap(),
        ];
        let mut selection = wave.select(&signals).unwrap();
        fn count(selection: &Selection<'_>) -> (usize, usize) {
            let crate::backends::Reader::Fsdb(reader) = &selection.waveform.reader else {
                unreachable!()
            };
            (reader.records_read, reader.point_queries)
        }
        let expected = format!("{:?}", selection.samples(Time::from_ticks(4000)).unwrap());
        let before = count(&selection);
        assert_eq!(
            format!("{:?}", selection.samples(Time::from_ticks(4000)).unwrap()),
            expected
        );
        assert_eq!(
            count(&selection),
            before,
            "repeated point must not enter native traversal"
        );
        let next = selection.samples(Time::from_ticks(4001)).unwrap();
        // The SDK can synthesize an unchanged word at the new window start.
        assert!(count(&selection).0 - before.0 <= 2);
        assert!(
            matches!(next[1], Sample::Value { changed_at: Some(time), .. } if time == Time::from_ticks(4000))
        );
        assert!(matches!(
            next[2],
            Sample::Value {
                changed_at: None,
                ..
            }
        ));
        assert_eq!(
            format!("{:?}", selection.samples(Time::from_ticks(4000)).unwrap()),
            expected
        );
        // EOF has no next representable tick at u64::MAX; don't wrap its position.
        selection.samples(Time::from_ticks(u64::MAX)).unwrap();
        assert!(selection.replay.is_none());
        assert_eq!(
            format!("{:?}", selection.samples(Time::from_ticks(4000)).unwrap()),
            expected
        );
        // Event counts require the full reference path, including repeated reads.
        let path = root.join("kleverhq.ondas-fixtures/fsdb0017-typed-records/waveform.fsdb");
        let mut wave = crate::open_with(path, "fsdb-lib").unwrap();
        let event = wave.hierarchy().signal("top.trigger").unwrap();
        let mut selection = wave.select(&[event]).unwrap();
        let expected = format!("{:?}", selection.samples(Time::from_ticks(2048)).unwrap());
        assert_eq!(
            format!("{:?}", selection.samples(Time::from_ticks(2048)).unwrap()),
            expected
        );
        assert!(selection.replay.is_none());
        assert!(selection.checkpoint.is_none());
    }

    #[test]
    fn vcd_replay_avoids_prefix_and_invalidates_on_stop() {
        let mut text = String::from("$var wire 1 ! bit $end $enddefinitions $end ");
        for tick in 0..100 {
            text.push_str(&format!("#{tick} {}! ", tick % 2));
        }
        let mut wave =
            crate::open_bytes_with("replay.vcd", text.into_bytes().into(), "vcd-native").unwrap();
        let signal = wave.hierarchy().signal("bit").unwrap();
        let mut selection = wave.select(&[signal]).unwrap();
        fn count(selection: &Selection<'_>) -> usize {
            let crate::backends::Reader::Vcd(reader) = &selection.waveform.reader else {
                unreachable!()
            };
            reader.records_read
        }
        let before = count(&selection);
        selection.samples(Time::from_ticks(90)).unwrap();
        assert_eq!(count(&selection) - before, 91);
        let before = count(&selection);
        selection.samples(Time::from_ticks(90)).unwrap();
        assert_eq!(count(&selection), before);
        selection.samples(Time::from_ticks(92)).unwrap();
        assert_eq!(count(&selection) - before, 2);
        let before = count(&selection);
        selection.samples(Time::from_ticks(91)).unwrap();
        assert_eq!(count(&selection) - before, 92);
        let _ = selection
            .visit_samples(Time::from_ticks(92), |_| ControlFlow::Break(()))
            .unwrap();
        assert!(selection.replay.is_none());
        let _ = selection
            .query(TimeRange::all(), &[0], |_| Ok(ControlFlow::Break(())))
            .unwrap();
        assert!(selection.replay.is_none());
        let error = selection.query(TimeRange::all(), &[0], |_| -> Result<ControlFlow<()>> {
            Err(Error::Backend {
                backend: "test".into(),
                operation: "query",
                message: "stop".into(),
            })
        });
        assert!(error.is_err());
        assert!(selection.replay.is_none());
        let before = count(&selection);
        selection.samples(Time::from_ticks(92)).unwrap();
        assert_eq!(count(&selection) - before, 93);
    }

    #[test]
    fn vcd_missing_state_reuses_position_and_initial_break_discards_it() {
        let mut text = String::from(
            "$var wire 1 ! bit $end $var wire 1 m missing $end $var wire 1 d delayed $end $enddefinitions $end ",
        );
        for tick in 0..100 {
            text.push_str(&format!("#{tick} {}! ", tick % 2));
        }
        text.push_str("#100 1d");
        let mut wave =
            crate::open_bytes_with("missing.vcd", text.into_bytes().into(), "vcd-native").unwrap();
        let signals = [
            wave.hierarchy().signal("missing").unwrap(),
            wave.hierarchy().signal("delayed").unwrap(),
        ];
        let mut selection = wave.select(&signals).unwrap();
        selection.samples(Time::from_ticks(90)).unwrap();
        let count = match &selection.waveform.reader {
            crate::backends::Reader::Vcd(reader) => reader.records_read,
            _ => unreachable!(),
        };
        selection.samples(Time::from_ticks(90)).unwrap();
        selection.samples(Time::from_ticks(92)).unwrap();
        let crate::backends::Reader::Vcd(reader) = &selection.waveform.reader else {
            unreachable!()
        };
        assert_eq!(reader.records_read - count, 2);
        let samples = selection.samples(Time::from_ticks(100)).unwrap();
        assert!(matches!(samples[0], Sample::Missing { .. }));
        assert!(matches!(
            samples[1],
            Sample::Value {
                changed_at: None,
                ..
            }
        ));
        let _ = selection
            .scan_each(
                TimeRange::closed(Time::from_ticks(101), Time::from_ticks(102)),
                |_, _| ControlFlow::Break(()),
            )
            .unwrap();
        assert!(selection.replay.is_none());
        selection.samples(Time::from_ticks(102)).unwrap();
        assert!(selection.replay.is_some());
    }

    #[test]
    fn vcd_boundary_checkpoint_skips_prefix_and_is_bounded() {
        let mut text = String::from("$var wire 1 ! bit $end $enddefinitions $end ");
        for tick in 0..1000 {
            text.push_str(&format!("#{tick} {}! ", tick % 2));
        }
        let mut wave =
            crate::open_bytes_with("checkpoint.vcd", text.into_bytes().into(), "vcd-native")
                .unwrap();
        let signal = wave.hierarchy().signal("bit").unwrap();
        let mut selection = wave.select(&[signal]).unwrap();
        let range = TimeRange::closed(Time::from_ticks(900), Time::from_ticks(910));
        selection.traces(range).unwrap();
        let bytes = checkpoint_bytes(&selection.checkpoint.as_ref().unwrap().slots);
        eprintln!("single-signal checkpoint storage: {bytes} bytes");
        assert!(bytes <= CHECKPOINT_BYTES);
        let before = match &selection.waveform.reader {
            crate::backends::Reader::Vcd(reader) => reader.records_read,
            _ => unreachable!(),
        };
        selection.traces(range).unwrap();
        let crate::backends::Reader::Vcd(reader) = &selection.waveform.reader else {
            unreachable!()
        };
        assert_eq!(reader.records_read - before, 11);
        assert_eq!(
            checkpoint_bytes(&selection.checkpoint.as_ref().unwrap().slots),
            bytes
        );
        let _ = selection
            .query(range, &[0], |_| Ok(ControlFlow::Break(())))
            .unwrap();
        assert!(selection.checkpoint.is_none());
        // An oversized selected value does not become an oversized index entry.
        let text = format!(
            "$var wire {} ! wide $end $enddefinitions $end #0 b0 ! #1 b1 !",
            CHECKPOINT_BYTES
        );
        let mut wave =
            crate::open_bytes_with("budget.vcd", text.into_bytes().into(), "vcd-native").unwrap();
        let signal = wave.hierarchy().signal("wide").unwrap();
        let mut selection = wave.select(&[signal]).unwrap();
        selection.samples(Time::from_ticks(1)).unwrap();
        assert!(selection.checkpoint.is_none());
        assert!(selection.replay.is_some()); // bounded selected state, not index storage
    }

    #[test]
    fn completed_tick_moves_pending_storage_after_the_callback() {
        let pending: Box<str> = "x".repeat(4096).into();
        let address = pending.as_ptr();
        let mut slots = [Slot {
            state: Some(State {
                value: Value::String("old".into()),
                changed_at: None,
            }),
            pending: Some(Value::String(pending)),
            ..Slot::default()
        }];
        let _ = complete_tick(
            &[],
            &mut slots,
            &mut vec![0],
            Time::from_ticks(1),
            None,
            &mut |_, _, slots| {
                assert!(matches!(
                    slots[0].state.as_ref().unwrap().value.as_ref(),
                    ValueRef::String("old")
                ));
                assert!(slots[0].changed);
                Ok(ControlFlow::<()>::Continue(()))
            },
        )
        .unwrap();
        let Value::String(retained) = &slots[0].state.as_ref().unwrap().value else {
            panic!("string")
        };
        assert_eq!(
            retained.as_ptr(),
            address,
            "commit must not copy the pending payload"
        );
        assert_eq!(
            slots[0].state.as_ref().unwrap().changed_at,
            Some(Time::from_ticks(1))
        );
        assert!(slots[0].pending.is_none());
    }

    #[test]
    fn zero_events_do_not_activate_duplicate_slots() {
        let mut records = vec![(0, 0, Value::Event { occurrences: 0 }); 100];
        records.push((0, 1, Value::Event { occurrences: 2 }));
        let mut wave = Waveform::memory(vec![Encoding::Event], records, None);
        let signal = wave.hierarchy().signals().next().unwrap();
        assert!(matches!(
            wave.sample(signal, Time::from_ticks(1)).unwrap(),
            Sample::Event { occurrences: 2, .. }
        ));
    }

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
