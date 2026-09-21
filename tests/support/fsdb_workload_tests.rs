use std::ops::ControlFlow;

use ondas::{Logic, SampleRef, Time, TimeRange, ValueRef};

use super::{load_fixture, provider};
#[path = "fsdb_workloads.rs"]
mod workloads;

fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

#[test]
#[ignore = "requires real FSDB runtime and locked fixtures; run just conformance-fsdb"]
fn fsdb_temporal_workloads() {
    for (fixture, end) in [
        ("fsdb0010-history-short", 4096),
        ("fsdb0011-history-long", 1_048_576),
    ] {
        let fixture = load_fixture(&provider(), fixture);
        let mut wave = ondas::open_with(&fixture.path, workloads::BACKEND).unwrap();
        let signals = ["top.clock", "top.word_00", "top.word_01"]
            .map(|name| wave.hierarchy().signal(name).unwrap());
        let mut selection = wave.select(&signals).unwrap();
        let check = |time: Time, index, sample: SampleRef<'_>| {
            let SampleRef::Value {
                value: ValueRef::Bits(bits),
                changed_at,
                ..
            } = sample
            else {
                panic!("history bits must be present");
            };
            let tick = time.ticks();
            let (expected, changed, width) = if index == 0 {
                (tick % 2, tick, 1)
            } else {
                (tick / 16 + index as u64 - 1, tick / 16 * 16, 32)
            };
            assert_eq!(bits.to_string(), format!("{expected:0width$b}"));
            assert_eq!(
                changed_at,
                (changed != 0).then(|| Time::from_ticks(changed))
            );
        };
        for start in [0, end - 31] {
            let mut visits = 0;
            let candidates = workloads::adjacent(
                &mut selection,
                range(start, start + 31),
                &[0],
                &[0, 1, 2],
                |time, index, sample| {
                    check(time, index, sample);
                    visits += 1;
                },
            )
            .unwrap();
            assert_eq!(candidates, 32);
            assert_eq!(visits, if start == 0 { 282 } else { 288 });
        }
        for tick in [
            0,
            1,
            1,
            2,
            1,
            0,
            end - 3,
            end - 2,
            end - 2,
            end - 1,
            end - 2,
            end - 3,
            end,
            end,
        ] {
            let time = Time::from_ticks(tick);
            for (index, sample) in selection.samples(time).unwrap().iter().enumerate() {
                check(time, index, sample.as_ref());
            }
        }
    }
}

#[test]
#[ignore = "requires real FSDB runtime and locked fixtures; run just conformance-fsdb"]
fn fsdb_loading_window_resets() {
    let fixture = load_fixture(&provider(), "fsdb0010-history-short");
    let mut wave = ondas::open_with(&fixture.path, workloads::BACKEND).unwrap();
    let clock = wave.hierarchy().signal("top.clock").unwrap();
    let word = wave.hierarchy().signal("top.word_00").unwrap();
    // Shrinking, expanding, changed selection, and a bound beyond the file.
    for (signal, tick) in [
        (clock, 0),
        (word, 4096),
        (clock, 16),
        (clock, u64::MAX),
        (word, 1),
    ] {
        let sample = wave.sample(signal, Time::from_ticks(tick)).unwrap();
        let SampleRef::Value {
            value: ValueRef::Bits(bits),
            changed_at,
            ..
        } = sample.as_ref()
        else {
            panic!("recorded value must survive changing load windows");
        };
        let tick = tick.min(4096);
        let (value, changed, width) = if signal == clock {
            (tick % 2, tick, 1)
        } else {
            (tick / 16, tick / 16 * 16, 32)
        };
        assert_eq!(bits.to_string(), format!("{value:0width$b}"));
        assert_eq!(changed_at, (changed > 0).then(|| Time::from_ticks(changed)));
    }
}

#[test]
#[ignore = "requires real FSDB runtime and locked fixtures; run just conformance-fsdb"]
fn fsdb_conditional_workloads() {
    let fixture = load_fixture(&provider(), workloads::WIDE);
    let mut wave = ondas::open_with(&fixture.path, workloads::BACKEND).unwrap();
    for shared in [false, true] {
        let signals = workloads::wide_signals(&wave, shared);
        let mut selection = wave.select(&signals).unwrap();
        // Covers distinct controls, simultaneous drivers, repeated driver indices,
        // and different selection entries referring to the same base history.
        for drivers in [&[0][..], &[0, 1, 0][..]] {
            for (start, after, period, stop) in [
                (0, 0, 2, false),
                (0, 0, 64, false),
                (2048, 2048, 64, false),
                (0, 128, 64, true),
                (2048, 2176, 64, true),
                (0, 3968, 64, true),
                (2048, 3968, 64, true),
                (0, 4097, 64, true),
                (2048, 4097, 64, true),
            ] {
                let mut expected: Vec<_> = (start..=4096)
                    .filter(|tick| *tick >= after && tick % period == 1)
                    .collect();
                if stop {
                    expected.truncate(1);
                }
                for sequential in [false, true] {
                    let mut actual = Vec::new();
                    let accept = |time: Time, high| {
                        high && time.ticks() >= after && time.ticks() % period == 1
                    };
                    let observe = |time: Time, value: ValueRef<'_>| {
                        let ValueRef::Bits(bits) = value else {
                            panic!("wide bits");
                        };
                        assert_eq!(bits.width(), 4096);
                        assert_eq!(bits.bit(0), Some(Logic::One));
                        assert!(bits.iter_msb().take(4095).all(|bit| bit == Logic::Zero));
                        actual.push(time.ticks());
                        if stop {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    };
                    let counts = if sequential {
                        workloads::sequential(
                            &mut selection,
                            range(start, 4096),
                            drivers,
                            accept,
                            observe,
                        )
                    } else {
                        workloads::conditional(
                            &mut selection,
                            range(start, 4096),
                            drivers,
                            accept,
                            observe,
                        )
                    }
                    .unwrap();
                    assert_eq!(actual, expected);
                    assert_eq!(counts.accepted, expected.len() as u64);
                    let last = if stop {
                        expected.first().copied().unwrap_or(4096)
                    } else {
                        4096
                    };
                    assert_eq!(counts.candidates, last - start + 1);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires real FSDB runtime and locked fixtures; run just conformance-fsdb"]
fn fsdb_typed_temporal_workload() {
    let fixture = load_fixture(&provider(), "fsdb0017-typed-records");
    let mut wave = ondas::open_with(&fixture.path, workloads::BACKEND).unwrap();
    let signals = [
        "top.trigger",
        "top.logic4",
        "top.real64",
        "top.real32",
        "top.text_short",
        "top.text_long",
    ]
    .map(|name| wave.hierarchy().signal(name).unwrap());
    let mut selection = wave.select(&signals).unwrap();
    let mut observations = Vec::new();
    let candidates = workloads::adjacent(
        &mut selection,
        range(2048, 2080),
        &[0],
        &[0, 1, 2, 3, 4, 5],
        |time, index, sample| {
            observations.push((time, index, format!("{sample:?}")));
        },
    )
    .unwrap();
    assert_eq!(candidates, 33);
    assert_eq!(observations.len(), 33 * 3 * 6);
    // Point reads are the independently invoked normalized reference; the public
    // provider's typed/event oracle is also checked by full_fsdb_pool.
    let expected: Vec<_> = (2047..=2080)
        .map(|tick| selection.samples(Time::from_ticks(tick)).unwrap())
        .collect();
    for (time, index, sample) in observations {
        assert_eq!(
            sample,
            format!(
                "{:?}",
                expected[(time.ticks() - 2047) as usize][index].as_ref()
            )
        );
    }
}
