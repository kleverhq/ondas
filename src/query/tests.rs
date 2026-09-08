use super::*;
use crate::{Bits, Encoding, Error};

fn bits(text: &str) -> Value {
    Value::Bits(Bits::from_ascii(text.as_bytes()).unwrap())
}

fn waveform() -> Waveform {
    Waveform::memory(
        vec![Encoding::Bits { width: 8 }, Encoding::Event],
        vec![
            (0, 10, bits("00000000")),
            (0, 20, bits("00000001")),
            (1, 20, Value::Event),
            (1, 20, Value::Event),
            (0, 30, bits("10100001")),
            (0, 30, bits("11110001")),
            (0, 40, bits("11110001")),
        ],
        None,
    )
}

fn text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Bits(bits) => bits.to_string(),
        ValueRef::Event => "event".into(),
        _ => panic!("unexpected value"),
    }
}

#[test]
fn samples_hold_final_tick_state_and_count_every_event() {
    let mut wave = waveform();
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    assert!(matches!(
        wave.sample(signals[0], Time::ZERO).unwrap(),
        Sample::Missing { .. }
    ));
    assert!(matches!(
        wave.sample(signals[1], Time::ZERO).unwrap(),
        Sample::Event { occurrences: 0, .. }
    ));
    let samples = wave
        .samples(&[signals[1], signals[0], signals[1]], Time::from_ticks(20))
        .unwrap();
    assert!(matches!(samples[0], Sample::Event { occurrences: 2, .. }));
    assert!(matches!(samples[2], Sample::Event { occurrences: 2, .. }));
    assert_eq!(
        samples.iter().map(Sample::signal).collect::<Vec<_>>(),
        [signals[1], signals[0], signals[1]]
    );
    for tick in [30, 40, u64::MAX] {
        let Sample::Value {
            value, changed_at, ..
        } = wave.sample(signals[0], Time::from_ticks(tick)).unwrap()
        else {
            panic!("missing state")
        };
        assert_eq!(text(value.as_ref()), "11110001");
        assert_eq!(changed_at, Some(Time::from_ticks(30)));
    }
}

#[test]
fn slices_have_independent_changes_and_entering_states() {
    let mut wave = waveform();
    let base = wave.hierarchy().signals().next().unwrap();
    let slice = base.slice(7, 4).unwrap();
    let trace = wave
        .trace(
            slice,
            TimeRange::closed(Time::from_ticks(20), Time::from_ticks(30)),
        )
        .unwrap();
    assert_eq!(trace.signal(), slice);
    assert_eq!(text(trace.initial().unwrap().value()), "0000");
    assert_eq!(trace.initial().unwrap().changed_at(), None);
    assert_eq!(
        trace
            .changes()
            .iter()
            .map(|c| (c.time().ticks(), text(c.value())))
            .collect::<Vec<_>>(),
        [(30, "1010".into()), (30, "1111".into())]
    );
    let Sample::Value {
        value, changed_at, ..
    } = wave.sample(slice, Time::from_ticks(25)).unwrap()
    else {
        panic!("missing state")
    };
    assert_eq!(text(value.as_ref()), "0000");
    assert_eq!(changed_at, None);
    let trace = wave
        .trace(slice, TimeRange::point(Time::from_ticks(31)))
        .unwrap();
    assert_eq!(
        trace.initial().unwrap().changed_at(),
        Some(Time::from_ticks(30))
    );
    assert!(trace.changes().is_empty());
}

#[test]
fn scan_emits_initials_first_and_keeps_duplicate_entries() {
    let mut wave = waveform();
    let sigs = wave.hierarchy().signals().collect::<Vec<_>>();
    let selected = [sigs[0], sigs[1], sigs[0]];
    let mut selection = wave.select(&selected).unwrap();
    assert_eq!(selection.signals(), selected);
    let mut records = Vec::new();
    let result = selection
        .scan(
            TimeRange::closed(Time::from_ticks(20), Time::from_ticks(30)),
            |item| {
                records.push(match item {
                    ScanRef::Initial { signal, value, .. } => (signal, None, text(value)),
                    ScanRef::Change {
                        signal,
                        time,
                        value,
                    } => (signal, Some(time.ticks()), text(value)),
                });
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
    assert_eq!(
        records
            .iter()
            .take(2)
            .map(|r| (r.0, r.1))
            .collect::<Vec<_>>(),
        [(sigs[0], None), (sigs[0], None)]
    );
    assert!(records.iter().skip(2).all(|r| r.1.is_some()));
    assert_eq!(records.iter().filter(|r| r.2 == "event").count(), 2);
    assert_eq!(
        records
            .iter()
            .filter(|r| r.1 == Some(30))
            .map(|r| r.2.as_str())
            .collect::<Vec<_>>(),
        ["10100001", "10100001", "11110001", "11110001"]
    );
    let traces = selection.traces(TimeRange::all()).unwrap();
    assert_eq!(
        traces.iter().map(Trace::signal).collect::<Vec<_>>(),
        selected
    );
    assert_eq!(traces[0].changes().len(), 4);
    assert_eq!(traces[2].changes().len(), 4);
    assert_eq!(traces[1].changes().len(), 2);
    let mut times = Vec::new();
    let _ = selection
        .scan_candidate_times(
            TimeRange::closed(Time::from_ticks(20), Time::from_ticks(30)),
            |t| {
                times.push(t.ticks());
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(times, [20, 30]);
}

#[test]
fn multiple_initials_follow_selection_order_not_history_order() {
    let mut wave = Waveform::memory(
        vec![
            Encoding::Bits { width: 1 },
            Encoding::Bits { width: 1 },
            Encoding::Event,
            Encoding::Bits { width: 1 },
        ],
        vec![
            (0, 1, bits("0")),
            (1, 2, bits("1")),
            (0, 3, bits("1")),
            (2, 5, Value::Event),
            (1, 5, bits("0")),
        ],
        None,
    );
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    let selected = [signals[1], signals[2], signals[0], signals[1], signals[3]];
    let mut selection = wave.select(&selected).unwrap();
    assert_eq!(selection.hierarchy().signals().collect::<Vec<_>>(), signals);
    let mut initials = Vec::new();
    let mut changes = Vec::new();
    let result = selection
        .scan(TimeRange::point(Time::from_ticks(5)), |record| {
            match record {
                ScanRef::Initial {
                    signal,
                    value,
                    changed_at,
                } => {
                    assert!(changes.is_empty(), "initial emitted after a change");
                    assert!(changed_at.is_none_or(|t| t < Time::from_ticks(5)));
                    initials.push((signal, text(value), changed_at));
                }
                ScanRef::Change {
                    signal,
                    time,
                    value,
                } => {
                    changes.push((signal, time.ticks(), text(value)));
                }
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
    assert_eq!(
        initials
            .iter()
            .map(|(s, v, _)| (*s, v.as_str()))
            .collect::<Vec<_>>(),
        [(signals[1], "1"), (signals[0], "1"), (signals[1], "1")]
    );
    assert_eq!(initials[1].2, Some(Time::from_ticks(3)));
    // Cross-signal ordering at one tick is deliberately not asserted.
    assert_eq!(changes.len(), 3);
    assert_eq!(
        changes
            .iter()
            .filter(|r| **r == (signals[1], 5, "0".into()))
            .count(),
        2
    );
    assert_eq!(
        changes
            .iter()
            .filter(|r| **r == (signals[2], 5, "event".into()))
            .count(),
        1
    );
}

#[test]
fn composed_projections_cohere_at_inclusive_boundaries() {
    let mut wave = Waveform::memory(
        vec![Encoding::Bits { width: 9 }],
        vec![
            (0, 2, bits("000000000")),
            (0, 5, bits("000101000")),
            (0, 5, bits("000010000")),
            (0, 6, bits("100010001")), // Only unobserved bits change.
            (0, 7, bits("100010001")),
            (0, u64::MAX, bits("100111001")),
        ],
        None,
    );
    let base = wave.hierarchy().signals().next().unwrap();
    let composed = base.slice(7, 2).unwrap().slice(3, 1).unwrap();
    let direct = base.slice(5, 3).unwrap();
    assert_eq!(composed, direct);
    let mut selection = wave.select(&[composed, direct]).unwrap();
    for (range, initial, expected) in [
        (
            TimeRange::closed(Time::ZERO, Time::from_ticks(1)),
            None,
            vec![],
        ),
        (
            TimeRange::point(Time::from_ticks(2)),
            None,
            vec![(2, "000")],
        ),
        (
            TimeRange::point(Time::from_ticks(5)),
            Some("000"),
            vec![(5, "101"), (5, "010")],
        ),
        (TimeRange::point(Time::from_ticks(6)), Some("010"), vec![]),
        (
            TimeRange::from(Time::from_ticks(6)),
            Some("010"),
            vec![(u64::MAX, "111")],
        ),
        (
            TimeRange::point(Time::from_ticks(u64::MAX)),
            Some("010"),
            vec![(u64::MAX, "111")],
        ),
    ] {
        for trace in selection.traces(range).unwrap() {
            assert_eq!(trace.signal(), direct);
            assert_eq!(trace.range(), range);
            assert_eq!(
                trace.initial().map(|v| text(v.value())),
                initial.map(str::to_owned)
            );
            if let Some(state) = trace.initial() {
                assert!(state.changed_at().is_none_or(|t| t < range.start()));
                if initial == Some("010") {
                    assert_eq!(state.changed_at(), Some(Time::from_ticks(5)));
                }
            }
            assert_eq!(
                trace
                    .changes()
                    .iter()
                    .map(|c| (c.time().ticks(), text(c.value())))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|(t, v)| (*t, (*v).to_owned()))
                    .collect::<Vec<_>>()
            );
        }
        let mut candidates = Vec::new();
        assert_eq!(
            selection
                .scan_candidate_times(range, |time| {
                    candidates.push(time);
                    ControlFlow::<()>::Continue(())
                })
                .unwrap(),
            ControlFlow::Continue(())
        );
        assert!(candidates.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            candidates
                .iter()
                .all(|&t| t >= range.start() && range.end().is_none_or(|end| t <= end))
        );
        assert!(
            expected
                .iter()
                .all(|(t, _)| candidates.contains(&Time::from_ticks(*t)))
        );
    }
    for (tick, expected, changed) in [
        (5, "010", 5),
        (6, "010", 5),
        (7, "010", 5),
        (u64::MAX, "111", u64::MAX),
    ] {
        for sample in selection.samples(Time::from_ticks(tick)).unwrap() {
            let SampleRef::Value {
                signal,
                value,
                changed_at,
            } = sample.as_ref()
            else {
                panic!("expected projected persistent state")
            };
            assert_eq!(signal, direct);
            assert_eq!(text(value), expected);
            assert_eq!(changed_at, Some(Time::from_ticks(changed)));
        }
    }
}

#[test]
fn empty_histories_and_eof_bounds_are_successful_observations() {
    let mut empty = Waveform::memory(
        vec![Encoding::Real, Encoding::String, Encoding::Event],
        vec![],
        None,
    );
    assert_eq!(empty.metadata().time_span(), None);
    let signals = empty.hierarchy().signals().collect::<Vec<_>>();
    for tick in [Time::ZERO, Time::from_ticks(u64::MAX)] {
        let samples = empty.samples(&signals, tick).unwrap();
        for sample in &samples[..2] {
            assert!(matches!(sample.as_ref(), SampleRef::Missing { .. }));
        }
        assert!(matches!(
            samples[2].as_ref(),
            SampleRef::Event { occurrences: 0, .. }
        ));
        for range in [TimeRange::from(tick), TimeRange::point(tick)] {
            for trace in empty.traces(&signals, range).unwrap() {
                assert_eq!(trace.range(), range);
                assert!(trace.initial().is_none());
                assert!(trace.changes().is_empty());
            }
            let result = empty
                .scan_candidate_times(&signals, range, |_| panic!("empty history candidate"))
                .unwrap();
            assert_eq!(result, ControlFlow::<()>::Continue(()));
        }
    }
    let mut wave = waveform();
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    assert_eq!(
        wave.metadata().time_span().unwrap().last(),
        Time::from_ticks(40)
    );
    for range in [
        TimeRange::from(Time::from_ticks(40)),
        TimeRange::closed(Time::from_ticks(41), Time::from_ticks(u64::MAX)),
    ] {
        let traces = wave.traces(&signals, range).unwrap();
        assert_eq!(traces[0].range(), range);
        let initial = traces[0].initial().unwrap();
        assert_eq!(text(initial.value()), "11110001");
        assert_eq!(initial.changed_at(), Some(Time::from_ticks(30)));
        assert!(traces.iter().all(|t| t.changes().is_empty()));
        assert!(traces[1].initial().is_none());
    }
    for trace in wave
        .traces(&signals, TimeRange::from(Time::from_ticks(41)))
        .unwrap()
    {
        assert!(
            trace.initial().is_none(),
            "unbounded range starts after EOF"
        );
        assert!(trace.changes().is_empty());
    }
    assert!(matches!(
        wave.sample(signals[1], Time::from_ticks(u64::MAX)).unwrap(),
        Sample::Event { occurrences: 0, .. }
    ));
}

#[test]
fn borrowed_mixed_values_remain_owned_after_queries_and_source_drop() {
    let mut wave = Waveform::memory(
        vec![
            Encoding::Real,
            Encoding::String,
            Encoding::Bits { width: 9 },
        ],
        vec![
            (0, 2, Value::Real(1.25)),
            (1, 2, Value::String("".into())),
            (2, 2, bits("01XZHUWL-")),
            (0, 3, Value::Real(-2.5)),
            (1, 3, Value::String("new\0λ".into())),
            (2, 3, bits("---------")),
        ],
        None,
    );
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    let mut retained = Vec::new();
    {
        let mut selection = wave.select(&signals).unwrap();
        assert_eq!(
            selection
                .visit_samples(Time::from_ticks(2), |sample| {
                    let SampleRef::Value {
                        signal,
                        value,
                        changed_at,
                    } = sample
                    else {
                        panic!("expected persistent value")
                    };
                    assert!(changed_at.is_none_or(|t| t <= Time::from_ticks(2)));
                    retained.push((signal, value.to_owned()));
                    ControlFlow::<()>::Continue(())
                })
                .unwrap(),
            ControlFlow::Continue(())
        );
    }
    let mut scanned = Vec::new();
    let _ = wave
        .scan(&signals, TimeRange::point(Time::from_ticks(3)), |record| {
            match record {
                ScanRef::Initial { signal, value, .. } => {
                    scanned.push((signal, None, value.to_owned()))
                }
                ScanRef::Change {
                    signal,
                    time,
                    value,
                } => scanned.push((signal, Some(time), value.to_owned())),
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    let traces = wave
        .traces(&signals, TimeRange::point(Time::from_ticks(3)))
        .unwrap();
    let samples = wave.samples(&signals, Time::from_ticks(3)).unwrap();
    drop(wave);
    let check = |index: usize, value: ValueRef<'_>, later: bool| match (index, value) {
        (0, ValueRef::Real(real)) => assert_eq!(real, if later { -2.5 } else { 1.25 }),
        (1, ValueRef::String(string)) => assert_eq!(string, if later { "new\0λ" } else { "" }),
        (2, ValueRef::Bits(bits)) => {
            assert_eq!(bits.width(), 9);
            assert_eq!(
                bits.to_string(),
                if later { "---------" } else { "01xzhuwl-" }
            );
        }
        _ => panic!("wrong value class"),
    };
    assert_eq!(retained.len(), 3);
    assert_eq!(scanned.len(), 6);
    for (index, (signal, value)) in retained.iter().enumerate() {
        assert_eq!(*signal, signals[index]);
        check(index, value.as_ref(), false);
        let trace = &traces[index];
        check(index, trace.initial().unwrap().value(), false);
        assert_eq!(trace.changes().len(), 1);
        assert_eq!(trace.changes()[0].time(), Time::from_ticks(3));
        check(index, trace.changes()[0].value(), true);
        let SampleRef::Value {
            value, changed_at, ..
        } = samples[index].as_ref()
        else {
            panic!("expected owned sample")
        };
        assert_eq!(changed_at, Some(Time::from_ticks(3)));
        check(index, value, true);
    }
    for (signal, time, value) in scanned {
        let index = signals.iter().position(|s| *s == signal).unwrap();
        assert!(time.is_none_or(|t| t == Time::from_ticks(3)));
        check(index, value.as_ref(), time.is_some());
    }
}

#[test]
fn empty_ranges_and_selections_do_not_visit_or_read() {
    let mut wave = Waveform::memory(
        vec![Encoding::Bits { width: 8 }],
        vec![(0, 0, bits("00000000"))],
        Some(0),
    );
    let sig = wave.hierarchy().signals().next().unwrap();
    let empty = TimeRange::closed(Time::from_ticks(2), Time::from_ticks(1));
    let result = wave
        .scan(&[sig], empty, |_| panic!("empty range visited"))
        .unwrap();
    assert_eq!(result, ControlFlow::<()>::Continue(()));
    let trace = wave.trace(sig, empty).unwrap();
    assert!(trace.initial().is_none() && trace.changes().is_empty());
    assert!(wave.samples(&[], Time::ZERO).unwrap().is_empty());
    assert!(wave.traces(&[], TimeRange::all()).unwrap().is_empty());
    for (signals, range) in [(&[sig][..], empty), (&[][..], TimeRange::all())] {
        assert_eq!(
            wave.scan_candidate_times(signals, range, |_| panic!("empty candidates visited"))
                .unwrap(),
            ControlFlow::<()>::Continue(())
        );
    }
}

#[test]
fn breaks_stop_the_reader_and_late_errors_preserve_prior_callbacks() {
    let mut wave = Waveform::memory(
        vec![Encoding::Bits { width: 1 }],
        vec![(0, 0, bits("0")), (0, 1, bits("1"))],
        Some(1),
    );
    let sig = wave.hierarchy().signals().next().unwrap();
    let mut calls = 0;
    assert_eq!(
        wave.scan(&[sig], TimeRange::all(), |_| {
            calls += 1;
            ControlFlow::Break("stop")
        })
        .unwrap(),
        ControlFlow::Break("stop")
    );
    assert_eq!(calls, 1);
    calls = 0;
    let result = wave.scan(&[sig], TimeRange::all(), |_| {
        calls += 1;
        ControlFlow::<()>::Continue(())
    });
    assert!(matches!(result, Err(Error::Backend { .. })));
    assert_eq!(calls, 1);
    assert!(wave.trace(sig, TimeRange::all()).is_err());
    assert!(matches!(
        wave.samples(&[sig, sig], Time::from_ticks(1)),
        Err(Error::Backend { .. })
    ));
    let mut sample_calls = 0;
    let result = wave
        .select(&[sig])
        .unwrap()
        .visit_samples(Time::from_ticks(1), |_| {
            sample_calls += 1;
            ControlFlow::<()>::Continue(())
        });
    assert!(matches!(result, Err(Error::Backend { .. })));
    assert_eq!(sample_calls, 0);
    let mut times = Vec::new();
    assert_eq!(
        wave.scan_candidate_times(&[sig], TimeRange::all(), |t| {
            times.push(t);
            ControlFlow::Break(7)
        })
        .unwrap(),
        ControlFlow::Break(7)
    );
    assert_eq!(times, [Time::ZERO]);
}

#[test]
fn invalid_and_unsupported_signals_fail_before_queries() {
    let mut wave = waveform();
    let other = waveform().hierarchy().signals().next().unwrap();
    assert!(matches!(
        wave.sample(other, Time::ZERO),
        Err(Error::InvalidSignal { .. })
    ));
    let mut unsupported = Waveform::memory(vec![Encoding::Unsupported], vec![], None);
    let sig = unsupported.hierarchy().signals().next().unwrap();
    assert!(matches!(
        unsupported.sample(sig, Time::ZERO),
        Err(Error::UnsupportedSignal { .. })
    ));
}

#[test]
fn genuine_tick_zero_events_and_borrowed_sample_breaks_survive() {
    let mut wave = Waveform::memory(
        vec![Encoding::Event],
        vec![(0, 0, Value::Event), (0, 0, Value::Event)],
        None,
    );
    let sig = wave.hierarchy().signals().next().unwrap();
    assert!(matches!(
        wave.sample(sig, Time::ZERO).unwrap(),
        Sample::Event { occurrences: 2, .. }
    ));
    assert_eq!(
        wave.trace(sig, TimeRange::point(Time::ZERO))
            .unwrap()
            .changes()
            .len(),
        2
    );
    let mut calls = 0;
    let result = wave
        .select(&[sig, sig])
        .unwrap()
        .visit_samples(Time::ZERO, |sample| {
            calls += 1;
            assert_eq!(sample.signal(), sig);
            ControlFlow::Break("first")
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Break("first"));
    assert_eq!(calls, 1);
}
