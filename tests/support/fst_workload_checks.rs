use std::ops::ControlFlow;

use ondas::{Logic, SampleRef, Time, TimeRange, ValueRef};

use super::{fixture_catalog as fixtures, fst_workloads as workloads};

fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn fst_wide_normalized_whole_and_stable_projection() {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), "fst0083-wide-compact-toggle");
    let mut wave = ondas::open_with(&path, "fst-lib").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    for signal in [wide, wide.slice(4095, 1).unwrap()] {
        let mut selection = wave.select(&[signal]).unwrap();
        let mut initials = 0;
        let mut changes = Vec::new();
        let _ = selection
            .scan(range(2048, 2052), |record| {
                match record {
                    ondas::ScanRef::Initial {
                        value, changed_at, ..
                    } => {
                        initials += 1;
                        assert_eq!(
                            changed_at,
                            if signal == wide {
                                Some(Time::from_ticks(2047))
                            } else {
                                None
                            }
                        );
                        let ValueRef::Bits(bits) = value else {
                            panic!("bits")
                        };
                        assert_eq!(bits.width(), signal.width().unwrap());
                    }
                    ondas::ScanRef::Change { time, value, .. } => {
                        let ValueRef::Bits(bits) = value else {
                            panic!("bits")
                        };
                        assert_eq!(
                            bits.bit(0),
                            Some(if time.ticks() % 2 == 0 {
                                Logic::Zero
                            } else {
                                Logic::One
                            })
                        );
                        changes.push(time.ticks());
                    }
                    _ => panic!("unexpected scan record"),
                }
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(initials, 1);
        assert_eq!(
            changes,
            if signal == wide {
                vec![2048, 2049, 2050, 2051, 2052]
            } else {
                vec![]
            }
        );
    }
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn fst_composed_wide_observations() {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), "fst0083-wide-compact-toggle");
    let mut wave = ondas::open_with(&path, "fst-lib").unwrap();
    let scalar = wave.hierarchy().signal("top.control").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let low = wide.slice(0, 0).unwrap();
    for driver in [scalar, low] {
        let mut selection = wave.select(&[driver, low, wide]).unwrap();
        for (start, end, stride, limit, expected_times) in [
            (
                1,
                4096,
                64,
                usize::MAX,
                (1..=4096).step_by(64).collect::<Vec<_>>(),
            ),
            (2, 4096, 8192, usize::MAX, vec![]),
            (2, 4096, 2, usize::MAX, (3..=4095).step_by(2).collect()),
            (2, 4096, 2048, 1, vec![2049]),
            (2, 4096, 2048, usize::MAX, vec![2049]),
            (4000, 4096, 64, usize::MAX, vec![4033]),
        ] {
            let mut reference = Vec::new();
            let count = workloads::conditional_scan(
                &mut selection,
                range(start, end),
                stride,
                limit,
                |time, value| {
                    reference.push((time.ticks(), value.to_owned()));
                },
            )
            .unwrap();
            assert_eq!(count, expected_times.len());
            assert_eq!(
                reference.iter().map(|row| row.0).collect::<Vec<_>>(),
                expected_times
            );
            for (_, value) in &reference {
                let ValueRef::Bits(bits) = value.as_ref() else {
                    panic!("expected bit payload")
                };
                assert_eq!(bits.width(), 4096);
                assert_eq!(bits.bit(0), Some(Logic::One));
                assert!((1..4096).all(|bit| bits.bit(bit) == Some(Logic::Zero)));
            }
            for eager in [false, true] {
                let mut output = Vec::new();
                assert_eq!(
                    workloads::conditional_query(
                        &mut selection,
                        range(start, end),
                        stride,
                        limit,
                        eager,
                        |time, value| {
                            output.push((time.ticks(), value.to_owned()));
                        }
                    )
                    .unwrap(),
                    count
                );
                assert_eq!(output.len(), reference.len());
                for ((time, value), (expected_time, expected_value)) in
                    output.iter().zip(&reference)
                {
                    assert_eq!(time, expected_time);
                    assert_eq!(bits(value.as_ref()), bits(expected_value.as_ref()));
                }
            }
        }
    }
    let mut selection = wave.select(&[scalar, low]).unwrap();
    for ticks in [
        [4094, 4094, 4094],
        [4093, 4094, 4095],
        [4095, 4094, 4093],
        [4094, 4095, 4094],
    ] {
        for tick in ticks {
            let _ = selection
                .visit_samples(Time::from_ticks(tick), |sample| {
                    assert_toggle(tick, sample);
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
        }
    }
    for (start, end, expected_reads) in [(0, 3, 20), (1, 3, 18), (4093, 4095, 18)] {
        assert_eq!(
            workloads::adjacent_query(&mut selection, range(start, end), |at, sample| {
                assert_toggle(at.ticks(), sample);
            })
            .unwrap(),
            expected_reads
        );
    }
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn fst_candidates_deduplicate_raw_activity_and_reuse_after_stop() {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), "fst0083-wide-compact-toggle");
    let mut wave = ondas::open_with(&path, "fst-lib").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let low = wide.slice(0, 0).unwrap();
    let stable = wide.slice(4095, 1).unwrap();
    let mut selection = wave.select(&[low, stable, low]).unwrap();
    let window = range(2048, 2052);
    let mut required = std::collections::BTreeSet::new();
    let _ = selection
        .scan(window, |record| {
            if let ondas::ScanRef::Change { time, .. } = record {
                required.insert(time);
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    let mut times = Vec::new();
    let _ = selection
        .scan_candidate_times(window, |time| {
            times.push(time);
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert!(!required.is_empty());
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(required.is_subset(&times.iter().copied().collect()));
    assert!(
        times
            .iter()
            .all(|time| (2048..=2052).contains(&time.ticks()))
    );
    let mut calls = 0;
    assert_eq!(
        selection
            .scan_candidate_times(window, |time| {
                calls += 1;
                ControlFlow::Break(time)
            })
            .unwrap(),
        ControlFlow::Break(times[0])
    );
    assert_eq!(calls, 1);
    let mut replay = Vec::new();
    let _ = selection
        .scan_candidate_times(window, |time| {
            replay.push(time);
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(replay, times);
    for window in [range(2052, 2048), range(4097, 4100)] {
        let _ = selection
            .scan_candidate_times::<()>(window, |_| {
                panic!("unexpected candidate in an empty or past-end window")
            })
            .unwrap_or_else(|error| panic!("{error}"));
    }
    let mut empty = wave.select(&[]).unwrap();
    assert_eq!(
        empty
            .scan_candidate_times(TimeRange::all(), |_| ControlFlow::Break(()))
            .unwrap(),
        ControlFlow::Continue(())
    );
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn fst_string_candidates_preserve_values_and_reuse_after_stop() {
    for fixture in [
        "fst0044-overlay-tb-issue-21",
        "fst0060-manytypes2",
        "fst0061-shortstring",
    ] {
        let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
        for bytes in [false, true] {
            let mut wave = if bytes {
                ondas::open_bytes_with(
                    "waveform.fst",
                    std::fs::read(&path).unwrap().into(),
                    "fst-lib",
                )
                .unwrap()
            } else {
                ondas::open_with(&path, "fst-lib").unwrap()
            };
            let mut signals = wave
                .hierarchy()
                .signals()
                .filter(|signal| signal.encoding() == ondas::Encoding::String)
                .collect::<Vec<_>>();
            assert!(!signals.is_empty());
            signals.push(signals[0]);
            let mut selection = wave.select(&signals).unwrap();
            let read_values = |selection: &mut ondas::Selection<'_>| {
                let mut values = Vec::new();
                let _ = selection
                    .scan_each(TimeRange::all(), |slot, record| {
                        if let ondas::ScanRef::Change { time, value, .. } = record {
                            let ValueRef::String(text) = value else {
                                panic!("expected string value")
                            };
                            values.push((slot, time, text.to_owned()));
                        }
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                values
            };
            let before = read_values(&mut selection);
            assert!(!before.is_empty());
            if fixture == "fst0061-shortstring" {
                assert!(
                    before
                        .iter()
                        .any(|(_, _, text)| text == &format!("{:<50}", "En lång röd räv"))
                );
            }
            let first = before.iter().map(|(_, time, _)| *time).min().unwrap();
            let last = before.iter().map(|(_, time, _)| *time).max().unwrap();
            for window in [
                TimeRange::all(),
                TimeRange::point(first),
                TimeRange::point(last),
            ] {
                let mut times = Vec::new();
                let _ = selection
                    .scan_candidate_times(window, |time| {
                        times.push(time);
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(
                    times.iter().all(|&time| time >= window.start()
                        && window.end().is_none_or(|end| time <= end))
                );
                for (_, time, _) in &before {
                    if *time >= window.start() && window.end().is_none_or(|end| *time <= end) {
                        assert!(times.contains(time));
                    }
                }
                let mut calls = 0;
                assert_eq!(
                    selection
                        .scan_candidate_times(window, |time| {
                            calls += 1;
                            ControlFlow::Break(time)
                        })
                        .unwrap(),
                    ControlFlow::Break(times[0])
                );
                assert_eq!(calls, 1);
                let mut replay = Vec::new();
                let _ = selection
                    .scan_candidate_times(window, |time| {
                        replay.push(time);
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                assert_eq!(replay, times);
            }
            assert_eq!(read_values(&mut selection), before);
        }
    }
}

fn bits(value: ValueRef<'_>) -> String {
    let ValueRef::Bits(bits) = value else {
        panic!("expected bit value")
    };
    bits.to_string()
}

fn assert_toggle(tick: u64, sample: SampleRef<'_>) {
    let SampleRef::Value {
        value: ValueRef::Bits(bits),
        changed_at,
        ..
    } = sample
    else {
        panic!("expected bit sample")
    };
    assert_eq!(
        bits.bit(0),
        Some(if tick.is_multiple_of(2) {
            Logic::Zero
        } else {
            Logic::One
        })
    );
    assert_eq!(changed_at, (tick != 0).then_some(Time::from_ticks(tick)));
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn fst_composed_sections_and_wrapper() {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), "fst0015-scr1-max-ahb-coremark");
    let mut wave = ondas::open_with(&path, "fst-lib").unwrap();
    let signals = ["TOP.clk", "TOP.$unit.SCR1_ARCH_RST_VECTOR"]
        .map(|name| wave.hierarchy().signal(name).unwrap());
    let mut selection = wave.select(&signals).unwrap();
    for boundary in [745342, 3248312, 5812392] {
        for start in [boundary - 1, boundary, boundary + 1] {
            let window = range(start, start + 44);
            let mut required = std::collections::BTreeSet::new();
            let _ = selection
                .scan(window, |record| {
                    if let ondas::ScanRef::Change { time, .. } = record {
                        required.insert(time);
                    }
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            let mut candidates = Vec::new();
            let _ = selection
                .scan_candidate_times(window, |time| {
                    candidates.push(time);
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert!(candidates.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(
                candidates
                    .iter()
                    .all(|time| (start..=start + 44).contains(&time.ticks()))
            );
            assert!(!required.is_empty());
            assert!(required.is_subset(&candidates.into_iter().collect()));
        }
    }
    for start in [1000, 745320, 5812370] {
        // Resolve independent point observations outside query callbacks.
        let expected = (start - 1..=start + 44)
            .map(|tick| {
                let mut values = Vec::new();
                let _ = selection
                    .visit_samples(Time::from_ticks(tick), |sample| {
                        let SampleRef::Value {
                            value, changed_at, ..
                        } = sample
                        else {
                            panic!("missing value")
                        };
                        values.push((bits(value), changed_at));
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                values
            })
            .collect::<Vec<_>>();
        let mut slot = 0;
        let reads =
            workloads::adjacent_query(&mut selection, range(start, start + 44), |at, sample| {
                let SampleRef::Value {
                    value, changed_at, ..
                } = sample
                else {
                    panic!("missing value")
                };
                assert_eq!(
                    (bits(value), changed_at),
                    expected[(at.ticks() - (start - 1)) as usize][slot]
                );
                slot = 1 - slot;
            })
            .unwrap();
        assert!(reads > 0);
        assert_eq!(reads % 6, 0);
    }
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), "fst0050-wellen-32");
    assert_eq!(
        std::fs::read(&path).unwrap()[0],
        254,
        "whole-file gzip wrapper"
    );
    let mut wave = ondas::open_with(&path, "fst-lib").unwrap();
    let signal = wave.hierarchy().signal("wellen_32.spisub_s.cs_n").unwrap();
    let mut selection = wave.select(&[signal]).unwrap();
    let window = range(1, 2_000_000_000);
    let expected = selection
        .scan(window, |record| {
            if let ondas::ScanRef::Change {
                time,
                value: ValueRef::Bits(bits),
                ..
            } = record
                && bits.bit(0) == Some(Logic::One)
            {
                return ControlFlow::Break(time);
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert!(expected.is_break());
    assert_eq!(
        workloads::first_high(&mut selection, window).unwrap(),
        expected
    );
}
