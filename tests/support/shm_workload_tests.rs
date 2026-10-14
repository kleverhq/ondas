use std::ops::ControlFlow;

use ondas::{Encoding, Logic, SampleRef, Time, ValueRef};

use super::{fixture_support, root};
#[path = "shm_workloads.rs"]
mod workloads;

fn open(name: &str) -> ondas::Waveform {
    let (path, _) = fixture_support::load_artifact(&root(), name);
    ondas::open_with(path, workloads::BACKEND).unwrap()
}

#[test]
#[ignore = "requires Xcelium and installed SHM fixtures"]
fn shm_history_workload_consumers() {
    for (fixture, end) in workloads::HISTORY {
        let mut wave = open(fixture);
        let signals = ["top.phase_0", "top.phase_1", "top.phase_2", "top.phase_3"]
            .map(|path| wave.hierarchy().signal(path).unwrap());
        let mut selection = wave.select(&signals).unwrap();
        for start in [0, end - 63] {
            let times = [
                start,
                start + 1,
                start + 1,
                start + 3,
                start + 2,
                start,
                start + 63,
            ]
            .map(Time::from_ticks);
            let times = times
                .into_iter()
                .chain((start + 56..=start + 63).map(Time::from_ticks))
                .collect::<Vec<_>>();
            assert_eq!(
                workloads::series(&mut selection, &times, |time, index, sample| {
                    let SampleRef::Value {
                        value: ValueRef::Bits(bits),
                        changed_at,
                        ..
                    } = sample
                    else {
                        panic!("phase must be present");
                    };
                    let period = 1 << index;
                    assert_eq!(
                        bits.bit(0),
                        Some(if time.ticks() / period % 2 == 1 {
                            Logic::One
                        } else {
                            Logic::Zero
                        })
                    );
                    let changed = time.ticks() / period * period;
                    assert_eq!(changed_at, (changed > 0).then(|| Time::from_ticks(changed)));
                })
                .unwrap(),
                60
            );
            let window = workloads::range(start, start + 63);
            assert_eq!(
                workloads::candidate_count(&mut selection, window).unwrap(),
                64
            );
            // Four entering-state records, then each normalized phase change.
            let expected = if start == 0 { 0 } else { 4 }
                + (0..4)
                    .map(|index| {
                        (start..=start + 63)
                            .filter(|tick| *tick % (1 << index) == 0)
                            .count() as u64
                    })
                    .sum::<u64>();
            assert_eq!(
                workloads::scan_count(&mut selection, window).unwrap(),
                expected
            );
            assert_eq!(
                workloads::first_change(&mut selection, window).unwrap(),
                ControlFlow::Break(Time::from_ticks(start))
            );
            let traces = selection.traces(window).unwrap();
            assert_eq!(traces.len(), 4);
        }
    }
}

#[test]
#[ignore = "requires Xcelium and installed SHM fixtures"]
fn shm_bank_and_topology_workload_consumers() {
    for fixture in [workloads::BANK, workloads::TOPOLOGY] {
        let mut wave = open(fixture);
        for class in ["dense", "sparse"] {
            let signals = (0..64)
                .map(|index| {
                    wave.hierarchy()
                        .signal(&format!("top.bank[{index}].{class}"))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                signals
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                64
            );
            for count in [1, 16, 64] {
                let mut selection = wave.select(&signals[..count]).unwrap();
                workloads::series(
                    &mut selection,
                    &[Time::from_ticks(255), Time::from_ticks(256)],
                    |time, index, sample| {
                        let SampleRef::Value {
                            value: ValueRef::Bits(bits),
                            ..
                        } = sample
                        else {
                            panic!("bank value must be present");
                        };
                        let tick = if class == "dense" {
                            time.ticks()
                        } else {
                            time.ticks() / 128
                        };
                        assert_eq!(
                            bits.bit(0),
                            Some(if (tick + index as u64) % 2 == 1 {
                                Logic::One
                            } else {
                                Logic::Zero
                            })
                        );
                    },
                )
                .unwrap();
                let window = workloads::range(257, 320);
                assert_eq!(
                    workloads::scan_count(&mut selection, window).unwrap(),
                    count as u64 * if class == "dense" { 65 } else { 1 }
                );
                assert_eq!(
                    workloads::candidate_count(&mut selection, window).unwrap(),
                    if class == "dense" { 64 } else { 0 }
                );
            }
        }
        if fixture == workloads::TOPOLOGY {
            assert_eq!(
                wave.hierarchy()
                    .variables()
                    .filter(|v| v.name() == "value")
                    .count(),
                16384
            );
        }
    }
}

#[test]
#[ignore = "requires Xcelium and installed SHM fixtures"]
fn shm_wide_composed_workload_consumers() {
    let mut wave = open(workloads::WIDE);
    let scalar = wave.hierarchy().signal("top.control").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    assert_eq!(wide.encoding(), Encoding::Bits { width: 4096 });
    let high = wide.slice(4095, 4080).unwrap();
    let low = wide.slice(0, 0).unwrap();
    let opposite = wave
        .hierarchy()
        .signal("top.wide_msb")
        .unwrap()
        .slice(4095, 4095)
        .unwrap();
    for control in [scalar, opposite] {
        for eager in [false, true] {
            let mut selection = wave.select(&[scalar, control, wide]).unwrap();
            let mut ticks = Vec::new();
            let counts = workloads::payload(
                &mut selection,
                workloads::range(0, 63),
                16,
                eager,
                |time, value| {
                    let ValueRef::Bits(bits) = value else {
                        panic!("wide bits required");
                    };
                    assert_eq!(bits.to_string(), format!("{}1", "0".repeat(4095)));
                    ticks.push(time.ticks());
                    ControlFlow::Continue(())
                },
            )
            .unwrap();
            assert_eq!(ticks, [1, 17, 33, 49]);
            assert_eq!(
                counts,
                workloads::Counts {
                    candidates: 64,
                    accepted: 4
                }
            );
            let mut first = Vec::new();
            let counts = workloads::payload(
                &mut selection,
                workloads::range(0, 63),
                16,
                eager,
                |time, _| {
                    first.push(time.ticks());
                    ControlFlow::Break(())
                },
            )
            .unwrap();
            assert_eq!(first, [1]);
            assert_eq!(
                counts,
                workloads::Counts {
                    candidates: 2,
                    accepted: 1
                }
            );
        }
    }
    let mut selection = wave.select(&[wide, low, high, low]).unwrap();
    assert_eq!(
        workloads::candidate_count(&mut selection, workloads::range(17, 80)).unwrap(),
        64
    );
    assert_eq!(
        workloads::scan_count(&mut selection, workloads::range(17, 80)).unwrap(),
        196
    );
    // Shared wide bases and duplicate entries remain separate output slots.
    assert_eq!(selection.traces(workloads::range(17, 80)).unwrap().len(), 4);
}

#[test]
#[ignore = "requires Xcelium and installed SHM fixtures"]
fn shm_typed_workload_consumer() {
    let mut wave = open(workloads::TYPED);
    let signals = [
        "top.real_value",
        "top.text_short",
        "top.text_long",
        "top.pulse",
    ]
    .map(|path| wave.hierarchy().signal(path).unwrap());
    let mut selection = wave.select(&signals).unwrap();
    let times = [0, 3, 5, 6, 7].map(Time::from_ticks);
    assert_eq!(
        workloads::series(&mut selection, &times, |time, index, sample| {
            match (index, sample) {
                (
                    0,
                    SampleRef::Value {
                        value: ValueRef::Real(value),
                        ..
                    },
                ) => {
                    let expected = if time.ticks() >= 7 {
                        2.5_f64
                    } else if time.ticks() >= 5 {
                        -0.0_f64
                    } else {
                        1.2345678901234567_f64
                    };
                    assert_eq!(value.to_bits(), expected.to_bits());
                }
                (
                    1,
                    SampleRef::Value {
                        value: ValueRef::String(value),
                        ..
                    },
                ) => assert_eq!(value, if time.ticks() >= 6 { "xyz" } else { "abc" }),
                (
                    2,
                    SampleRef::Value {
                        value: ValueRef::String(value),
                        ..
                    },
                ) => assert_eq!(
                    value,
                    if time.ticks() >= 6 { "B" } else { "A" }.repeat(2048)
                ),
                (3, SampleRef::Event { occurrences, .. }) => assert_eq!(
                    occurrences,
                    match time.ticks() {
                        3 => 3,
                        5 => 1,
                        _ => 0,
                    }
                ),
                _ => panic!("unexpected typed sample"),
            }
        })
        .unwrap(),
        20
    );
}
