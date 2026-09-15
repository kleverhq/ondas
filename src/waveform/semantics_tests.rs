use super::*;
use crate::{Bits, Encoding, SampleRef, ScanRef, Trace, Value, ValueRef};
use std::{collections::BTreeSet, ops::ControlFlow};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Bits(String),
    Real(u64),
    String(String),
    Event(u64),
}

type History = Vec<(u64, Key)>;
type Row = (usize, bool, Option<u64>, Key);
// base history, optional normalized projection; aliases occupy separate slots.
const SLOTS: [(usize, Option<(u32, u32)>); 10] = [
    (0, None),
    (0, None),
    (0, Some((8, 0))),
    (0, Some((6, 2))),
    (0, Some((4, 0))),
    (1, None),
    (2, None),
    (3, None),
    (3, None),
    (4, None),
];

fn key(value: ValueRef<'_>) -> Key {
    match value {
        ValueRef::Bits(bits) => Key::Bits(bits.to_string()),
        ValueRef::Real(real) => Key::Real(real.to_bits()),
        ValueRef::String(text) => Key::String(text.into()),
        ValueRef::Event { occurrences } => Key::Event(occurrences),
    }
}

fn bits(text: &str) -> Value {
    Value::Bits(Bits::from_ascii(text.as_bytes()).unwrap())
}

fn fixture(reverse: bool, extra: bool) -> (Waveform, Vec<Signal>, Vec<History>) {
    let encodings = vec![
        Encoding::Bits { width: 9 },
        Encoding::Real,
        Encoding::String,
        Encoding::Event,
        Encoding::Real,
    ];
    let mut records = vec![
        (0, 7, bits("000000000")),
        (0, 7, bits("ux01zwlh-")),
        (1, 7, Value::Real(f64::from_bits(0x7ff8_0000_0000_0001))),
        (2, 7, Value::String("röd".into())),
        (3, 7, Value::Event { occurrences: 1 }),
        (3, 7, Value::Event { occurrences: 1 }),
        (1, 8, Value::Real(f64::from_bits(0x7ff8_0000_0000_0002))),
        (0, 9, bits("000000000")),
        (0, 9, bits("111111111")),
        (1, 9, Value::Real(0.0)),
        (0, 10, bits("000000000")),
        (0, 10, bits("111111111")),
        (1, 10, Value::Real(-0.0)),
        (2, 11, Value::String("payload\0 ".into())),
        (3, 11, Value::Event { occurrences: 1 }),
        (3, 11, Value::Event { occurrences: 1 }),
        (3, 11, Value::Event { occurrences: 1 }),
        (2, 12, Value::String("payload\0 ".into())),
        (0, 14, bits("001101011")),
        (1, 14, Value::Real(f64::INFINITY)),
        (3, 14, Value::Event { occurrences: 1 }),
        (3, 14, Value::Event { occurrences: 1 }),
        (3, 14, Value::Event { occurrences: 1 }),
    ];
    if extra {
        records.push((0, 12, bits("111111111")));
    }
    // Stable sorting changes only cross-signal order at a tick.
    records.sort_by_key(|(id, tick, _)| (*tick, if reverse { usize::MAX - id } else { *id }));
    let mut wave = Waveform::memory(encodings.clone(), records, None);
    let variables = [
        ("bus", 0),
        ("alias", 0),
        ("real", 1),
        ("string", 2),
        ("event", 3),
        ("event_alias", 3),
        ("never", 4),
    ]
    .into_iter()
    .map(|(name, signal)| crate::hierarchy::VariableData {
        name: name.into(),
        parent: None,
        kind: "variable".into(),
        direction: crate::Direction::Unknown,
        range: None,
        is_constant: false,
        type_name: None,
        signedness: None,
        logic_domain: None,
        enumeration: None,
        signal: Some(signal),
    })
    .collect();
    wave.hierarchy = Hierarchy::new(vec![], variables, encodings);
    let bases = wave.hierarchy().signals().collect::<Vec<_>>();
    assert_eq!(
        wave.hierarchy().signal("bus").unwrap(),
        wave.hierarchy().signal("alias").unwrap()
    );
    assert_eq!(
        wave.hierarchy().signal("event").unwrap(),
        wave.hierarchy().signal("event_alias").unwrap()
    );
    // Explicit normalized histories, authored separately from the raw observations.
    let base_histories = [
        vec![
            (7, Key::Bits("ux01zwlh-".into())),
            (9, Key::Bits("111111111".into())),
            (14, Key::Bits("001101011".into())),
        ],
        vec![
            (7, Key::Real(0x7ff8_0000_0000_0001)),
            (8, Key::Real(0x7ff8_0000_0000_0002)),
            (9, Key::Real(0)),
            (10, Key::Real(0x8000_0000_0000_0000)),
            (14, Key::Real(f64::INFINITY.to_bits())),
        ],
        vec![
            (7, Key::String("röd".into())),
            (11, Key::String("payload\0 ".into())),
        ],
        vec![(7, Key::Event(2)), (11, Key::Event(3)), (14, Key::Event(3))],
        vec![],
    ];
    let mut selected = Vec::new();
    let mut histories = Vec::new();
    for (base, projection) in SLOTS {
        selected.push(projection.map_or(bases[base], |(msb, lsb)| {
            bases[base].slice(msb, lsb).unwrap()
        }));
        let mut history: History = Vec::new();
        for (tick, value) in &base_histories[base] {
            let projected = if let Some((msb, lsb)) = projection {
                let Key::Bits(text) = value else {
                    unreachable!()
                };
                Key::Bits(text[text.len() - msb as usize - 1..text.len() - lsb as usize].into())
            } else {
                value.clone()
            };
            if projection.is_none()
                || !history
                    .last()
                    .is_some_and(|(_, previous)| *previous == projected)
            {
                history.push((*tick, projected));
            }
        }
        histories.push(history);
    }
    (wave, selected, histories)
}

fn assert_sample(actual: SampleRef<'_>, signal: Signal, history: &History, event: bool, tick: u64) {
    assert_eq!(actual.signal(), signal);
    if event {
        let count = history
            .iter()
            .find(|(time, _)| *time == tick)
            .map_or(0, |(_, value)| {
                let Key::Event(count) = value else {
                    unreachable!()
                };
                *count
            });
        assert!(matches!(actual, SampleRef::Event { occurrences, .. } if occurrences == count));
    } else if let Some((changed, expected)) = history.iter().rev().find(|(time, _)| *time <= tick) {
        let SampleRef::Value {
            value, changed_at, ..
        } = actual
        else {
            panic!("expected known state at {tick}")
        };
        assert_eq!(key(value), *expected);
        if let Some(time) = changed_at {
            assert_eq!(time.ticks(), *changed);
        }
    } else {
        assert!(matches!(actual, SampleRef::Missing { .. }));
    }
}

// This fixture ends at 14. An absent end means EOF, not positive infinity.
fn empty_trace_range(range: TimeRange) -> bool {
    range.start().ticks() > range.end().map_or(14, Time::ticks)
}

fn expected_rows(histories: &[History], range: TimeRange) -> Vec<Row> {
    if empty_trace_range(range) {
        return vec![];
    }
    let mut rows = Vec::new();
    for (index, history) in histories.iter().enumerate() {
        if SLOTS[index].0 != 3
            && let Some((_, value)) = history
                .iter()
                .rev()
                .find(|(time, _)| *time < range.start().ticks())
        {
            rows.push((index, true, None, value.clone()));
        }
        for (time, value) in history {
            if *time >= range.start().ticks() && range.end().is_none_or(|end| *time <= end.ticks())
            {
                rows.push((index, false, Some(*time), value.clone()));
            }
        }
    }
    rows.sort();
    rows
}

fn assert_trace(trace: &Trace, signal: Signal, history: &History, event: bool, range: TimeRange) {
    assert_eq!(trace.signal(), signal);
    assert_eq!(trace.range(), range);
    let initial = if empty_trace_range(range) || event {
        None
    } else {
        history
            .iter()
            .rev()
            .find(|(time, _)| *time < range.start().ticks())
    };
    assert_eq!(trace.initial().is_some(), initial.is_some());
    if let (Some(actual), Some((time, value))) = (trace.initial(), initial) {
        assert_eq!(key(actual.value()), *value);
        if let Some(actual) = actual.changed_at() {
            assert_eq!(actual.ticks(), *time);
        }
    }
    let expected = history
        .iter()
        .filter(|(time, _)| {
            !empty_trace_range(range)
                && *time >= range.start().ticks()
                && range.end().is_none_or(|end| *time <= end.ticks())
        })
        .collect::<Vec<_>>();
    assert_eq!(trace.changes().len(), expected.len());
    for (actual, (time, value)) in trace.changes().iter().zip(expected) {
        assert_eq!(actual.time().ticks(), *time);
        assert_eq!(key(actual.value()), *value);
    }
}

fn scan_row(
    index: usize,
    record: ScanRef<'_>,
    signals: &[Signal],
    histories: &[History],
    range: TimeRange,
) -> Row {
    match record {
        ScanRef::Initial {
            signal,
            value,
            changed_at,
        } => {
            assert_eq!(signal, signals[index]);
            let (expected_time, _) = histories[index]
                .iter()
                .rev()
                .find(|(time, _)| *time < range.start().ticks())
                .unwrap();
            if let Some(time) = changed_at {
                assert_eq!(time.ticks(), *expected_time);
            }
            (index, true, None, key(value))
        }
        ScanRef::Change {
            signal,
            time,
            value,
        } => {
            assert_eq!(signal, signals[index]);
            (index, false, Some(time.ticks()), key(value))
        }
    }
}

#[test]
fn facade_operations_match_explicit_histories_under_permutations() {
    for reverse in [false, true] {
        for extra in [false, true] {
            let (mut wave, signals, histories) = fixture(reverse, extra);
            assert_eq!(
                wave.metadata().time_span().unwrap().first(),
                Time::from_ticks(7)
            );
            for tick in [0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
                let batch = wave.samples(&signals, Time::from_ticks(tick)).unwrap();
                for (index, &signal) in signals.iter().enumerate() {
                    assert_sample(
                        wave.sample(signal, Time::from_ticks(tick))
                            .unwrap()
                            .as_ref(),
                        signal,
                        &histories[index],
                        SLOTS[index].0 == 3,
                        tick,
                    );
                    assert_sample(
                        batch[index].as_ref(),
                        signal,
                        &histories[index],
                        SLOTS[index].0 == 3,
                        tick,
                    );
                }
                let mut selection = wave.select(&signals).unwrap();
                for (index, actual) in selection
                    .samples(Time::from_ticks(tick))
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    assert_sample(
                        actual.as_ref(),
                        signals[index],
                        &histories[index],
                        SLOTS[index].0 == 3,
                        tick,
                    );
                }
                let mut index = 0;
                let _ = selection
                    .visit_samples(Time::from_ticks(tick), |actual| {
                        assert_sample(
                            actual,
                            signals[index],
                            &histories[index],
                            SLOTS[index].0 == 3,
                            tick,
                        );
                        index += 1;
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                assert_eq!(index, signals.len());
            }
            for range in [
                TimeRange::all(),
                TimeRange::point(Time::ZERO),
                TimeRange::point(Time::from_ticks(7)),
                TimeRange::closed(Time::from_ticks(7), Time::from_ticks(10)),
                TimeRange::point(Time::from_ticks(8)),
                TimeRange::closed(Time::from_ticks(12), Time::from_ticks(13)),
                TimeRange::from(Time::from_ticks(15)),
                TimeRange::closed(Time::from_ticks(15), Time::from_ticks(16)),
                TimeRange::closed(Time::from_ticks(12), Time::from_ticks(11)),
            ] {
                let batch = wave.traces(&signals, range).unwrap();
                for (index, &signal) in signals.iter().enumerate() {
                    assert_trace(
                        &wave.trace(signal, range).unwrap(),
                        signal,
                        &histories[index],
                        SLOTS[index].0 == 3,
                        range,
                    );
                    assert_trace(
                        &batch[index],
                        signal,
                        &histories[index],
                        SLOTS[index].0 == 3,
                        range,
                    );
                }
                let expected = expected_rows(&histories, range);
                let mut rows = Vec::new();
                let _ = wave
                    .scan(&signals, range, |record| {
                        let signal = match record {
                            ScanRef::Initial { signal, .. } | ScanRef::Change { signal, .. } => {
                                signal
                            }
                        };
                        let index = signals.iter().position(|s| *s == signal).unwrap();
                        rows.push(scan_row(index, record, &signals, &histories, range));
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                rows.sort();
                let mut plain_expected = expected.clone();
                for row in &mut plain_expected {
                    row.0 = signals.iter().position(|s| *s == signals[row.0]).unwrap();
                }
                plain_expected.sort();
                assert_eq!(rows, plain_expected);
                let mut selection = wave.select(&signals).unwrap();
                for (index, trace) in selection.traces(range).unwrap().iter().enumerate() {
                    assert_trace(
                        trace,
                        signals[index],
                        &histories[index],
                        SLOTS[index].0 == 3,
                        range,
                    );
                }
                let mut indexed = Vec::new();
                let mut initials = Vec::new();
                let mut last_change = None;
                let _ = selection
                    .scan_each(range, |index, record| {
                        let row = scan_row(index, record, &signals, &histories, range);
                        if row.1 {
                            assert!(last_change.is_none());
                            initials.push(index);
                        } else {
                            assert!(last_change.is_none_or(|previous| row.2.unwrap() >= previous));
                            last_change = row.2;
                        }
                        indexed.push(row);
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                assert!(initials.windows(2).all(|pair| pair[0] < pair[1]));
                indexed.sort();
                assert_eq!(indexed, expected);
                let indices = (0..signals.len()).collect::<Vec<_>>();
                let mut seen = BTreeSet::new();
                let mut previous = None;
                let _ = selection
                    .query(range, &indices, |ctx| {
                        let tick = ctx.time().ticks();
                        assert!(previous.is_none_or(|before| before < tick));
                        previous = Some(tick);
                        assert!(
                            tick >= range.start().ticks()
                                && range.end().is_none_or(|end| tick <= end.ticks())
                        );
                        seen.insert(tick);
                        for at in std::iter::once(tick).chain(tick.checked_sub(1)) {
                            let _ = ctx.visit_samples(
                                Time::from_ticks(at),
                                &indices,
                                |index, actual| {
                                    assert_sample(
                                        actual,
                                        signals[index],
                                        &histories[index],
                                        SLOTS[index].0 == 3,
                                        at,
                                    );
                                    Ok(ControlFlow::<()>::Continue(()))
                                },
                            )?;
                        }
                        Ok(ControlFlow::<()>::Continue(()))
                    })
                    .unwrap();
                let required = expected
                    .iter()
                    .filter_map(|row| row.2)
                    .collect::<BTreeSet<_>>();
                assert!(required.is_subset(&seen));
            }
            let mut selection = wave.select(&signals).unwrap();
            let mut confirmed = Vec::new();
            let mut candidates = Vec::new();
            let _ = selection
                .query(TimeRange::all(), &[0], |ctx| {
                    candidates.push(ctx.time().ticks());
                    let mut values = Vec::new();
                    for tick in [
                        ctx.time().ticks().checked_sub(1).unwrap(),
                        ctx.time().ticks(),
                    ] {
                        let _ = ctx.visit_samples(Time::from_ticks(tick), &[0], |_, sample| {
                            values.push(match sample {
                                SampleRef::Missing { .. } => None,
                                SampleRef::Value { value, .. } => Some(key(value)),
                                _ => panic!("expected persistent state"),
                            });
                            Ok(ControlFlow::<()>::Continue(()))
                        })?;
                    }
                    if values[0] != values[1] {
                        confirmed.push(ctx.time().ticks());
                    }
                    Ok(ControlFlow::<()>::Continue(()))
                })
                .unwrap();
            assert_eq!(confirmed, [7, 9, 14]);
            assert_eq!(candidates.contains(&12), extra);
        }
    }
}

#[test]
fn zero_and_maximum_tick_boundaries_do_not_wrap_or_make_events_sticky() {
    let mut wave = Waveform::memory(
        vec![Encoding::Bits { width: 1 }, Encoding::Event],
        vec![
            (0, 0, bits("x")),
            (1, 0, Value::Event { occurrences: 2 }),
            (0, u64::MAX - 1, bits("0")),
            (1, u64::MAX - 1, Value::Event { occurrences: 1 }),
            (0, u64::MAX, bits("1")),
            (1, u64::MAX, Value::Event { occurrences: 3 }),
        ],
        None,
    );
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    assert!(matches!(
        wave.sample(signals[1], Time::from_ticks(1)).unwrap(),
        crate::Sample::Event { occurrences: 0, .. }
    ));
    let mut selection = wave.select(&signals).unwrap();
    let _ = selection.query(TimeRange::point(Time::ZERO),&[0], |ctx| {
        assert!(matches!(ctx.visit_samples(Time::from_ticks(u64::MAX),&[0],|_,_| Ok(ControlFlow::<()>::Continue(()))),Err(crate::Error::InvalidQueryTime {..})));
        ctx.visit_samples(Time::ZERO,&[0,1],|index,sample| {
            if index==0 { assert!(matches!(sample,SampleRef::Value {value:ValueRef::Bits(bits),changed_at:None,..} if bits.to_string()=="x")); }
            else { assert!(matches!(sample,SampleRef::Event {occurrences:2,..})); }
            Ok(ControlFlow::<()>::Continue(()))
        })
    }).unwrap();
    let mut calls = 0;
    let _ = selection.query(TimeRange::point(Time::from_ticks(u64::MAX)),&[0,1], |ctx| {
        calls+=1;
        for tick in [u64::MAX,u64::MAX.checked_sub(1).unwrap(),u64::MAX] {
            let _ = ctx.visit_samples(Time::from_ticks(tick),&[0,1],|index,sample| {
                if index==0 {
                    assert!(matches!(sample,SampleRef::Value {value:ValueRef::Bits(bits),changed_at:Some(time),..} if bits.to_string()==if tick==u64::MAX {"1"}else{"0"} && time.ticks()==tick));
                } else { assert!(matches!(sample,SampleRef::Event {occurrences,..} if occurrences==if tick==u64::MAX {3}else{1})); }
                Ok(ControlFlow::<()>::Continue(()))
            })?;
        }
        Ok(ControlFlow::<()>::Continue(()))
    }).unwrap();
    assert_eq!(calls, 1);
    let traces = selection
        .traces(TimeRange::point(Time::from_ticks(u64::MAX)))
        .unwrap();
    assert_eq!(
        key(traces[0].initial().unwrap().value()),
        Key::Bits("0".into())
    );
    assert_eq!(traces[0].changes().len(), 1);
    assert!(traces[1].initial().is_none());
    assert_eq!(key(traces[1].changes()[0].value()), Key::Event(3));
}
