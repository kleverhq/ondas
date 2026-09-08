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
