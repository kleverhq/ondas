//! Bounded consumers shared by the VCD baseline and its observation checks.
use std::ops::ControlFlow;

use ondas::{
    Logic, Result, SampleRef, ScanRef, Selection, Signal, Time, TimeRange, Value, ValueRef,
    Waveform,
};

pub const FIXTURE: &str = "vcd0071-swerv1";
pub const BACKEND: &str = "vcd-native";
pub const PATHS: [&str; 4] = [
    "TOP.core_clk",
    "TOP.tb_top.cycleCnt",
    "TOP.tb_top.WriteData",
    "TOP.tb_top.commit_count",
];

pub fn signals(wave: &Waveform) -> [Signal; 4] {
    PATHS.map(|path| wave.hierarchy().signal(path).expect("VCD workload signal"))
}

pub fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

fn low_word(value: ValueRef<'_>) -> Option<u32> {
    let ValueRef::Bits(bits) = value else {
        return None;
    };
    let mut word = 0;
    for bit in 0..bits.width().min(32) {
        match bits.bit(bit)? {
            Logic::Zero => (),
            Logic::One => word |= 1 << bit,
            _ => return None,
        }
    }
    Some(word)
}

fn sample_word(sample: SampleRef<'_>) -> Option<u32> {
    match sample {
        SampleRef::Value { value, .. } => low_word(value),
        _ => None,
    }
}

/// Several reads in one session window, including repeated reads of t-1.
pub fn temporal(
    selection: &mut Selection<'_>,
    range: TimeRange,
    mut observe: impl FnMut(Time, usize, SampleRef<'_>),
) -> Result<()> {
    let _ = selection.query(range, &[0], |context| {
        let time = context.time();
        if let Some(previous) = time.ticks().checked_sub(1).map(Time::from_ticks) {
            for at in [previous, time, previous] {
                let _ = context.visit_samples(at, &[0, 1, 2], |slot, sample| {
                    observe(at, slot, sample);
                    Ok(ControlFlow::<()>::Continue(()))
                })?;
            }
        } else {
            let _ = context.visit_samples(time, &[0, 1, 2], |slot, sample| {
                observe(time, slot, sample);
                Ok(ControlFlow::<()>::Continue(()))
            })?;
        }
        Ok(ControlFlow::<()>::Continue(()))
    })?;
    Ok(())
}

/// Rising clock, native counter predicate, pre-edge payload. Mask zero accepts
/// every rising edge; larger masks keep the artifact and selected work fixed.
pub fn conditional(
    selection: &mut Selection<'_>,
    range: TimeRange,
    drivers: &[usize],
    mask: u32,
    mut emit: impl FnMut(Time, ValueRef<'_>) -> ControlFlow<()>,
) -> Result<(u64, u64)> {
    let (mut candidates, mut accepted) = (0, 0);
    let _ = selection.query(range, drivers, |context| {
        candidates += 1;
        let time = context.time();
        let Some(previous) = time.ticks().checked_sub(1).map(Time::from_ticks) else {
            return Ok(ControlFlow::Continue(()));
        };
        let mut before = None;
        let _ = context.visit_samples(previous, &[0], |_, sample| {
            before = sample_word(sample);
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        let mut current = [None; 2];
        let _ = context.visit_samples(time, &[0, 1], |slot, sample| {
            current[slot] = sample_word(sample);
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        if before == Some(0)
            && current[0] == Some(1)
            && current[1].is_some_and(|word| word & mask == mask)
        {
            return context.visit_samples(previous, &[2], |_, sample| {
                if let SampleRef::Value { value, .. } = sample {
                    accepted += 1;
                    return Ok(emit(time, value));
                }
                Ok(ControlFlow::Continue(()))
            });
        }
        Ok(ControlFlow::Continue(()))
    })?;
    Ok((candidates, accepted))
}

/// Independent bounded sequential consumer: retain one state per slot, process
/// a complete scan tick, and use the old payload before applying that tick.
pub fn conditional_scan(
    selection: &mut Selection<'_>,
    range: TimeRange,
    mask: u32,
    mut emit: impl FnMut(Time, ValueRef<'_>) -> ControlFlow<()>,
) -> Result<u64> {
    let mut states: [Option<Value>; 4] = Default::default();
    let mut pending: [Option<Value>; 4] = Default::default();
    let mut tick = None;
    let mut accepted = 0;
    let mut finish = |time, states: &mut [Option<Value>; 4], pending: &mut [Option<Value>; 4]| {
        let word = |slot: usize| {
            pending[slot]
                .as_ref()
                .or(states[slot].as_ref())
                .and_then(|v| low_word(v.as_ref()))
        };
        let before = states[0].as_ref().and_then(|v| low_word(v.as_ref()));
        let mut stop = ControlFlow::Continue(());
        if before == Some(0)
            && word(0) == Some(1)
            && word(1).is_some_and(|word| word & mask == mask)
            && let Some(payload) = &states[2]
        {
            accepted += 1;
            stop = emit(time, payload.as_ref());
        }
        for (state, next) in states.iter_mut().zip(pending) {
            if let Some(value) = next.take() {
                *state = Some(value);
            }
        }
        stop
    };
    let stopped = selection.scan_each(range, |slot, record| {
        match record {
            ScanRef::Initial { value, .. } => states[slot] = Some(value.to_owned()),
            ScanRef::Change { time, value, .. } => {
                if let Some(previous) = tick
                    && previous != time
                    && finish(previous, &mut states, &mut pending).is_break()
                {
                    return ControlFlow::Break(());
                }
                tick = Some(time);
                pending[slot] = Some(value.to_owned());
            }
            _ => panic!("unexpected scan record"),
        }
        ControlFlow::Continue(())
    })?;
    if stopped.is_continue()
        && let Some(time) = tick
    {
        let _ = finish(time, &mut states, &mut pending);
    }
    Ok(accepted)
}
