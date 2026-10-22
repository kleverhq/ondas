use ondas::{BitRange, Error, SampleRef, Signal, SliceError, Time, TimeRange, ValueRef, Waveform};
use std::ops::ControlFlow;

fn text(sample: SampleRef<'_>) -> Option<String> {
    match sample {
        SampleRef::Missing { .. } => None,
        SampleRef::Value {
            value: ValueRef::Bits(bits),
            ..
        } => Some(bits.to_string()),
        _ => panic!("expected a bit observation"),
    }
}

fn source(reverse: bool) -> Waveform {
    let mut input = String::from(
        "$scope module top $end \
        $var wire 4 h hi [7:4] $end $var wire 4 h alias [0:3] $end \
        $var wire 4 l lo [-2:1] $end $upscope $end $enddefinitions $end \
        #2 b0110 l #3 bxxxx h #4 b0110 l #5 ",
    );
    let parts = if reverse {
        "b0011 l b1100 l b0011 l b1001 h"
    } else {
        "b1001 h b0011 l b1100 l b0011 l"
    };
    input.push_str(parts);
    input.push_str(if reverse {
        " #6 b1111 l b0011 l b1010 h b1001 h"
    } else {
        " #6 b1010 h b1001 h b1111 l b0011 l"
    });
    input.push_str(" #7 b1010 h #8 b1011 l");
    ondas::open_bytes_with("parts.vcd", input.into_bytes().into(), "vcd-native").unwrap()
}

fn expected(tick: u64) -> [Option<&'static str>; 6] {
    match tick {
        0..=1 => [None; 6],
        2..=4 => [
            Some("xxxx0110"),
            Some("xxxx"),
            Some("0110"),
            Some("xx01"),
            Some("xxxxxxxx"),
            Some("xxxx0110"),
        ],
        5..=6 => [
            Some("10010011"),
            Some("1001"),
            Some("0011"),
            Some("0100"),
            Some("10011001"),
            Some("10010011"),
        ],
        7 => [
            Some("10100011"),
            Some("1010"),
            Some("0011"),
            Some("1000"),
            Some("10101010"),
            Some("10100011"),
        ],
        _ => [
            Some("10101011"),
            Some("1010"),
            Some("1011"),
            Some("1010"),
            Some("10101010"),
            Some("10101011"),
        ],
    }
}

fn handles(wave: &mut Waveform) -> (Signal, Signal, [Signal; 6]) {
    let hi = wave.hierarchy().signal("top.hi").unwrap();
    let lo = wave.hierarchy().signal("top.lo").unwrap();
    let bus = wave.concat(&[hi, lo]).unwrap();
    let high = bus.slice(7, 4).unwrap();
    let nested = wave.concat(&[high, hi]).unwrap();
    (
        hi,
        lo,
        [
            bus,
            high,
            bus.slice(3, 0).unwrap(),
            bus.slice(5, 2).unwrap(),
            nested,
            bus,
        ],
    )
}

#[test]
fn common_operations_preserve_nested_d3_and_completed_ticks() {
    for reverse in [false, true] {
        let mut wave = source(reverse);
        const LAST_TICK: u64 = 8;
        assert_eq!(
            wave.metadata().time_span().unwrap().last().ticks(),
            LAST_TICK
        );
        let (hi, lo, selected) = handles(&mut wave);
        for tick in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, u64::MAX] {
            let time = Time::from_ticks(tick);
            let samples = wave.samples(&selected, time).unwrap();
            for (slot, sample) in samples.iter().enumerate() {
                assert_eq!(sample.signal(), selected[slot]);
                assert_eq!(
                    text(sample.as_ref()).as_deref(),
                    expected(tick)[slot],
                    "tick {tick}, slot {slot}"
                );
                assert_eq!(
                    text(wave.sample(selected[slot], time).unwrap().as_ref()),
                    text(sample.as_ref())
                );
            }
        }
        assert!(matches!(
            wave.sample(hi, Time::from_ticks(2)).unwrap().as_ref(),
            SampleRef::Missing { .. }
        ));
        assert_eq!(
            text(wave.sample(lo, Time::from_ticks(2)).unwrap().as_ref()).as_deref(),
            Some("0110")
        );
        let histories = [
            vec![
                (2, "xxxx0110"),
                (5, "10010011"),
                (7, "10100011"),
                (8, "10101011"),
            ],
            vec![(2, "xxxx"), (5, "1001"), (7, "1010")],
            vec![(2, "0110"), (5, "0011"), (8, "1011")],
            vec![(2, "xx01"), (5, "0100"), (7, "1000"), (8, "1010")],
            vec![(2, "xxxxxxxx"), (5, "10011001"), (7, "10101010")],
            vec![
                (2, "xxxx0110"),
                (5, "10010011"),
                (7, "10100011"),
                (8, "10101011"),
            ],
        ];
        for range in [
            TimeRange::all(),
            TimeRange::closed(Time::from_ticks(3), Time::from_ticks(6)),
            TimeRange::point(Time::from_ticks(6)),
            TimeRange::closed(Time::from_ticks(8), Time::from_ticks(7)),
            TimeRange::from(Time::from_ticks(20)),
        ] {
            let traces = wave.traces(&selected, range).unwrap();
            let empty = range.is_empty();
            let mut expected_rows = Vec::new();
            let mut expected_initials = Vec::new();
            for (slot, trace) in traces.iter().enumerate() {
                let initial = if empty {
                    None
                } else {
                    histories[slot]
                        .iter()
                        .rev()
                        .find(|(t, _)| *t < range.start().ticks())
                };
                assert_eq!(
                    trace.initial().map(|v| {
                        let ValueRef::Bits(b) = v.value() else {
                            panic!()
                        };
                        (v.changed_at(), b.to_string())
                    }),
                    initial.map(|(t, v)| {
                        // Tick 2 establishes the first observation; later net
                        // changes have known timestamps in this authored history.
                        ((*t != 2).then_some(Time::from_ticks(*t)), v.to_string())
                    })
                );
                expected_initials.push(
                    initial
                        .map(|(t, v)| ((*t != 2).then_some(Time::from_ticks(*t)), v.to_string()))
                        .into_iter()
                        .collect::<Vec<_>>(),
                );
                let actual: Vec<_> = trace
                    .changes()
                    .iter()
                    .map(|c| {
                        let ValueRef::Bits(b) = c.value() else {
                            panic!()
                        };
                        (c.time().ticks(), b.to_string())
                    })
                    .collect();
                let expected: Vec<_> = histories[slot]
                    .iter()
                    .filter(|(t, _)| {
                        !empty
                            && *t >= range.start().ticks()
                            && range.end().is_none_or(|end| *t <= end.ticks())
                    })
                    .map(|(t, v)| (*t, v.to_string()))
                    .collect();
                assert_eq!(actual, expected, "slot {slot} range {range:?}");
                expected_rows.push(expected);
            }
            let mut selection = wave.select(&selected).unwrap();
            let mut rows = vec![Vec::new(); selected.len()];
            let mut initials = vec![Vec::new(); selected.len()];
            let _ = selection
                .scan_each(range, |slot, record| {
                    match record {
                        ondas::ScanRef::Change {
                            signal,
                            time,
                            value: ValueRef::Bits(bits),
                            ..
                        } => {
                            assert_eq!(signal, selected[slot]);
                            rows[slot].push((time.ticks(), bits.to_string()));
                        }
                        ondas::ScanRef::Initial {
                            signal,
                            changed_at,
                            value: ValueRef::Bits(bits),
                            ..
                        } => {
                            assert_eq!(signal, selected[slot]);
                            initials[slot].push((changed_at, bits.to_string()));
                        }
                        _ => panic!("unexpected non-bit scan record"),
                    }
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert_eq!(rows, expected_rows, "scan changes in {range:?}");
            assert_eq!(initials, expected_initials, "scan initials in {range:?}");
            assert_eq!(rows[0], rows[5]);
            assert_eq!(initials[0], initials[5]);
        }
        let mut query = wave.select(&selected).unwrap();
        let mut times = Vec::new();
        let _ = query
            .query(TimeRange::from(Time::from_ticks(3)), &[1, 1], |ctx| {
                let t = ctx.time().ticks();
                times.push(t);
                for tick in [t, t - 1, t] {
                    let mut slots = Vec::new();
                    let _ = ctx.visit_samples(
                        Time::from_ticks(tick),
                        &[4, 1, 0, 4],
                        |slot, sample| {
                            slots.push(slot);
                            assert_eq!(
                                text(sample).as_deref(),
                                expected(tick)[slot],
                                "query {t} reading {tick} slot {slot}"
                            );
                            Ok(ControlFlow::<()>::Continue(()))
                        },
                    )?;
                    assert_eq!(slots, [4, 1, 0, 4]);
                }
                Ok(ControlFlow::<()>::Continue(()))
            })
            .unwrap();
        assert!(times.contains(&5) && times.contains(&7));
        assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
        for (slot, tick) in [(0, 8), (1, 7), (2, 8), (3, 8), (4, 7), (5, 8)] {
            let sample = query.samples(Time::from_ticks(9)).unwrap().remove(slot);
            assert!(
                matches!(sample.as_ref(),SampleRef::Value{changed_at:Some(t),..} if t.ticks()==tick)
            );
        }
    }
}

#[test]
fn identities_ranges_repeats_and_retained_values() {
    let mut wave = source(false);
    let hierarchy = wave.hierarchy().clone();
    let (hi, lo, handles) = handles(&mut wave);
    let bus = handles[0];
    assert_eq!(bus, wave.concat(&[hi, lo]).unwrap());
    assert_eq!(
        bus,
        wave.concat(&[hierarchy.signal("top.alias").unwrap(), lo])
            .unwrap()
    );
    assert_ne!(bus, wave.concat(&[lo, hi]).unwrap());
    assert_eq!(wave.concat(&[bus]).unwrap(), bus);
    assert_eq!(
        bus.slice(7, 4).unwrap().slice(2, 1).unwrap(),
        bus.slice(6, 5).unwrap()
    );
    assert_eq!(bus.slice(7, 0).unwrap(), bus);
    assert_eq!(bus.slice(7, 4).unwrap().base(), bus);
    assert_eq!(hierarchy.signals().count(), 2);
    assert_eq!(hierarchy.variables().count(), 3);
    assert_eq!(hierarchy.aliases(bus).unwrap().count(), 0);
    assert_eq!(
        hierarchy.aliases(bus.slice(7, 4).unwrap()).unwrap().count(),
        0
    );
    assert_eq!(hierarchy.aliases(hi).unwrap().count(), 2);
    let repeated = wave
        .concat(&[hi.slice(3, 2).unwrap(), lo, hi.slice(3, 2).unwrap()])
        .unwrap();
    assert_eq!(
        text(wave.sample(repeated, Time::from_ticks(5)).unwrap().as_ref()).as_deref(),
        Some("10001110")
    );
    for (range, left, right, msb, lsb) in [
        (BitRange::new(7, 0), 5, 2, 5, 2),
        (BitRange::new(0, 7), 2, 5, 5, 2),
        (BitRange::new(-2, -9), -4, -7, 5, 2),
        (BitRange::new(-2, 5), 0, 3, 5, 2),
    ] {
        assert_eq!(
            range.slice(bus, left, right).unwrap(),
            bus.slice(msb, lsb).unwrap()
        );
    }
    assert!(matches!(
        BitRange::new(0, 7).slice(bus, 5, 2),
        Err(SliceError::HdlDirectionMismatch { .. })
    ));
    assert!(matches!(
        BitRange::new(0, 7).slice(bus, -1, 5),
        Err(SliceError::HdlOutOfBounds { .. })
    ));
    assert!(matches!(
        BitRange::new(0, 6).slice(bus, 2, 5),
        Err(SliceError::HdlRangeWidthMismatch { .. })
    ));
    assert!(matches!(
        BitRange::new(i64::MIN, i64::MAX).slice(bus, 0, 1),
        Err(SliceError::HdlRangeWidthMismatch { .. })
    ));
    assert_eq!(
        BitRange::new(i64::MIN + 7, i64::MIN)
            .slice(bus, i64::MIN + 5, i64::MIN + 2)
            .unwrap(),
        bus.slice(5, 2).unwrap()
    );
    let scalar = bus.slice(0, 0).unwrap();
    assert_eq!(BitRange::new(-3, -3).slice(scalar, -3, -3).unwrap(), scalar);
    assert_eq!(
        wave.concat(&[handles[1], hi]).unwrap(),
        wave.concat(&[bus.slice(7, 4).unwrap().slice(3, 0).unwrap(), hi])
            .unwrap()
    );
    let trace = wave.trace(bus, TimeRange::all()).unwrap();
    let mut other = source(false);
    assert!(matches!(
        other.sample(bus, Time::ZERO),
        Err(Error::InvalidSignal { .. })
    ));
    assert!(matches!(
        other.concat(&[bus, lo]),
        Err(Error::InvalidSignal { .. })
    ));
    drop(wave);
    assert_eq!(trace.changes().len(), 4);
    let ValueRef::Bits(bits) = trace.changes()[0].value() else {
        panic!()
    };
    assert_eq!(bits.to_string(), "xxxx0110");
}

#[test]
fn all_missing_observed_x_and_breaks_remain_distinct() {
    let input =
        b"$var wire 4 h hi $end $var wire 4 l lo $end $enddefinitions $end #3 bxxxx h #5 b0000 l";
    let mut wave = ondas::open_bytes("unknown.vcd", input.as_slice().into()).unwrap();
    let hi = wave.hierarchy().signal("hi").unwrap();
    let lo = wave.hierarchy().signal("lo").unwrap();
    let bus = wave.concat(&[hi, lo]).unwrap();
    let upper = bus.slice(7, 4).unwrap();
    for tick in [0, 2] {
        for sample in wave.samples(&[bus, upper], Time::from_ticks(tick)).unwrap() {
            assert_eq!(text(sample.as_ref()), None);
        }
    }
    assert_eq!(
        text(wave.sample(bus, Time::from_ticks(3)).unwrap().as_ref()).as_deref(),
        Some("xxxxxxxx")
    );
    let mut selection = wave.select(&[bus, upper, bus]).unwrap();
    let mut seen = Vec::new();
    assert_eq!(
        selection
            .scan_each(TimeRange::all(), |slot, _| {
                seen.push(slot);
                ControlFlow::Break("stop")
            })
            .unwrap(),
        ControlFlow::Break("stop")
    );
    assert_eq!(seen, [0]);
    assert_eq!(
        selection
            .query(TimeRange::all(), &[0], |ctx| ctx.visit_samples(
                ctx.time(),
                &[2, 0],
                |slot, _| Ok(ControlFlow::Break(slot))
            ))
            .unwrap(),
        ControlFlow::Break(2)
    );
    assert_eq!(selection.samples(Time::from_ticks(5)).unwrap().len(), 3);
}
