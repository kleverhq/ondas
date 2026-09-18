//! FST benchmark consumers, also exercised by fixture-backed conformance tests.
use std::ops::ControlFlow;

use ondas::{Logic, Result, SampleRef, ScanRef, Selection, Time, TimeRange, Value, ValueRef};

fn high(value: ValueRef<'_>) -> bool {
    matches!(value, ValueRef::Bits(bits) if bits.width() == 1 && bits.bit(0) == Some(Logic::One))
}

fn accepts(time: Time, control: Option<&Value>, stride: u64) -> bool {
    time.ticks() % stride == 1 && control.is_some_and(|value| high(value.as_ref()))
}

/// Slots are driver, one-bit control, payload; both scalar entries drive the
/// same candidate union in these fixtures. `limit` counts accepted results,
/// not candidate callbacks. The caller chooses sparse acceptance by absolute tick.
pub fn conditional_query(
    selection: &mut Selection<'_>,
    range: TimeRange,
    stride: u64,
    limit: usize,
    eager: bool,
    mut consume: impl FnMut(Time, ValueRef<'_>),
) -> Result<usize> {
    let mut accepted = 0;
    let _ = selection.query(range, &[0, 1], |context| {
        let time = context.time();
        let mut enabled = false;
        let _ = context.visit_samples(time, &[1], |_, sample| {
            enabled = matches!(sample, SampleRef::Value { value, .. } if high(value));
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        let pass = enabled && time.ticks() % stride == 1;
        if eager || pass {
            let _ = context.visit_samples(time, &[2], |_, sample| {
                if let SampleRef::Value { value, .. } = sample {
                    // Eager mode deliberately owns even rejected payloads.
                    if eager {
                        let owned = value.to_owned();
                        std::hint::black_box(&owned);
                        if pass {
                            consume(time, owned.as_ref());
                        }
                    } else {
                        consume(time, value);
                    }
                }
                Ok(ControlFlow::<()>::Continue(()))
            })?;
        }
        if pass {
            accepted += 1;
            if accepted == limit {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    })?;
    Ok(accepted)
}

/// A bounded, normalized scan consumer: retain only the three current values
/// and finish each tick before evaluating the same condition as the query.
pub fn conditional_scan(
    selection: &mut Selection<'_>,
    range: TimeRange,
    stride: u64,
    limit: usize,
    mut consume: impl FnMut(Time, ValueRef<'_>),
) -> Result<usize> {
    let mut values: [Option<Value>; 3] = [None, None, None];
    let mut tick = None;
    let mut driver_changed = false;
    let mut accepted = 0;
    let mut finish = |time, driver_changed, values: &[Option<Value>; 3]| {
        if driver_changed && accepts(time, values[1].as_ref(), stride) {
            if let Some(value) = &values[2] {
                consume(time, value.as_ref());
            }
            accepted += 1;
            if accepted == limit {
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    };
    let flow = selection.scan_each(range, |slot, record| {
        let value = match record {
            ScanRef::Initial { value, .. } => value,
            ScanRef::Change { time, value, .. } => {
                if tick != Some(time) {
                    if let Some(previous) = tick
                        && finish(previous, driver_changed, &values).is_break()
                    {
                        return ControlFlow::Break(());
                    }
                    tick = Some(time);
                    driver_changed = false;
                }
                driver_changed |= slot < 2;
                value
            }
            _ => unreachable!("unsupported scan record"),
        };
        values[slot] = Some(value.to_owned());
        ControlFlow::Continue(())
    })?;
    if flow.is_continue()
        && let Some(time) = tick
    {
        let _ = finish(time, driver_changed, &values);
    }
    Ok(accepted)
}

/// Stop at the first high scalar, allowing earlier candidates to be rejected.
pub fn first_high(selection: &mut Selection<'_>, range: TimeRange) -> Result<ControlFlow<Time>> {
    selection.query(range, &[0], |context| {
        let mut enabled = false;
        let _ = context.visit_samples(context.time(), &[0], |_, sample| {
            enabled = matches!(sample, SampleRef::Value { value, .. } if high(value));
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        Ok(if enabled {
            ControlFlow::Break(context.time())
        } else {
            ControlFlow::Continue(())
        })
    })
}

/// Read two selected signals at t-1, t, t-1 at every driver candidate.
/// At zero only the current tick is available. The consumer observes each read.
pub fn adjacent_query(
    selection: &mut Selection<'_>,
    range: TimeRange,
    mut consume: impl FnMut(Time, SampleRef<'_>),
) -> Result<usize> {
    let mut reads = 0;
    let _ = selection.query(range, &[0], |context| {
        let time = context.time();
        let previous = time.ticks().checked_sub(1).map(Time::from_ticks);
        for at in [previous, Some(time), previous].into_iter().flatten() {
            let _ = context.visit_samples(at, &[0, 1], |_, sample| {
                consume(at, sample);
                reads += 1;
                Ok(ControlFlow::<()>::Continue(()))
            })?;
        }
        Ok(ControlFlow::<()>::Continue(()))
    })?;
    Ok(reads)
}
