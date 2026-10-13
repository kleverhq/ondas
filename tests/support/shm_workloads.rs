//! Concrete SHM benchmark consumers checked independently by conformance.
use std::ops::ControlFlow;

use ondas::{Logic, Result, SampleRef, ScanRef, Selection, Time, TimeRange, ValueRef};

pub const BACKEND: &str = "shm-lib";
pub const HISTORY: [(&str, u64); 2] = [
    ("shm/shm0080-history-short", 4096),
    ("shm/shm0081-history-long", 1_048_576),
];
pub const BANK: &str = "shm/shm0082-sparse-dense-bank";
pub const TOPOLOGY: &str = "shm/shm0083-topology-many-handles";
pub const WIDE: &str = "shm/shm0084-wide-compact-toggle";
pub const TYPED: &str = "shm/shm0085-typed-records";

pub fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

pub fn scan_count(selection: &mut Selection<'_>, range: TimeRange) -> Result<u64> {
    let mut count = 0;
    let _ = selection.scan(range, |_| {
        count += 1;
        ControlFlow::<()>::Continue(())
    })?;
    Ok(count)
}

pub fn candidate_count(selection: &mut Selection<'_>, range: TimeRange) -> Result<u64> {
    let mut count = 0;
    let _ = selection.scan_candidate_times(range, |_| {
        count += 1;
        ControlFlow::<()>::Continue(())
    })?;
    Ok(count)
}

pub fn first_change(selection: &mut Selection<'_>, range: TimeRange) -> Result<ControlFlow<Time>> {
    selection.scan(range, |record| match record {
        ScanRef::Change { time, .. } => ControlFlow::Break(time),
        _ => ControlFlow::Continue(()),
    })
}

pub fn series(
    selection: &mut Selection<'_>,
    times: &[Time],
    mut observe: impl FnMut(Time, usize, SampleRef<'_>),
) -> Result<u64> {
    let mut count = 0;
    for &time in times {
        let mut index = 0;
        let _ = selection.visit_samples(time, |sample| {
            count += 1;
            observe(time, index, sample);
            index += 1;
            ControlFlow::<()>::Continue(())
        })?;
    }
    Ok(count)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub candidates: u64,
    pub accepted: u64,
}

// Slots are scalar driver, scalar control and wide payload. Eager access reads
// the payload at every candidate. Selective access borrows it only when accepted.
// Both paths can decode/retain the native payload before the public callback.
pub fn payload(
    selection: &mut Selection<'_>,
    range: TimeRange,
    period: u64,
    eager: bool,
    mut observe: impl FnMut(Time, ValueRef<'_>) -> ControlFlow<()>,
) -> Result<Counts> {
    let mut counts = Counts::default();
    // The repeated driver also exercises public candidate deduplication.
    let _ = selection.query(range, &[0, 1, 0], |ctx| {
        counts.candidates += 1;
        let time = ctx.time();
        let mut accept = false;
        let slots = if eager { &[1, 2][..] } else { &[1][..] };
        let outcome = ctx.visit_samples(time, slots, |index, sample| {
            let SampleRef::Value { value, .. } = sample else {
                panic!("wide workload requires persistent recorded values");
            };
            if index == 1 {
                accept = matches!(value, ValueRef::Bits(bits) if bits.bit(0) == Some(Logic::One))
                    && time.ticks() % period == 1;
            } else if accept {
                counts.accepted += 1;
                return Ok(observe(time, value));
            }
            Ok(ControlFlow::Continue(()))
        })?;
        if outcome.is_break() || eager || !accept {
            return Ok(outcome);
        }
        counts.accepted += 1;
        ctx.visit_samples(time, &[2], |_, sample| {
            let SampleRef::Value { value, .. } = sample else {
                panic!("wide workload payload must be recorded");
            };
            Ok(observe(time, value))
        })
    })?;
    Ok(counts)
}
