use std::{collections::HashSet, ops::ControlFlow};

use ondas::{Error, HierarchyPath, Logic, SampleRef, Signedness, Time, ValueRef};

use super::{
    fixture_support as fixtures, ghw_workloads as coordinates, hierarchy, load_fixture, open,
    query_workloads as workloads, sample_queries, scan_queries, window_queries,
};
use coordinates::range;

fn bits(value: ValueRef<'_>) -> String {
    let ValueRef::Bits(bits) = value else {
        panic!("expected bits")
    };
    bits.to_string()
}

fn bit_sample(sample: SampleRef<'_>, tick: u64) {
    let SampleRef::Value {
        value, changed_at, ..
    } = sample
    else {
        panic!("missing value")
    };
    assert_eq!(bits(value), if tick % 2 == 1 { "1" } else { "0" });
    assert_eq!(changed_at, (tick != 0).then_some(Time::from_ticks(tick)));
}

#[test]
#[ignore = "requires installed fixtures; run just conformance"]
fn ghw_wide_consumers_match_authored_and_native_evidence() {
    let fixture = load_fixture(&fixtures::root(), coordinates::WIDE);
    for bytes in [false, true] {
        let mut wave = open(&fixture, bytes).unwrap();
        let wide = wave.hierarchy().signal(coordinates::WIDE_PATH).unwrap();
        let expected_signals = hierarchy(&wave, &fixture.oracle, false);
        assert_eq!(expected_signals["signal-1"], [wide], "oracle wide identity");
        let scalar = wave.hierarchy().signal(coordinates::CONTROL_PATH).unwrap();
        let low = wide.slice(0, 0).unwrap();
        let high = wide.slice(4095, 1).unwrap();
        // Derive projected complete windows from native raw evidence before normalization.
        let expected = &fixture.oracle["signals"]["signal-1"];
        assert_eq!(expected["encoding"]["width"], 4096);
        for signal in [wide, low, high] {
            let mut windows = expected["windows"].as_array().unwrap().clone();
            // A bounded complete prefix keeps exhaustive point checks small.
            for window in &mut windows {
                window["end"] = serde_json::json!("129");
                window["changes"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|change| super::tick(&change["time"]) <= 129);
            }
            if signal != wide {
                for window in &mut windows {
                    let project = |value: &mut serde_json::Value| {
                        let raw = value["bits"].as_str().unwrap();
                        *value = serde_json::json!({"bits": if signal == low { &raw[4095..] } else { &raw[..4095] }});
                    };
                    if !window["initial"].is_null() {
                        project(&mut window["initial"]["value"]);
                        window["initial"]
                            .as_object_mut()
                            .unwrap()
                            .remove("changed_at");
                    }
                    for change in window["changes"].as_array_mut().unwrap() {
                        project(&mut change["value"]);
                    }
                }
            }
            for window in &windows {
                window_queries(&mut wave, signal, window, &windows, "GHW wide projection");
            }
        }
        scan_queries(
            &mut wave,
            &[wide, low, high, wide, low],
            range(63, 67),
            "GHW duplicate projections",
        );
        for driver in [scalar, low] {
            let mut selection = wave.select(&[driver, low, wide]).unwrap();
            for (start, end, stride, limit, times) in [
                (
                    1,
                    4096,
                    64,
                    usize::MAX,
                    (1..=4096).step_by(64).collect::<Vec<_>>(),
                ),
                (2, 64, 64, usize::MAX, vec![]),
                (2, 129, 64, usize::MAX, vec![65, 129]),
                (2, 129, 64, 1, vec![65]),
            ] {
                let observe = |time: Time, value: ValueRef<'_>| {
                    let ValueRef::Bits(bits) = value else {
                        panic!("bits")
                    };
                    assert_eq!(bits.width(), 4096);
                    assert_eq!(bits.bit(0), Some(Logic::One));
                    assert!((1..4096).all(|i| bits.bit(i) == Some(Logic::Zero)));
                    time.ticks()
                };
                let mut scan = Vec::new();
                assert_eq!(
                    workloads::conditional_scan(
                        &mut selection,
                        range(start, end),
                        stride,
                        limit,
                        |t, v| scan.push(observe(t, v))
                    )
                    .unwrap(),
                    times.len()
                );
                assert_eq!(scan, times);
                for eager in [false, true] {
                    let mut query = Vec::new();
                    assert_eq!(
                        workloads::conditional_query(
                            &mut selection,
                            range(start, end),
                            stride,
                            limit,
                            eager,
                            |t, v| query.push(observe(t, v))
                        )
                        .unwrap(),
                        times.len()
                    );
                    assert_eq!(query, times);
                }
            }
        }
        let mut selection = wave.select(&[scalar, low]).unwrap();
        for schedule in [[64, 64, 64], [63, 64, 65], [65, 64, 63], [64, 65, 64]] {
            for tick in schedule {
                let _ = selection
                    .visit_samples(Time::from_ticks(tick), |sample| {
                        bit_sample(sample, tick);
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
            }
        }
        for (start, end, count) in [(0, 3, 20), (1, 3, 18), (63, 65, 18)] {
            assert_eq!(
                workloads::adjacent_query(&mut selection, range(start, end), |at, sample| {
                    bit_sample(sample, at.ticks())
                })
                .unwrap(),
                count
            );
        }
        assert_eq!(
            workloads::first_high(&mut wave.select(&[scalar]).unwrap(), range(2, 129)).unwrap(),
            ControlFlow::Break(Time::from_ticks(3))
        );
    }
}

#[test]
#[ignore = "requires installed fixtures; run just conformance"]
fn ghw_topology_unique_sparse_probes_and_unrelated_controls() {
    for name in coordinates::TOPOLOGIES {
        let fixture = load_fixture(&fixtures::root(), name);
        let mut wave = open(&fixture, false).unwrap();
        let probes = (0..64)
            .map(|i| wave.hierarchy().signal(&coordinates::probe(i)).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(probes.iter().collect::<HashSet<_>>().len(), 64);
        for count in [1, 16, 64] {
            let mut selection = wave.select(&probes[..count]).unwrap();
            for tick in [0, 1, 64, 65, 129, 193, 256] {
                let expected = if tick == 0 || ((tick - 1) / 64 + 1) % 2 == 0 {
                    "0"
                } else {
                    "1"
                };
                let _ = selection
                    .visit_samples(Time::from_ticks(tick), |sample| {
                        let SampleRef::Value { value, .. } = sample else {
                            panic!("missing probe")
                        };
                        assert_eq!(bits(value), expected);
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
            }
            let mut candidates = Vec::new();
            let _ = selection
                .scan_candidate_times(range(2, 193), |t| {
                    candidates.push(t.ticks());
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert!(candidates.windows(2).all(|p| p[0] < p[1]));
            assert!([65, 129, 193].iter().all(|t| candidates.contains(t)));
        }
        let constant = wave.hierarchy().signal(coordinates::CONSTANT_PATH).unwrap();
        let noise = wave.hierarchy().signal(coordinates::NOISE_PATH).unwrap();
        assert_eq!(
            noise.width(),
            Some(if name == coordinates::TOPOLOGIES[2] {
                16384
            } else {
                16
            })
        );
        let mut selection = wave.select(&[constant]).unwrap();
        let _ = selection
            .scan_candidate_times::<()>(range(2, 256), |_| {
                panic!("unrelated writes are not selected activity")
            })
            .unwrap();
    }
}

#[test]
#[ignore = "requires installed fixtures; run just conformance"]
fn ghw_native_types_preserve_metadata_nulls_and_source_names() {
    let fixture = load_fixture(&fixtures::root(), "ghw/ghw0095-native-types");
    for bytes in [false, true] {
        let mut wave = open(&fixture, bytes).unwrap();
        hierarchy(&wave, &fixture.oracle, true);
        let h = wave.hierarchy();
        assert_eq!(h.variables().count(), 33);
        assert_eq!(h.signals().count(), 32);
        for name in ["i32", "i64", "p32", "p64"] {
            assert_eq!(
                h.variable(&format!("top.{name}")).unwrap().signedness(),
                Some(Signedness::Signed)
            );
        }
        assert_eq!(h.variable("top.state").unwrap().signedness(), None);
        assert!(h.variable("top.empty_vector").unwrap().signal().is_none());
        assert!(h.variable("top.empty_vector").unwrap().range().is_none());
        assert_eq!(h.scope("top.empty_array").unwrap().children().count(), 0);
        assert_eq!(h.scope("top.P0").unwrap().kind(), "process");
        let exact = HierarchyPath::from_components(["top", "\\bus.name [x]\\"]);
        assert!(h.signal_path(&exact).is_ok());
        for (name, width, value) in [
            ("i32", 32, -17_i64),
            ("i64", 64, -9223372036854775807),
            ("p32", 32, -9),
            ("p64", 64, 9223372036854775807),
        ] {
            let signal = wave.hierarchy().signal(&format!("top.{name}")).unwrap();
            sample_queries(
                &mut wave,
                signal,
                &serde_json::json!({"time":"0","value":{"bits":format!("{:0width$b}", (value as u64)&(u64::MAX >> (64-width)),width=width as usize)}}),
                0,
                "GHW signed storage",
            );
        }
        let raw = wave
            .hierarchy()
            .signal_path(&HierarchyPath::from_components([
                "top",
                "raw_nested",
                "[0]",
            ]))
            .unwrap();
        let resolved = wave
            .hierarchy()
            .signal_path(&HierarchyPath::from_components([
                "top",
                "resolved_nested",
                "[0]",
            ]))
            .unwrap();
        for tick in [0, 1_000_000] {
            let values = wave
                .samples(&[raw, resolved], Time::from_ticks(tick))
                .unwrap();
            for sample in values {
                let SampleRef::Value { value, .. } = sample.as_ref() else {
                    panic!("missing")
                };
                assert_eq!(bits(value), if tick == 0 { "0" } else { "1" });
            }
        }
    }
}

#[test]
#[ignore = "requires installed fixtures; run just conformance"]
fn ghw_issue_146_generated_instances_follow_source_bit_order() {
    let fixture = load_fixture(&fixtures::root(), "ghw/ghw0096-generate-order");
    for bytes in [false, true] {
        let mut wave = open(&fixture, bytes).unwrap();
        let whole = wave.hierarchy().signal("top.y").unwrap();
        let gen0 = wave.hierarchy().signal("top.gen(0).dut.y").unwrap();
        let gen1 = wave.hierarchy().signal("top.gen(1).dut.y").unwrap();
        let selected = [
            whole,
            gen0,
            gen1,
            whole.slice(0, 0).unwrap(),
            whole.slice(1, 1).unwrap(),
            whole,
        ];
        for (tick, expected) in [
            (1_000_000, ["10", "0", "1", "0", "1", "10"]),
            (11_000_000, ["01", "1", "0", "1", "0", "01"]),
        ] {
            let actual = wave.samples(&selected, Time::from_ticks(tick)).unwrap();
            for (sample, expected) in actual.iter().zip(expected) {
                let SampleRef::Value { value, .. } = sample.as_ref() else {
                    panic!("missing")
                };
                assert_eq!(bits(value), expected);
            }
        }
        scan_queries(
            &mut wave,
            &selected,
            range(0, 11_000_000),
            "GHW generated aliases",
        );
    }
}

#[test]
#[ignore = "requires installed fixtures; run just conformance"]
fn ghw_issue_99_point_neighbors_and_projected_complete_window() {
    let fixture = load_fixture(&fixtures::root(), "ghw/ghw0094-surfer-issue-547");
    for bytes in [false, true] {
        let mut wave = open(&fixture, bytes).unwrap();
        let addr = wave
            .hierarchy()
            .signal("cpu_core_tb.uut.reg_file.addr_c")
            .unwrap();
        for (tick, expected) in [
            (149_999_999, "00000"),
            (150_000_000, "00001"),
            (151_000_000, "00001"),
            (152_000_000, "00000"),
        ] {
            sample_queries(
                &mut wave,
                addr,
                &serde_json::json!({"time":tick.to_string(),"value":{"bits":expected}}),
                tick,
                "GHW issue 99",
            );
        }
        scan_queries(
            &mut wave,
            &[
                addr,
                addr.slice(0, 0).unwrap(),
                addr.slice(4, 1).unwrap(),
                addr,
            ],
            range(149_000_000, 153_000_000),
            "GHW issue 99 projections",
        );
    }
}

#[test]
#[ignore = "requires installed fixtures; run just conformance"]
fn ghw_historical_issue_538_has_missing_nested_bounds() {
    let (path, _) = fixtures::load_artifact(&fixtures::root(), "ghw/ghw0002-ghdl-issue-538");
    for result in [
        ondas::open_with(&path, "ghw-native"),
        ondas::open_bytes_with(
            "waveform.ghw",
            std::fs::read(&path).unwrap().into(),
            "ghw-native",
        ),
    ] {
        let Err(Error::Malformed {
            backend,
            message,
            format,
        }) = result
        else {
            panic!("missing inner bounds must be malformed")
        };
        assert_eq!(backend, "ghw-native");
        assert_eq!(format, ondas::Format::Ghw);
        assert!(message.contains("byte 909") && message.contains("type tag 35"));
    }
}
