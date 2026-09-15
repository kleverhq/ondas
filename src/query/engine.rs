use super::*;
use crate::{Encoding, Error};

struct State {
    value: Value,
    changed_at: Option<Time>,
}

#[derive(Default)]
struct Slot {
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

fn flush_tick<B>(
    signals: &[Signal],
    slots: &mut [Slot],
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
        if let Some(value) = slot.pending.take()
            && update(&mut slot.state, value.as_ref(), time)
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
        let events = std::mem::take(&mut slot.events);
        if time >= range.start() {
            for _ in 0..events {
                visitor(
                    index,
                    ScanRef::Change {
                        signal,
                        time,
                        value: ValueRef::Event,
                    },
                )?;
            }
        }
    }
    ControlFlow::Continue(())
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
                    if matches!(value, ValueRef::Event) {
                        // The normalizer already checked this slot's per-tick count.
                        events[index] += 1;
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

    pub(super) fn scan_each<B>(
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
        let mut pending_time = None;
        let backend = self.waveform.backend().to_owned();
        // One entering/final value and event count per slot, never intra-tick writes.
        let result = self
            .waveform
            .reader
            .read(&self.bases, end, |base, time, value| {
                if let Some(previous) = pending_time
                    && previous != time
                {
                    flush_tick(
                        &self.signals,
                        &mut slots,
                        previous,
                        range,
                        &mut emitted_initials,
                        &mut visitor,
                    )
                    .map_break(Ok)?;
                }
                pending_time = Some(time);
                for &index in &self.groups[&base] {
                    let slot = &mut slots[index];
                    if matches!(value, ValueRef::Event) {
                        let Some(count) = slot.events.checked_add(1) else {
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
                ControlFlow::Continue(())
            })?;
        if let ControlFlow::Break(value) = result {
            return value.map(ControlFlow::Break);
        }
        // Only successful EOF completes the final pending tick.
        if let Some(time) = pending_time
            && let ControlFlow::Break(value) = flush_tick(
                &self.signals,
                &mut slots,
                time,
                range,
                &mut emitted_initials,
                &mut visitor,
            )
        {
            return Ok(ControlFlow::Break(value));
        }
        if !emitted_initials {
            return Ok(initials(&self.signals, &slots, &mut visitor));
        }
        Ok(ControlFlow::Continue(()))
    }
}
