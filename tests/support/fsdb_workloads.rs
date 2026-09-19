//! Concrete FSDB benchmark consumers, also exercised outside timing by conformance.
use std::ops::ControlFlow;

use ondas::{
    Logic, Result, SampleRef, ScanRef, Selection, Signal, Time, TimeRange, Value, ValueRef,
    Waveform,
};

pub const BACKEND: &str = "fsdb-lib";
pub const WIDE: &str = "fsdb0015-wide-compact-toggle";

// Slots are driver, control, payload. Sharing duplicates a base identity; the
// independent control is a projection of a different, simultaneously active base.
pub fn wide_signals(wave: &Waveform, shared: bool) -> [Signal; 3] {
    let driver = wave.hierarchy().signal("top.control").unwrap();
    let control = if shared {
        driver
    } else {
        wave.hierarchy()
            .signal("top.wide_msb")
            .unwrap()
            .slice(4095, 4095)
            .unwrap()
    };
    [
        driver,
        control,
        wave.hierarchy().signal("top.wide").unwrap(),
    ]
}

pub fn high(value: ValueRef<'_>) -> bool {
    matches!(value, ValueRef::Bits(bits) if bits.bit(0) == Some(Logic::One))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub candidates: u64,
    pub accepted: u64,
}

// The predicate runs only after reading control; payload is borrowed only when
// accepted. This says nothing about values already decoded/retained by the reader.
pub fn conditional(
    selection: &mut Selection<'_>,
    range: TimeRange,
    drivers: &[usize],
    mut accept: impl FnMut(Time, bool) -> bool,
    mut observe: impl FnMut(Time, ValueRef<'_>) -> ControlFlow<()>,
) -> Result<Counts> {
    let mut counts = Counts::default();
    let _ = selection.query(range, drivers, |ctx| {
        counts.candidates += 1;
        let mut control = false;
        let _ = ctx.visit_samples(ctx.time(), &[1], |_, sample| {
            if let SampleRef::Value { value, .. } = sample {
                control = high(value);
            }
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        if !accept(ctx.time(), control) {
            return Ok(ControlFlow::Continue(()));
        }
        counts.accepted += 1;
        ctx.visit_samples(ctx.time(), &[2], |_, sample| {
            let SampleRef::Value { value, .. } = sample else {
                panic!("wide fixture payload must be persistent and present");
            };
            Ok(observe(ctx.time(), value))
        })
    })?;
    Ok(counts)
}

// Bounded sequential comparison: retain just the three current values, finish
// each tick before testing control, and never collect a candidate/output list.
// This consumer is intentionally for the persistent-bit wide fixture only.
pub fn sequential(
    selection: &mut Selection<'_>,
    range: TimeRange,
    drivers: &[usize],
    mut accept: impl FnMut(Time, bool) -> bool,
    mut observe: impl FnMut(Time, ValueRef<'_>) -> ControlFlow<()>,
) -> Result<Counts> {
    let mut values: [Option<Value>; 3] = [None, None, None];
    let mut tick = None;
    let mut candidate = false;
    let mut counts = Counts::default();
    let mut finish = |time, candidate, values: &[Option<Value>; 3]| {
        if candidate {
            counts.candidates += 1;
            let control = values[1].as_ref().is_some_and(|value| high(value.as_ref()));
            if accept(time, control) {
                counts.accepted += 1;
                return observe(time, values[2].as_ref().expect("wide payload").as_ref());
            }
        }
        ControlFlow::Continue(())
    };
    let outcome = selection.scan_each(range, |index, record| {
        match record {
            ScanRef::Initial { value, .. } => values[index] = Some(value.to_owned()),
            ScanRef::Change { time, value, .. } => {
                if let Some(previous) = tick
                    && previous != time
                {
                    if finish(previous, candidate, &values).is_break() {
                        return ControlFlow::Break(());
                    }
                    candidate = false;
                }
                tick = Some(time);
                values[index] = Some(value.to_owned());
                candidate |= drivers.contains(&index);
            }
            _ => unreachable!("persistent-bit scan record"),
        }
        ControlFlow::Continue(())
    })?;
    if outcome.is_continue()
        && let Some(time) = tick
    {
        let _ = finish(time, candidate, &values);
    }
    Ok(counts)
}

pub fn adjacent(
    selection: &mut Selection<'_>,
    range: TimeRange,
    drivers: &[usize],
    readable: &[usize],
    mut observe: impl FnMut(Time, usize, SampleRef<'_>),
) -> Result<u64> {
    let mut candidates = 0;
    let _ = selection.query(range, drivers, |ctx| {
        candidates += 1;
        let previous = ctx.time().ticks().checked_sub(1).map(Time::from_ticks);
        // At zero there is no predecessor. Else exercise t-1 -> t -> t-1,
        // not the previous candidate's timestamp.
        for time in [previous, Some(ctx.time()), previous].into_iter().flatten() {
            let _ = ctx.visit_samples(time, readable, |index, sample| {
                observe(time, index, sample);
                Ok(ControlFlow::<()>::Continue(()))
            })?;
        }
        Ok(ControlFlow::<()>::Continue(()))
    })?;
    Ok(candidates)
}
