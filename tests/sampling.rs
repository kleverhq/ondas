use ondas::{Error, Sample, SampleRef, Signal, Time, TimeRange, ValueRef, Waveform};
use std::ops::ControlFlow;

fn t(tick: u64) -> Time {
    Time::from_ticks(tick)
}
fn key(sample: SampleRef<'_>) -> String {
    match sample {
        SampleRef::Missing { .. } => "Missing".into(),
        SampleRef::Value {
            value: ValueRef::Bits(bits),
            changed_at,
            ..
        } => format!("{bits}@{:?}", changed_at.map(Time::ticks)),
        SampleRef::Event { occurrences, .. } => format!("Event({occurrences})"),
        _ => panic!("unexpected class"),
    }
}
fn source() -> (Waveform, [Signal; 5]) {
    let bytes = b"$var wire 4 a a $end $var wire 4 a alias $end $var wire 1 m late $end \
        $var event 1 e pulse $end $enddefinitions $end #0 #2 b0010 a \
        #6 b0110 a 1e 1e #9 xm #10 b1010 a 1e #16";
    let wave =
        ondas::open_bytes_with("schedule.vcd", bytes.as_slice().into(), "vcd-native").unwrap();
    let a = wave.hierarchy().signal("a").unwrap();
    let alias = wave.hierarchy().signal("alias").unwrap();
    assert_eq!(a, alias);
    let e = wave.hierarchy().signal("pulse").unwrap();
    let m = wave.hierarchy().signal("late").unwrap();
    (wave, [a, e, alias, a.slice(1, 0).unwrap(), m])
}

#[test]
fn requested_times_and_slots_keep_order_duplicates_and_quiet_ticks() {
    let (mut wave, selected) = source();
    let times = [12, 3, 12, 8].map(t);
    let rows = wave.samples_at(&selected, &times).unwrap();
    assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), times);
    let expected = [
        [
            "1010@Some(10)",
            "Event(0)",
            "1010@Some(10)",
            "10@None",
            "x@None",
        ],
        ["0010@None", "Event(0)", "0010@None", "10@None", "Missing"],
        [
            "1010@Some(10)",
            "Event(0)",
            "1010@Some(10)",
            "10@None",
            "x@None",
        ],
        [
            "0110@Some(6)",
            "Event(0)",
            "0110@Some(6)",
            "10@None",
            "Missing",
        ],
    ];
    for ((time, row), expected) in rows.iter().zip(expected) {
        assert_eq!(row.iter().map(|s| s.signal()).collect::<Vec<_>>(), selected);
        assert_eq!(
            row.iter().map(|s| key(s.as_ref())).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            row.iter().map(|s| key(s.as_ref())).collect::<Vec<_>>(),
            wave.samples(&selected, *time)
                .unwrap()
                .iter()
                .map(|s| key(s.as_ref()))
                .collect::<Vec<_>>()
        );
    }
    drop(wave);
    assert_eq!(key(rows[0].1[0].as_ref()), "1010@Some(10)");
}

#[test]
fn saved_samples_preserve_coverage_initials_missing_and_exact_events() {
    let (mut wave, selected) = source();
    let range = TimeRange::closed(t(4), t(12));
    let traces = wave.traces(&selected, range).unwrap();
    let known_initial = wave
        .trace(selected[0], TimeRange::closed(t(8), t(12)))
        .unwrap();
    assert_eq!(known_initial.initial().unwrap().changed_at(), Some(t(6)));
    assert_eq!(key(known_initial.sample(t(8)).unwrap()), "0110@Some(6)");
    assert_eq!(key(traces[0].sample(t(4)).unwrap()), "0010@None");
    assert_eq!(key(traces[4].sample(t(8)).unwrap()), "Missing");
    assert_eq!(key(traces[4].sample(t(9)).unwrap()), "x@None");
    for (tick, count) in [(4, 0), (6, 2), (8, 0), (10, 1), (12, 0)] {
        assert_eq!(
            key(traces[1].sample(t(tick)).unwrap()),
            format!("Event({count})")
        );
    }
    for trace in &traces {
        for tick in [0, 3, 13, u64::MAX] {
            assert!(
                matches!(trace.sample(t(tick)), Err(Error::TraceCoverage {requested,range:r}) if requested==t(tick)&&r==range)
            );
        }
        assert!(
            matches!(trace.samples_at(&[t(8),t(13)]), Err(Error::TraceCoverage {requested,..}) if requested==t(13))
        );
        assert!(trace.samples_at(&[]).unwrap().is_empty());
    }
    let rows = {
        let times = [t(12), t(6), t(12), t(8)];
        traces[0].samples_at(&times).unwrap()
    };
    drop(wave);
    assert_eq!(
        rows.iter().map(|r| r.0.ticks()).collect::<Vec<_>>(),
        [12, 6, 12, 8]
    );
    assert_eq!(
        rows.iter().map(|r| key(r.1)).collect::<Vec<_>>(),
        [
            "1010@Some(10)",
            "0110@Some(6)",
            "1010@Some(10)",
            "0110@Some(6)"
        ]
    );
    let owned: Vec<_> = rows.into_iter().map(|(_, s)| s.to_owned()).collect();
    drop(traces);
    assert_eq!(key(owned[1].as_ref()), "0110@Some(6)");
}

#[test]
fn eof_zero_max_and_empty_dimensions_share_point_semantics() {
    let (mut wave, selected) = source();
    let times = [u64::MAX, 0, 6, 9, 10, 17, u64::MAX].map(t);
    let rows = wave.samples_at(&selected, &times).unwrap();
    for (time, row) in &rows {
        let points = wave.samples(&selected, *time).unwrap();
        assert_eq!(
            row.iter().map(|s| key(s.as_ref())).collect::<Vec<_>>(),
            points.iter().map(|s| key(s.as_ref())).collect::<Vec<_>>()
        );
    }
    assert_eq!(key(rows[0].1[0].as_ref()), "1010@Some(10)");
    assert_eq!(key(rows[1].1[0].as_ref()), "Missing");
    assert_eq!(key(rows[2].1[1].as_ref()), "Event(2)");
    for range in [
        TimeRange::all(),
        TimeRange::from(t(17)),
        TimeRange::from(t(u64::MAX)),
        TimeRange::closed(t(17), t(20)),
    ] {
        for trace in wave.traces(&selected, range).unwrap() {
            let tick = if range.end().is_some() {
                t(20)
            } else {
                t(u64::MAX)
            };
            assert_eq!(
                key(trace.sample(tick).unwrap()),
                key(wave.sample(trace.signal(), tick).unwrap().as_ref())
            );
            if range.start() > t(16) && trace.signal() != selected[1] {
                assert!(trace.initial().is_some());
                assert!(trace.changes().is_empty());
            }
        }
    }
    for trace in wave
        .traces(&selected, TimeRange::closed(t(12), t(4)))
        .unwrap()
    {
        for tick in [0, 4, 8, 12, u64::MAX] {
            assert!(matches!(
                trace.sample(t(tick)),
                Err(Error::TraceCoverage { .. })
            ));
        }
    }
    assert!(wave.samples_at(&selected, &[]).unwrap().is_empty());
    let empty = wave.samples_at(&[], &times).unwrap();
    assert_eq!(empty.iter().map(|r| r.0).collect::<Vec<_>>(), times);
    assert!(empty.iter().all(|r| r.1.is_empty()));
    let (_, foreign) = source();
    assert!(matches!(
        wave.samples_at(&foreign, &[]),
        Err(Error::InvalidSignal { .. })
    ));
}

#[test]
fn nested_compositions_cohere_across_point_batch_query_and_trace() {
    let input = b"$var wire 4 h hi $end $var wire 4 l lo $end $enddefinitions $end \
        #2 b0110 l #3 bxxxx h #5 b1001 h b0011 l #6 b1010 h b1001 h #7 b1010 h #8 b1011 l";
    let mut wave = ondas::open_bytes("computed.vcd", input.as_slice().into()).unwrap();
    let hi = wave.hierarchy().signal("hi").unwrap();
    let lo = wave.hierarchy().signal("lo").unwrap();
    let bus = wave.concat(&[hi, lo]).unwrap();
    let upper = bus.slice(7, 4).unwrap();
    let nested = wave.concat(&[upper, hi]).unwrap();
    let selected = [bus, upper, nested, bus];
    let times = [2, 0, 2, 4, 6, 7, 8, 9].map(t);
    let rows = wave.samples_at(&selected, &times).unwrap();
    assert_eq!(key(rows[0].1[0].as_ref()), "xxxx0110@None");
    assert_eq!(key(rows[0].1[1].as_ref()), "xxxx@None");
    assert_eq!(key(rows[0].1[2].as_ref()), "xxxxxxxx@None");
    assert_eq!(key(rows[4].1[0].as_ref()), "10010011@Some(5)");
    let traces = wave.traces(&selected, TimeRange::all()).unwrap();
    for (time, row) in &rows {
        for (slot, sample) in row.iter().enumerate() {
            assert_eq!(
                key(sample.as_ref()),
                key(wave.sample(selected[slot], *time).unwrap().as_ref())
            );
            assert_eq!(
                key(sample.as_ref()),
                key(traces[slot].sample(*time).unwrap())
            );
        }
    }
    let mut candidates = vec![];
    let _ = wave
        .select(&selected)
        .unwrap()
        .query(TimeRange::all(), &[0], |ctx| {
            candidates.push(ctx.time());
            ctx.visit_samples(ctx.time(), &[2, 0, 2], |slot, sample| {
                assert_eq!(key(sample), key(traces[slot].sample(ctx.time()).unwrap()));
                Ok(ControlFlow::<()>::Continue(()))
            })
        })
        .unwrap();
    assert!(candidates.contains(&t(2)) && candidates.contains(&t(7)));
    assert!(matches!(rows[1].1[0], Sample::Missing { .. }));
}
