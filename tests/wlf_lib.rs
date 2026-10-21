//! Directed SDK integration checks; ordinary CI requires no vendor installation.
#![cfg(feature = "wlf-lib")]
use ondas::{Error, Format, Sample, Time, TimeRange, Value};
use std::{ops::ControlFlow, path::PathBuf, sync::Arc};
fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/wlf")
        .join(name)
        .join("waveform.wlf")
}
fn counter() -> PathBuf {
    path("wlf0038-count4b-3-2-vsim")
}
fn bits(sample: Sample) -> String {
    let Sample::Value {
        value: Value::Bits(bits),
        ..
    } = sample
    else {
        panic!("expected bit sample")
    };
    bits.as_ref().to_string()
}

#[test]
#[ignore = "requires Questa and installed enum fixtures; run just conformance-wlf"]
fn wlf_enum_tables_and_samples() {
    for fixture in ["wlf0083-enum-core", "wlf0084-enum-xz"] {
        let oracle: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path(fixture).with_file_name("fixture.json")).unwrap(),
        )
        .unwrap();
        let oracle = &oracle["oracle"];
        let mut wave = ondas::open_with(path(fixture), "wlf-lib").unwrap();
        let hierarchy = wave.hierarchy().clone();
        let mut tables = 0;
        for variable in oracle["hierarchy"]["variables"].as_array().unwrap() {
            let expected = &variable["enumeration"];
            if expected.is_null() {
                continue;
            }
            tables += 1;
            let path = ondas::HierarchyPath::from_components(
                variable["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap()),
            );
            let declaration = hierarchy.variable_path(&path).unwrap();
            let signal = declaration.signal().unwrap();
            let table = declaration.enumeration().unwrap();
            let actual = table
                .variants()
                .map(|v| {
                    assert_eq!(v.encoded.width(), signal.width().unwrap());
                    serde_json::json!({"encoded":v.encoded.to_string(),"label":v.label})
                })
                .collect::<Vec<_>>();
            assert_eq!(serde_json::json!(actual), expected["variants"], "{path}");
            let evidence = &oracle["signals"][variable["signal"].as_str().unwrap()];
            for sample in evidence["samples"].as_array().unwrap() {
                let tick = sample["time"].as_str().unwrap().parse().unwrap();
                let Sample::Value {
                    value: Value::Bits(value),
                    ..
                } = wave.sample(signal, Time::from_ticks(tick)).unwrap()
                else {
                    panic!("expected enum Bits at {path}:{tick}")
                };
                assert_eq!(
                    value.as_ref().to_string(),
                    sample["value"]["bits"].as_str().unwrap(),
                    "{path}:{tick}"
                );
                let expected_labels = expected["variants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|v| v["encoded"] == sample["value"]["bits"])
                    .map(|v| v["label"].as_str().unwrap())
                    .collect::<Vec<_>>();
                let labels = table
                    .variants()
                    .filter(|v| v.encoded.iter_msb().eq(value.as_ref().iter_msb()))
                    .map(|v| v.label)
                    .collect::<Vec<_>>();
                assert_eq!(labels, expected_labels, "{path}:{tick}");
            }
        }
        assert_eq!(tables, if fixture.ends_with("core") { 22 } else { 3 });
    }
}

#[test]
#[ignore = "requires Questa and installed enum fixtures; run just conformance-wlf"]
fn wlf_enum_existing_types_and_owned_metadata() {
    let mut wave = ondas::open_with(path("wlf0057-vsim"), "wlf-lib").unwrap();
    let hierarchy = wave.hierarchy().clone();
    let variable = hierarchy.variable("tb.u1.state").unwrap();
    let signal = variable.signal().unwrap();
    let table = variable.enumeration().unwrap();
    let pairs = table
        .variants()
        .map(|v| (v.encoded.to_string(), v.label))
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [
            ("00".into(), "IDLE"),
            ("01".into(), "S5"),
            ("10".into(), "S10")
        ]
    );
    for (tick, expected) in [(0, "xx"), (10, "00"), (30, "01"), (70, "10")] {
        assert_eq!(
            bits(wave.sample(signal, Time::from_ticks(tick)).unwrap()),
            expected
        );
    }
    drop(wave);
    // A different native owner and resolution must not invalidate the table.
    let mut other = ondas::open_with(path("wlf0070-typed-deltas-aliases"), "wlf-lib").unwrap();
    let h = other.hierarchy().clone();
    let variable = h.variable("top.logic_value").unwrap();
    let logic = variable.signal().unwrap();
    let variants = variable
        .enumeration()
        .unwrap()
        .variants()
        .map(|v| (v.encoded.to_owned(), v.label))
        .collect::<Vec<_>>();
    assert_eq!(variants.len(), 9);
    for (tick, (key, _)) in variants.iter().enumerate() {
        assert_eq!(key.width(), 1);
        assert_eq!(
            bits(other.sample(logic, Time::from_ticks(tick as u64)).unwrap()),
            key.as_ref().to_string()
        );
    }
    assert_eq!(table.variants().nth(1).unwrap().encoded.to_string(), "01");
    assert!(h.variable("top.bus").unwrap().enumeration().is_none());

    let mut ordinal = ondas::open_with(path("wlf0066-wlftb6kaiv"), "wlf-lib").unwrap();
    let h = ordinal.hierarchy().clone();
    let variable = h
        .variable("tb_rgb_system.uut.control_R.current_state")
        .unwrap();
    let signal = variable.signal().unwrap();
    assert_eq!(signal.width(), Some(8));
    let table = variable.enumeration().unwrap();
    let labels = table.variants().map(|v| v.label).collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            "IDLE",
            "first_row0",
            "run",
            "last_row0",
            "last_row1",
            "bef_last",
            "close",
            "finish"
        ]
    );
    for (tick, position) in [
        (0, 0),
        (45000, 1),
        (55000, 2),
        (2625000, 3),
        (2635000, 4),
        (2645000, 5),
        (2655000, 6),
        (2665000, 7),
    ] {
        let key = table.variants().nth(position).unwrap().encoded;
        assert_eq!(
            bits(ordinal.sample(signal, Time::from_ticks(tick)).unwrap()),
            key.to_string()
        );
    }
}

#[test]
#[ignore = "requires Questa and installed enum fixtures; run just conformance-wlf"]
fn wlf_enum_same_resolution_owners_survive_other_close() {
    for _ in 0..8 {
        let mut core = ondas::open_with(path("wlf0083-enum-core"), "wlf-lib").unwrap();
        let core_signal = core.hierarchy().signal("top.sparse8").unwrap();
        let mut xz = ondas::open_with(path("wlf0084-enum-xz"), "wlf-lib").unwrap();
        let xz_signal = xz.hierarchy().signal("top.logic2xz").unwrap();
        assert_eq!(core.metadata().timescale(), xz.metadata().timescale());
        // Opening xz suspends core's native owner. This sample reopens it
        // while xz stays live; closing core must leave xz usable.
        assert_eq!(
            bits(core.sample(core_signal, Time::ZERO).unwrap()),
            "00000001"
        );
        drop(core);
        assert_eq!(bits(xz.sample(xz_signal, Time::ZERO).unwrap()), "00");
    }
}
fn borrowed_bits(sample: ondas::SampleRef<'_>) -> String {
    let ondas::SampleRef::Value {
        value: ondas::ValueRef::Bits(bits),
        ..
    } = sample
    else {
        panic!("expected borrowed bit sample")
    };
    bits.to_string()
}

fn trace_point(trace: &ondas::Trace, time: Time) -> Sample {
    let mut sample = Sample::Missing {
        signal: trace.signal(),
    };
    for (index, change) in trace.changes().iter().enumerate() {
        if change.time() > time {
            break;
        }
        sample = Sample::Value {
            signal: trace.signal(),
            value: change.value().to_owned(),
            changed_at: (index != 0).then_some(change.time()),
        };
    }
    sample
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_positioned_points_and_windows_match_chronological_history() {
    for name in [
        "wlf0070-typed-deltas-aliases",
        "wlf0073-delayed-logging",
        "wlf0074-history-short",
        "wlf0079-wide-compact-toggle",
    ] {
        let mut wave = ondas::open_with(path(name), "wlf-lib").unwrap();
        let mut signals = Vec::new();
        for variable in wave.hierarchy().variables() {
            if let Some(signal) = variable.signal()
                && let ondas::Encoding::Bits { width } = signal.encoding()
            {
                signals.extend([
                    signal,
                    signal.slice(0, 0).unwrap(),
                    signal.slice(width - 1, width - 1).unwrap(),
                    signal,
                ]);
            }
        }
        assert!(!signals.is_empty());
        // A range starting at zero always uses chronological traversal.
        let reference = wave.traces(&signals, TimeRange::all()).unwrap();
        for tick in [
            0,
            4,
            5,
            7,
            8,
            9,
            10,
            11,
            12,
            15,
            16,
            17,
            255,
            256,
            2047,
            2048,
            u64::MAX,
            1,
            12,
            5,
        ] {
            let mut selected = wave.select(&signals).unwrap();
            let time = Time::from_ticks(tick);
            let expected = reference
                .iter()
                .map(|trace| trace_point(trace, time))
                .collect::<Vec<_>>();
            let actual = selected.samples(time).unwrap();
            assert_eq!(
                format!("{actual:?}"),
                format!("{expected:?}"),
                "{name} tick {tick}"
            );
            let mut borrowed = Vec::new();
            let _ = selected
                .visit_samples(time, |sample| {
                    borrowed.push(format!("{sample:?}"));
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert_eq!(
                borrowed,
                expected
                    .iter()
                    .map(|sample| format!("{:?}", sample.as_ref()))
                    .collect::<Vec<_>>()
            );
        }
        {
            let mut selected = wave.select(&signals).unwrap();
            for tick in [12, 15, 15, 1, 5, 7, 8, 9, 10, 11, 12, u64::MAX, 0] {
                let time = Time::from_ticks(tick);
                let expected = reference
                    .iter()
                    .map(|trace| trace_point(trace, time))
                    .collect::<Vec<_>>();
                assert_eq!(
                    format!("{:?}", selected.samples(time).unwrap()),
                    format!("{expected:?}"),
                    "{name} prepared tick {tick}"
                );
            }
        }
        for (start, end) in [(5, 12), (8, 15), (17, 48), (2017, 2048)] {
            let start = Time::from_ticks(start);
            let end = Time::from_ticks(end);
            let actual = wave
                .traces(&signals, TimeRange::closed(start, end))
                .unwrap();
            assert_eq!(actual.len(), reference.len());
            for (expected, actual) in reference.iter().zip(actual) {
                let before = trace_point(expected, Time::from_ticks(start.ticks() - 1));
                match before {
                    Sample::Missing { .. } => assert!(actual.initial().is_none()),
                    Sample::Value {
                        value, changed_at, ..
                    } => {
                        let initial = actual.initial().expect("recorded entering state");
                        assert_eq!(
                            format!("{:?}", initial.value()),
                            format!("{:?}", value.as_ref())
                        );
                        assert_eq!(
                            initial.changed_at(),
                            changed_at,
                            "{name} entering {start:?}"
                        );
                    }
                    _ => unreachable!(),
                }
                let expected = expected
                    .changes()
                    .iter()
                    .filter(|change| change.time() >= start && change.time() <= end)
                    .map(|change| (change.time(), format!("{:?}", change.value())))
                    .collect::<Vec<_>>();
                let actual = actual
                    .changes()
                    .iter()
                    .map(|change| (change.time(), format!("{:?}", change.value())))
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected, "{name} window {start:?}..={end:?}");
            }
        }
    }
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_point_fallback_remaps_reordered_shared_bases() {
    let mut wave = ondas::open(path("wlf0074-history-short")).unwrap();
    let h = wave.hierarchy();
    let clock = h.signal("top.clock").unwrap();
    let words = [
        h.signal("top.word_00").unwrap(),
        h.signal("top.word_01").unwrap(),
        h.signal("top.word_02").unwrap(),
    ];
    let signals = [
        words[1].slice(31, 16).unwrap(),
        clock,
        words[2],
        words[0].slice(31, 16).unwrap(),
        words[1],
        words[2].slice(31, 16).unwrap(),
        words[1].slice(31, 16).unwrap(),
    ];
    let reference = wave.traces(&signals, TimeRange::all()).unwrap();
    for tick in [255, 256, 253, u64::MAX] {
        let time = Time::from_ticks(tick);
        let expected = reference
            .iter()
            .map(|trace| trace_point(trace, time))
            .collect::<Vec<_>>();
        let actual = wave.samples(&signals, time).unwrap();
        assert_eq!(
            format!("{actual:?}"),
            format!("{expected:?}"),
            "tick {tick}"
        );
    }
}

#[test]
#[ignore = "requires Questa and installed one-bit range fixture"]
fn wlf_one_bit_declaration_ranges() {
    let mut wave = ondas::open_with(path("wlf0081-single-bit-ranges"), "wlf-lib").unwrap();
    for (name, range) in [
        ("positive", Some(ondas::BitRange::new(7, 7))),
        ("negative", Some(ondas::BitRange::new(-3, -3))),
        ("scalar", None),
    ] {
        let variable = wave.hierarchy().variable(&format!("top.{name}")).unwrap();
        assert_eq!(variable.range(), range);
        assert_eq!(variable.reader_name(), None);
        let signal = variable.signal().unwrap();
        assert_eq!(signal.width(), Some(1));
        // Projections use normalized offsets, independently of declaration bounds.
        assert_eq!(signal.slice(0, 0).unwrap(), signal);
        for (tick, value) in [(0, "0"), (5, "1"), (10, "0")] {
            assert_eq!(
                bits(wave.sample(signal, Time::from_ticks(tick)).unwrap()),
                value
            );
        }
    }
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_parallel_context_switches() {
    let inputs = [
        ("wlf0081-single-bit-ranges", "top.positive", 5, "0"),
        ("wlf0038-count4b-3-2-vsim", "count4b.clk", 50, "u"),
        ("wlf0074-history-short", "top.clock", 17, "0"),
        ("wlf0070-typed-deltas-aliases", "top.real_value", 15, "0"),
    ];
    let start = Arc::new(std::sync::Barrier::new(inputs.len()));
    std::thread::scope(|threads| {
        for (name, variable, tick, initial) in inputs {
            let start = start.clone();
            threads.spawn(move || {
                start.wait();
                for _ in 0..32 {
                    let path = path(name);
                    let mut wave = ondas::open_with(&path, "wlf-lib").unwrap();
                    let signal = wave.hierarchy().signal(variable).unwrap();
                    for (tick, expected_bit, expected_real) in [
                        (0, initial, 0x8000000000000000_u64),
                        (tick, "1", 0x7ff8000000000002),
                    ] {
                        match wave.sample(signal, Time::from_ticks(tick)).unwrap() {
                            Sample::Value {
                                value: Value::Bits(value),
                                ..
                            } => {
                                assert_eq!(value.as_ref().to_string(), expected_bit);
                            }
                            Sample::Value {
                                value: Value::Real(value),
                                ..
                            } => {
                                assert_eq!(value.to_bits(), expected_real);
                            }
                            _ => panic!("expected recorded bit or real value"),
                        }
                        let metadata = ondas::read_metadata(&path).unwrap();
                        assert_eq!(metadata.timescale(), wave.metadata().timescale());
                    }
                }
            });
        }
    });
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_controlled_benchmark_inputs() {
    for (name, end) in [
        ("wlf0074-history-short", 256),
        ("wlf0075-history-long", 4096),
    ] {
        let mut wave = ondas::open_with(path(name), "wlf-lib").unwrap();
        let clock = wave.hierarchy().signal("top.clock").unwrap();
        let words = (0..64)
            .map(|i| {
                wave.hierarchy()
                    .signal(&format!("top.word_{i:02}"))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            words.iter().collect::<std::collections::HashSet<_>>().len(),
            64
        );
        let time = Time::from_ticks(end);
        let batch = wave
            .samples(&words, time)
            .unwrap()
            .into_iter()
            .map(bits)
            .collect::<Vec<_>>();
        for (i, &signal) in words.iter().enumerate() {
            assert_eq!(batch[i], format!("{:032b}", end / 16 + i as u64));
            assert_eq!(bits(wave.sample(signal, time).unwrap()), batch[i]);
        }
        let mut selected = wave.select(&words).unwrap();
        assert_eq!(
            selected
                .samples(time)
                .unwrap()
                .into_iter()
                .map(bits)
                .collect::<Vec<_>>(),
            batch
        );
        let mut borrowed = Vec::new();
        let _ = selected
            .visit_samples(time, |sample| {
                borrowed.push(borrowed_bits(sample));
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(borrowed, batch);
        for (start, stop) in [(17, 48), (end - 31, end)] {
            let window = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(stop));
            let mut selected = wave.select(&[clock]).unwrap();
            assert_eq!(selected.traces(window).unwrap()[0].changes().len(), 32);
            assert_eq!(
                selected
                    .scan(window, |row| match row {
                        ondas::ScanRef::Change { time, .. } => ControlFlow::Break(time),
                        _ => ControlFlow::Continue(()),
                    })
                    .unwrap(),
                ControlFlow::Break(Time::from_ticks(start))
            );
        }
    }
    for name in [
        "wlf0076-topology-small",
        "wlf0077-topology-many-handles",
        "wlf0078-topology-many-times",
        "wlf0080-topology-many-aliases",
    ] {
        let mut wave = ondas::open_with(path(name), "wlf-lib").unwrap();
        let sparse = wave.hierarchy().signal("top.probe_sparse").unwrap();
        let constant = wave.hierarchy().signal("top.probe_constant").unwrap();
        for (start, end, count) in [(17, 49, 0), (4031, 4063, 1)] {
            let window = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));
            let traces = wave.traces(&[sparse, constant], window).unwrap();
            assert_eq!(traces[0].changes().len(), count);
            assert!(traces[1].changes().is_empty());
        }
        if name == "wlf0080-topology-many-aliases" {
            assert_eq!(wave.hierarchy().aliases(sparse).unwrap().count(), 257);
            let aliases = (0..256)
                .map(|i| {
                    wave.hierarchy()
                        .signal(&format!("top.alias_{i:03}"))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert!(aliases.iter().all(|&signal| signal == sparse));
            assert!(
                wave.samples(&aliases, Time::from_ticks(4096))
                    .unwrap()
                    .into_iter()
                    .all(|sample| bits(sample) == "0")
            );
        }
    }
    let mut wave = ondas::open_with(path("wlf0079-wide-compact-toggle"), "wlf-lib").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let scalar = wave.hierarchy().signal("top.control").unwrap();
    assert_eq!(wide.width(), Some(4096));
    let low = wide.slice(0, 0).unwrap();
    let high = wide.slice(4095, 1).unwrap();
    let inputs = [wide, low, high, scalar, low, wide];
    let mut selected = wave.select(&inputs).unwrap();
    for tick in [0, 1, 2047, 2048, 1, 2047] {
        let time = Time::from_ticks(tick);
        let samples = selected
            .samples(time)
            .unwrap()
            .into_iter()
            .map(bits)
            .collect::<Vec<_>>();
        assert_eq!(samples[0], "0".repeat(4095) + &samples[1]);
        assert_eq!(samples[1], (tick % 2).to_string());
        assert_eq!(samples[2], "0".repeat(4095));
        assert_eq!(samples[1], samples[3]);
        assert_eq!(samples[1], samples[4]);
        assert_eq!(samples[0], samples[5]);
        let mut borrowed = Vec::new();
        let _ = selected
            .visit_samples(time, |sample| {
                borrowed.push(borrowed_bits(sample));
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(borrowed, samples);
    }
    let window = TimeRange::closed(Time::from_ticks(2017), Time::from_ticks(2048));
    let traces = selected.traces(window).unwrap();
    assert!(traces[2].changes().is_empty());
    for index in [0, 1, 3, 4, 5] {
        assert_eq!(traces[index].changes().len(), 32);
    }
}
#[test]
fn wlf_bytes_are_rejected_before_native_open() {
    assert!(matches!(
        ondas::open_bytes_with("anything", Arc::from(&b""[..]), "wlf-lib"),
        Err(Error::UnsupportedInput { .. })
    ));
    assert!(matches!(
        ondas::open_bytes("dump.WLF", Arc::from(&b""[..])),
        Err(Error::UnsupportedInput { .. })
    ));
    assert!(matches!(
        ondas::open_with("missing.wlf", "unknown"),
        Err(Error::UnknownBackend { .. })
    ));
    assert!(matches!(ondas::open("missing.wlf"), Err(Error::Io(_))));
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires both SDKs and installed fixtures"]
fn wlf_fsdb_interleaved_visitors() {
    let mut wlf = ondas::open(counter()).unwrap();
    let wlf_clock = wlf.hierarchy().signal("count4b.clk").unwrap();
    let fsdb_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/fsdb/fsdb0005-compare-xz/waveform.fsdb");
    let mut fsdb = ondas::open(&fsdb_path).unwrap();
    let fsdb_clock = fsdb.hierarchy().signal("compare_xz_tb.clk").unwrap();
    let result = wlf
        .scan(
            &[wlf_clock],
            TimeRange::closed(Time::ZERO, Time::from_ticks(100)),
            |_| {
                assert_eq!(
                    bits(fsdb.sample(fsdb_clock, Time::from_ticks(5)).unwrap()),
                    "1"
                );
                assert_eq!(
                    bits(fsdb.sample(fsdb_clock, Time::from_ticks(10)).unwrap()),
                    "0"
                );
                assert_eq!(
                    ondas::read_metadata(&fsdb_path).unwrap().time_span(),
                    fsdb.metadata().time_span()
                );
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
    let result = fsdb
        .scan(
            &[fsdb_clock],
            TimeRange::closed(Time::ZERO, Time::from_ticks(10)),
            |_| {
                assert_eq!(
                    bits(wlf.sample(wlf_clock, Time::from_ticks(50)).unwrap()),
                    "1"
                );
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
}

#[test]
#[ignore = "requires Questa and installed fixtures"]
fn wlf_lifecycle_and_reentrant_visitors() {
    let mut wave = ondas::open_with(counter(), "wlf-lib").unwrap();
    let clock = wave.hierarchy().signal("count4b.clk").unwrap();
    let mut other = ondas::open(counter()).unwrap();
    let other_clock = other.hierarchy().signal("count4b.clk").unwrap();
    let caller = std::thread::current().id();
    let rows = std::rc::Rc::new(std::cell::Cell::new(0));
    let outcome = wave.scan(
        &[clock, clock],
        TimeRange::closed(Time::ZERO, Time::from_ticks(100)),
        |row| {
            assert_eq!(std::thread::current().id(), caller);
            let (time, value) = match row {
                ondas::ScanRef::Change { time, value, .. } => (time, value),
                ondas::ScanRef::Initial { value, .. } => (Time::ZERO, value),
                _ => panic!("expected recorded value"),
            };
            let ondas::ValueRef::Bits(value) = value else {
                panic!("expected bits")
            };
            assert_eq!(
                bits(other.sample(other_clock, time).unwrap()),
                value.to_string()
            );
            let mut nested = ondas::open(counter()).unwrap();
            let reset = nested.hierarchy().signal("count4b.reset").unwrap();
            assert_eq!(bits(nested.sample(reset, Time::ZERO).unwrap()), "1");
            rows.set(rows.get() + 1);
            ControlFlow::Break(17)
        },
    );
    assert_eq!(outcome.unwrap(), ControlFlow::Break(17));
    assert_eq!(rows.get(), 1);
    assert_eq!(
        bits(wave.sample(clock, Time::from_ticks(100)).unwrap()),
        "0"
    );
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = wave.scan(
            &[clock],
            TimeRange::closed(Time::ZERO, Time::from_ticks(100)),
            |_| -> ControlFlow<()> {
                panic!("caller panic");
            },
        );
    }));
    assert!(panic.is_err());
    assert_eq!(bits(wave.sample(clock, Time::from_ticks(50)).unwrap()), "1");
    std::thread::spawn(move || {
        assert_eq!(bits(wave.sample(clock, Time::ZERO).unwrap()), "u");
        let mut third = ondas::open(counter()).unwrap();
        let signal = third.hierarchy().signal("count4b.reset").unwrap();
        assert_eq!(bits(third.sample(signal, Time::ZERO).unwrap()), "1");
    })
    .join()
    .unwrap();
    assert_eq!(
        bits(other.sample(other_clock, Time::from_ticks(100)).unwrap()),
        "0"
    );
}
#[test]
#[ignore = "requires Questa and installed fixtures"]
fn wlf_metadata_matches_open_and_empty_database_bounds() {
    for name in ["wlf0038-count4b-3-2-vsim", "wlf0036-comparator-vsim"] {
        let path = path(name);
        let metadata = ondas::read_metadata(&path).unwrap();
        let wave = ondas::open(path).unwrap();
        assert_eq!(metadata.time_span(), wave.metadata().time_span());
        assert_eq!(metadata.timescale(), wave.metadata().timescale());
        assert_eq!(metadata.writer(), wave.metadata().writer());
        assert_eq!(wave.format(), Format::Wlf);
    }
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_exact_binary64_payloads() {
    let mut wave = ondas::open(path("wlf0070-typed-deltas-aliases")).unwrap();
    let real = wave.hierarchy().signal("top.real_value").unwrap();
    for (time, expected) in [
        (0, 0x8000000000000000_u64),
        (7, 0xc00e000000000000),
        (11, 0),
        (12, 0x7ff0000000000000),
        (13, 0xfff0000000000000),
        (14, 0x7ff8000000000001),
        (15, 0x7ff8000000000002),
    ] {
        let Sample::Value {
            value: Value::Real(value),
            ..
        } = wave.sample(real, Time::from_ticks(time)).unwrap()
        else {
            panic!("expected real")
        };
        assert_eq!(value.to_bits(), expected);
    }
    let trace = wave
        .trace(
            real,
            TimeRange::closed(Time::from_ticks(14), Time::from_ticks(15)),
        )
        .unwrap();
    assert_eq!(
        trace
            .changes()
            .iter()
            .map(|change| change.time().ticks())
            .collect::<Vec<_>>(),
        [14, 15]
    );
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_mixed_resolution_mid_batch_events() {
    let mut events = ondas::open(path("wlf0071-event-occurrences")).unwrap();
    let pulse = events.hierarchy().signal("top.pulse").unwrap();
    let count = events.hierarchy().signal("top.count").unwrap();
    let mut typed = ondas::open(path("wlf0070-typed-deltas-aliases")).unwrap();
    let real = typed.hierarchy().signal("top.real_value").unwrap();
    let mut pulses = Vec::new();
    let mut counts = Vec::new();
    let mut calls = 0;
    let outcome = events
        .scan(
            &[pulse, count],
            TimeRange::closed(Time::ZERO, Time::from_ticks(6)),
            |row| {
                let ondas::ScanRef::Change {
                    signal,
                    time,
                    value,
                } = row
                else {
                    panic!("unexpected entering event state")
                };
                let time = time.ticks();
                if signal == pulse {
                    let ondas::ValueRef::Event { occurrences } = value else {
                        panic!("expected event")
                    };
                    pulses.push((time, occurrences));
                } else {
                    let ondas::ValueRef::Bits(bits) = value else {
                        panic!("expected count")
                    };
                    counts.push((time, u64::from_str_radix(&bits.to_string(), 2).unwrap()));
                }
                // Force a 1 ns / 10 ps switch after each delivered batch slot.
                let tick = 14 + calls % 2;
                let Sample::Value {
                    value: Value::Real(real),
                    ..
                } = typed.sample(real, Time::from_ticks(tick)).unwrap()
                else {
                    panic!("expected real")
                };
                assert_eq!(real.to_bits(), 0x7ff8000000000001 + calls % 2);
                calls += 1;
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(outcome, ControlFlow::Continue(()));
    assert_eq!(pulses, [(0, 2), (3, 3), (5, 1), (6, 1)]);
    assert_eq!(counts, [(0, 2), (3, 5), (5, 6), (6, 7)]);
    assert_eq!(calls, 8);
    // Metadata-only reading can suspend an active query's native context too.
    assert_eq!(
        ondas::read_metadata(counter())
            .unwrap()
            .timescale()
            .unwrap()
            .factor(),
        1
    );
    let Sample::Value {
        value: Value::Real(real),
        ..
    } = typed.sample(real, Time::from_ticks(15)).unwrap()
    else {
        panic!("expected real")
    };
    assert_eq!(real.to_bits(), 0x7ff8000000000002);
}

#[test]
#[ignore = "requires Questa and installed fixtures; run just conformance-wlf"]
fn wlf_multi_batch_resolution_switches_preserve_every_tick() {
    let mut history = ondas::open(path("wlf0075-history-long")).unwrap();
    let clock = history.hierarchy().signal("top.clock").unwrap();
    let mut typed = ondas::open(path("wlf0070-typed-deltas-aliases")).unwrap();
    let real = typed.hierarchy().signal("top.real_value").unwrap();
    let mut changes = Vec::new();
    let caller = std::thread::current().id();
    let _ = history
        .scan(
            &[clock],
            TimeRange::closed(Time::from_ticks(17), Time::from_ticks(600)),
            |row| {
                assert_eq!(std::thread::current().id(), caller);
                if let ondas::ScanRef::Change {
                    time,
                    value: ondas::ValueRef::Bits(value),
                    ..
                } = row
                {
                    changes.push((time.ticks(), value.to_string()));
                }
                let Sample::Value {
                    value: Value::Real(value),
                    ..
                } = typed.sample(real, Time::from_ticks(14)).unwrap()
                else {
                    panic!("expected recorded real");
                };
                assert_eq!(value.to_bits(), 0x7ff8000000000001);
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    let expected = (17..=600)
        .map(|tick| (tick, (tick % 2).to_string()))
        .collect::<Vec<_>>();
    assert_eq!(changes, expected);
}
