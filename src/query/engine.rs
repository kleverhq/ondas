use super::*;
use crate::{Encoding, Error};

struct State {
    value: Value,
    changed_at: Option<Time>,
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
    states: &[Option<State>],
    visitor: &mut impl for<'v> FnMut(usize, ScanRef<'v>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    for (index, (signal, state)) in signals.iter().zip(states).enumerate() {
        if let Some(state) = state
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
        let _ = self
            .waveform
            .reader
            .read(&self.bases, time, |base, tick, value| {
                for &index in &self.groups[&base] {
                    if matches!(value, ValueRef::Event) {
                        if tick == time {
                            events[index] += 1;
                        }
                    } else {
                        update(
                            &mut states[index],
                            project(value, self.signals[index]),
                            tick,
                        );
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
        let mut states = (0..self.signals.len())
            .map(|_| None)
            .collect::<Vec<Option<State>>>();
        let mut emitted_initials = false;
        // Only one prior value per selection entry is retained, not whole histories.
        // Reader traversal repeats per query; selection reuses validated handles and grouping.
        let result = self
            .waveform
            .reader
            .read(&self.bases, end, |base, time, value| {
                if !emitted_initials && time >= range.start() {
                    emitted_initials = true;
                    if let ControlFlow::Break(value) =
                        initials(&self.signals, &states, &mut visitor)
                    {
                        return ControlFlow::Break(value);
                    }
                }
                for &index in &self.groups[&base] {
                    let signal = self.signals[index];
                    let value = project(value, signal);
                    let changed =
                        matches!(value, ValueRef::Event) || update(&mut states[index], value, time);
                    if changed
                        && time >= range.start()
                        && let ControlFlow::Break(value) = visitor(
                            index,
                            ScanRef::Change {
                                signal,
                                time,
                                value,
                            },
                        )
                    {
                        return ControlFlow::Break(value);
                    }
                }
                ControlFlow::Continue(())
            })?;
        match result {
            ControlFlow::Break(value) => Ok(ControlFlow::Break(value)),
            ControlFlow::Continue(()) if !emitted_initials => {
                Ok(initials(&self.signals, &states, &mut visitor))
            }
            ControlFlow::Continue(()) => Ok(ControlFlow::Continue(())),
        }
    }
}
