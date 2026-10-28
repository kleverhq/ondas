//! Directed checks for the optional native reader.
#![cfg(feature = "wdb-lib")]
use ondas::{Error, Format, Sample, Time, TimeRange, Value};
use std::{ops::ControlFlow, path::PathBuf, sync::Arc};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/wdb/wdb0305-enum-core/waveform.wdb")
}
fn bits(sample: Sample) -> String {
    let Sample::Value {
        value: Value::Bits(bits),
        ..
    } = sample
    else {
        panic!("expected recorded bits")
    };
    bits.as_ref().to_string()
}

#[test]
fn wdb_automatic_and_explicit_backends_reject_bytes() {
    let bytes: Arc<[u8]> = b"Xilinx WAVE DATABASE ".as_slice().into();
    for result in [
        ondas::open_bytes("recording.wdb", bytes.clone()),
        ondas::open_bytes_with("input", bytes, "wdb-lib"),
    ] {
        assert!(
            matches!(result, Err(Error::UnsupportedInput { backend, input: ondas::InputKind::Bytes }) if backend == "wdb-lib")
        );
    }
}

#[test]
fn wdb_invalid_files_return_errors_before_native_entry() {
    let root = std::env::temp_dir().join(format!(
        "ondas-wdb-invalid-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for (name, bytes) in [
        ("empty.wdb", Vec::new()),
        ("truncated.wdb", b"Xilinx WAVE DATABASE 01\0".to_vec()),
        ("invalid.wdb", vec![b'!'; 256]),
    ] {
        let path = root.join(name);
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            ondas::open(&path),
            Err(Error::Malformed {
                format: Format::Wdb,
                ..
            })
        ));
        assert!(matches!(
            ondas::read_metadata(&path),
            Err(Error::Malformed {
                format: Format::Wdb,
                ..
            })
        ));
    }
    let path = root.join("future.wdb");
    let mut bytes = vec![0; 256];
    bytes[..24].copy_from_slice(b"Xilinx WAVE DATABASE 99\0");
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        ondas::open(&path),
        Err(Error::UnsupportedFeature {
            format: Format::Wdb,
            ..
        })
    ));
    assert!(matches!(
        ondas::read_metadata(&path),
        Err(Error::UnsupportedFeature {
            format: Format::Wdb,
            ..
        })
    ));
}

#[test]
#[ignore = "requires the selected WDB reader and installed enum fixture"]
fn wdb_lifecycle_stop_panic_and_reentrant_visitors() {
    let mut wave = ondas::open_with(fixture(), "wdb-lib").unwrap();
    let signal = wave.hierarchy().signal("top.sparse8").unwrap();
    let mut other = ondas::open(fixture()).unwrap();
    let other_signal = other.hierarchy().signal("top.sparse8").unwrap();
    assert_eq!(wave.format(), Format::Wdb);
    assert_eq!(wave.backend(), "wdb-lib");
    let metadata = ondas::read_metadata(fixture()).unwrap();
    assert_eq!(wave.metadata().timescale(), metadata.timescale());
    assert_eq!(wave.metadata().time_span(), metadata.time_span());
    assert_eq!(wave.metadata().source_name(), metadata.source_name());
    let caller = std::thread::current().id();
    let rows = std::rc::Rc::new(std::cell::Cell::new(0));
    let result = wave
        .scan(
            &[signal, signal],
            TimeRange::closed(Time::ZERO, Time::from_ticks(100)),
            |row| {
                assert_eq!(std::thread::current().id(), caller);
                let ondas::ScanRef::Change { time, value, .. } = row else {
                    panic!("expected an in-range change")
                };
                assert_eq!(time, Time::ZERO);
                let ondas::ValueRef::Bits(value) = value else {
                    panic!("expected bit value")
                };
                assert_eq!(value.to_string(), "00000001");
                assert_eq!(
                    bits(other.sample(other_signal, time).unwrap()),
                    value.to_string()
                );
                let mut nested = ondas::open(fixture()).unwrap();
                let nested_signal = nested.hierarchy().signal("top.array_values[0]").unwrap();
                assert_eq!(
                    bits(nested.sample(nested_signal, Time::from_ticks(9)).unwrap()),
                    "00101010"
                );
                rows.set(rows.get() + 1);
                ControlFlow::Break(17)
            },
        )
        .unwrap();
    assert_eq!(result, ControlFlow::Break(17));
    assert_eq!(rows.get(), 1);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = wave.scan(&[signal], TimeRange::all(), |_| -> ControlFlow<()> {
            panic!("caller panic");
        });
    }));
    assert!(panic.is_err());
    assert_eq!(
        bits(wave.sample(signal, Time::from_ticks(64)).unwrap()),
        "00010010"
    );
    let owned = wave.hierarchy().clone();
    let table = owned
        .variable("top.sparse8")
        .unwrap()
        .enumeration()
        .unwrap();
    std::thread::spawn(move || {
        assert_eq!(
            bits(wave.sample(signal, Time::from_ticks(9)).unwrap()),
            "00101010"
        );
        let mut third = ondas::open(fixture()).unwrap();
        let signal = third.hierarchy().signal("top.sparse8").unwrap();
        assert_eq!(
            bits(third.sample(signal, Time::from_ticks(97)).unwrap()),
            "00000111"
        );
    })
    .join()
    .unwrap();
    assert_eq!(
        table.variants().next().unwrap().encoded.to_string(),
        "00000001"
    );
    assert_eq!(
        bits(other.sample(other_signal, Time::from_ticks(97)).unwrap()),
        "00000111"
    );
}

#[test]
#[ignore = "requires the selected WDB reader and installed enum fixture"]
fn wdb_point_projections_keep_normalized_times_after_seek_and_stop() {
    let mut wave = ondas::open_with(fixture(), "wdb-lib").unwrap();
    let base = wave.hierarchy().signal("top.sparse8").unwrap();
    let bit = base.slice(1, 1).unwrap();
    let constant = base.slice(7, 6).unwrap();
    let t = Time::from_ticks;
    for (time, full, changed, projected, projected_changed) in [
        (97, "00000111", Some(97), "1", Some(64)),
        (9, "00101010", Some(9), "1", Some(4)),
        (0, "00000001", None, "0", None),
        (64, "00010010", Some(64), "1", Some(64)),
        (15, "00000001", Some(15), "0", Some(15)),
        (4, "00000111", Some(4), "1", Some(4)),
    ] {
        let samples = wave.samples(&[base, bit, constant, base], t(time)).unwrap();
        for (sample, signal, expected, changed) in [
            (&samples[0], base, full, changed),
            (&samples[1], bit, projected, projected_changed),
            (&samples[2], constant, "00", None),
            (&samples[3], base, full, changed),
        ] {
            let Sample::Value {
                signal: actual,
                value: Value::Bits(value),
                changed_at,
            } = sample
            else {
                panic!("expected recorded bits");
            };
            assert_eq!(*actual, signal);
            assert_eq!(value.as_ref().to_string(), expected);
            assert_eq!(*changed_at, changed.map(t));
        }
    }
    let mut selection = wave.select(&[base, bit, constant]).unwrap();
    selection.samples(t(9)).unwrap();
    let traces = selection.traces(TimeRange::closed(t(10), t(64))).unwrap();
    let initial = traces[1].initial().unwrap();
    let text = |value| match value {
        ondas::ValueRef::Bits(bits) => bits.to_string(),
        _ => panic!("expected bit value"),
    };
    assert_eq!(text(initial.value()), "1");
    assert_eq!(initial.changed_at(), Some(t(4)));
    assert_eq!(
        traces[1]
            .changes()
            .iter()
            .map(|change| (change.time(), text(change.value())))
            .collect::<Vec<_>>(),
        [(t(15), "0".into()), (t(64), "1".into())]
    );
    assert!(traces[2].changes().is_empty());
    assert_eq!(traces[2].initial().unwrap().changed_at(), None);
    selection.samples(t(9)).unwrap();
    assert_eq!(
        selection
            .scan(TimeRange::closed(t(10), t(100)), |record| match record {
                ondas::ScanRef::Change { time, .. } => ControlFlow::Break(time),
                _ => ControlFlow::Continue(()),
            })
            .unwrap(),
        ControlFlow::Break(t(15))
    );
    let resumed = selection.samples(t(97)).unwrap();
    let Sample::Value { changed_at, .. } = &resumed[1] else {
        panic!("expected recorded bits");
    };
    assert_eq!(*changed_at, Some(t(64)));
}

#[test]
#[ignore = "requires the selected WDB reader and installed enum fixture"]
fn wdb_repeated_source_points_seed_ranges_without_changing_projection_times() {
    let mut wave = ondas::open_with(fixture(), "wdb-lib").unwrap();
    let base = wave.hierarchy().signal("top.sparse8").unwrap();
    let bus = wave.concat(&[base, base, base]).unwrap();
    let cut = bus.slice(15, 8).unwrap();
    let nested = wave.concat(&[cut, base]).unwrap();
    let signals = [bus, cut, nested, bus];
    let t = Time::from_ticks;
    let mut selection = wave.select(&signals).unwrap();
    for (time, expected, changed) in [
        (97, "00000111", Some(97)),
        (9, "00101010", Some(9)),
        (0, "00000001", None),
        (64, "00010010", Some(64)),
        (15, "00000001", Some(15)),
        (4, "00000111", Some(4)),
        (100, "00000111", Some(97)),
        (u64::MAX, "00000111", Some(97)),
    ] {
        let samples = selection.samples(t(time)).unwrap();
        assert_eq!(samples.len(), signals.len());
        for ((sample, signal), copies) in samples.iter().zip(signals).zip([3, 1, 2, 3]) {
            let Sample::Value {
                signal: actual,
                value: Value::Bits(value),
                changed_at,
            } = sample
            else {
                panic!("expected recorded bits");
            };
            assert_eq!(*actual, signal);
            assert_eq!(value.as_ref().to_string(), expected.repeat(copies));
            assert_eq!(*changed_at, changed.map(t));
        }
    }
    let text = |value| match value {
        ondas::ValueRef::Bits(bits) => bits.to_string(),
        _ => panic!("expected bit value"),
    };
    let traces = selection.traces(TimeRange::closed(t(10), t(64))).unwrap();
    assert_eq!(traces.len(), signals.len());
    for (trace, copies) in traces.iter().zip([3, 1, 2, 3]) {
        let initial = trace.initial().unwrap();
        assert_eq!(text(initial.value()), "00101010".repeat(copies));
        assert_eq!(initial.changed_at(), Some(t(9)));
        assert_eq!(
            trace
                .changes()
                .iter()
                .map(|change| (change.time(), text(change.value())))
                .collect::<Vec<_>>(),
            [
                (t(15), "00000001".repeat(copies)),
                (t(64), "00010010".repeat(copies)),
            ]
        );
    }
    assert_eq!(
        selection
            .scan(TimeRange::closed(t(10), t(100)), |record| match record {
                ondas::ScanRef::Change { time, .. } => ControlFlow::Break(time),
                _ => ControlFlow::Continue(()),
            })
            .unwrap(),
        ControlFlow::Break(t(15))
    );
    let resumed = selection.samples(t(97)).unwrap();
    assert_eq!(bits(resumed[2].clone()), "00000111".repeat(2));
    let Sample::Value { changed_at, .. } = &resumed[2] else {
        panic!("expected recorded bits");
    };
    assert_eq!(*changed_at, Some(t(97)));
    drop(selection);

    // The intermediate bit loses changes, even when the output is a full byte.
    let bit = bus.slice(1, 1).unwrap();
    let rebuilt = wave.concat(&[bit; 8]).unwrap();
    let Sample::Value {
        value: Value::Bits(value),
        changed_at,
        ..
    } = wave.sample(rebuilt, t(9)).unwrap()
    else {
        panic!("expected recorded bits");
    };
    assert_eq!(value.as_ref().to_string(), "11111111");
    assert_eq!(changed_at, Some(t(4)));
}

#[test]
#[ignore = "requires the selected WDB reader and installed enum fixture"]
fn wdb_independent_owners_on_parallel_threads() {
    let workers = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..4 {
                    let mut wave = ondas::open(fixture()).unwrap();
                    let signal = wave.hierarchy().signal("top.sparse8").unwrap();
                    for (tick, expected) in [(97, "00000111"), (0, "00000001"), (9, "00101010")] {
                        assert_eq!(
                            bits(wave.sample(signal, Time::from_ticks(tick)).unwrap()),
                            expected
                        );
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
#[ignore = "requires source-backed local controls in ONDAS_WDB_CONTROLS"]
fn wdb_native_unavailable_values_fail_without_losing_other_histories() {
    let root = PathBuf::from(
        std::env::var_os("ONDAS_WDB_CONTROLS").expect("ONDAS_WDB_CONTROLS is required"),
    );
    let mut wave = ondas::open(root.join("wdb/string-event/waveform.wdb")).unwrap();
    let clock = wave.hierarchy().signal("top.clock").unwrap();
    for path in ["top.text", "top.pulse"] {
        let signal = wave.hierarchy().signal(path).unwrap();
        assert_eq!(signal.encoding(), ondas::Encoding::Unsupported);
        assert!(matches!(
            wave.sample(signal, Time::from_ticks(4)),
            Err(Error::UnsupportedSignal { .. })
        ));
        assert_eq!(bits(wave.sample(clock, Time::from_ticks(4)).unwrap()), "1");
    }
    let mut wave = ondas::open(root.join("wdb/metadata/waveform.wdb")).unwrap();
    let hierarchy = wave.hierarchy().clone();
    assert_eq!(
        hierarchy
            .variable("top.dut.binary_bus")
            .unwrap()
            .logic_domain(),
        Some(ondas::LogicDomain::TwoState)
    );
    assert_eq!(
        hierarchy
            .variable("top.dut.signed_bus")
            .unwrap()
            .logic_domain(),
        Some(ondas::LogicDomain::FourState)
    );
    let member = hierarchy.variable("top.dut.pair.hi").unwrap();
    assert_eq!(member.reader_name(), Some(".hi"));
    assert_eq!(
        bits(
            wave.sample(member.signal().unwrap(), Time::from_ticks(4))
                .unwrap()
        ),
        "0011"
    );
    let wave = ondas::open(root.join("wdb/vhdl-enum/waveform.wdb")).unwrap();
    let character = wave
        .hierarchy()
        .variable("std_logic_1164.NBSP")
        .unwrap()
        .enumeration()
        .unwrap();
    assert_eq!(character.variants().count(), 256);
    let nbsp = character.variants().nth(160).unwrap();
    assert_eq!(nbsp.encoded.to_string(), "10100000");
    assert_eq!(nbsp.label, "\u{a0}");
    let wave = ondas::open(root.join("wdb/packed-shapes/waveform.wdb")).unwrap();
    for name in ["top.packed_enum[0]", "top.packed_enum[1]"] {
        let variable = wave.hierarchy().variable(name).unwrap();
        assert_eq!(variable.kind(), "enum");
        let variants = variable
            .enumeration()
            .unwrap()
            .variants()
            .map(|variant| (variant.encoded.to_string(), variant.label.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            variants,
            [
                ("0001".into(), "A".into()),
                ("0100".into(), "B".into()),
                ("1001".into(), "C".into())
            ]
        );
    }
}

#[test]
#[ignore = "requires source-backed local controls in ONDAS_WDB_CONTROLS"]
fn wdb_strings_names_and_decode_error_recovery() {
    let root = PathBuf::from(
        std::env::var_os("ONDAS_WDB_CONTROLS").expect("ONDAS_WDB_CONTROLS is required"),
    );
    let mut wave = ondas::open(root.join("wdb/vhdl-strings-ports/waveform.wdb")).unwrap();
    let hierarchy = wave.hierarchy().clone();
    for (name, reader_name, expected) in [
        ("message.with.dot", "\\message.with.dot\\", "more"),
        ("slash\\name", "\\slash\\\\name\\", "path"),
        ("with_nul", "with_nul", "A\0B"),
        ("reverse_message", "reverse_message", "last"),
    ] {
        let path = ondas::HierarchyPath::from_components(["top", name]);
        let variable = hierarchy.variable_path(&path).unwrap();
        assert_eq!(variable.name(), name);
        assert_eq!(variable.reader_name(), Some(reader_name));
        assert!(!variable.name_was_escaped());
        assert_eq!(variable.kind(), "string");
        let signal = variable.signal().unwrap();
        assert_eq!(signal.encoding(), ondas::Encoding::String);
        let Sample::Value { value, .. } = wave.sample(signal, Time::from_ticks(4000)).unwrap()
        else {
            panic!("expected string value")
        };
        assert!(matches!(value, Value::String(value) if value.as_ref() == expected));
    }
    let bad = hierarchy.signal("top.non_ascii").unwrap();
    for _ in 0..2 {
        assert!(matches!(
            wave.trace(bad, TimeRange::all()),
            Err(Error::NonAsciiString { format: Format::Wdb, time, .. }) if time == Time::ZERO
        ));
        let good = hierarchy.signal("top.message").unwrap();
        assert!(matches!(
            wave.sample(good, Time::from_ticks(9000)).unwrap(),
            Sample::Value { value: Value::String(value), .. } if value.as_ref() == "done"
        ));
    }
}

#[test]
#[ignore = "requires source-backed local controls in ONDAS_WDB_CONTROLS"]
fn wdb_compositions_preserve_missing_and_validate_foreign_handles() {
    let root = PathBuf::from(
        std::env::var_os("ONDAS_WDB_CONTROLS").expect("ONDAS_WDB_CONTROLS is required"),
    );
    let path = root.join("wdb/partial-logging/waveform.wdb");
    let mut wave = ondas::open(&path).unwrap();
    let recorded = wave.hierarchy().signal("top.recorded").unwrap();
    let hidden = wave.hierarchy().signal("top.hidden").unwrap();
    let repeated_hidden = wave.concat(&[hidden, hidden, hidden]).unwrap();
    for tick in [9, 0, 3] {
        assert!(matches!(
            wave.sample(repeated_hidden, Time::from_ticks(tick))
                .unwrap(),
            Sample::Missing { .. }
        ));
    }
    let missing = wave
        .trace(
            repeated_hidden,
            TimeRange::closed(Time::from_ticks(3), Time::from_ticks(9)),
        )
        .unwrap();
    assert!(missing.initial().is_none());
    assert!(missing.changes().is_empty());
    let bus = wave.concat(&[recorded, hidden]).unwrap();
    let nested = wave.concat(&[bus.slice(0, 0).unwrap(), recorded]).unwrap();
    assert_eq!(bus, wave.concat(&[recorded, hidden]).unwrap());
    for (tick, expected, expected_nested) in [(9, "1x", "x1"), (0, "0x", "x0"), (3, "0x", "x0")] {
        let samples = wave
            .samples(&[bus, nested, bus, hidden], Time::from_ticks(tick))
            .unwrap();
        assert_eq!(bits(samples[0].clone()), expected);
        assert_eq!(bits(samples[1].clone()), expected_nested);
        assert_eq!(bits(samples[2].clone()), expected);
        assert!(matches!(samples[3], Sample::Missing { .. }));
    }
    let trace = wave.trace(bus, TimeRange::all()).unwrap();
    assert_eq!(trace.changes().len(), 2);
    assert_eq!(trace.changes()[0].time(), Time::ZERO);
    assert_eq!(trace.changes()[1].time(), Time::from_ticks(4));
    let mut selection = wave.select(&[nested, bus, nested]).unwrap();
    let stopped = selection
        .query(TimeRange::all(), &[0, 1], |context| {
            assert_eq!(context.time(), Time::ZERO);
            Ok(ControlFlow::Break(23))
        })
        .unwrap();
    assert_eq!(stopped, ControlFlow::Break(23));
    assert_eq!(
        bits(selection.samples(Time::from_ticks(9)).unwrap().remove(0)),
        "x1"
    );
    let mut other = ondas::open(&path).unwrap();
    let foreign = other.hierarchy().signal("top.recorded").unwrap();
    assert!(matches!(
        wave.sample(foreign, Time::ZERO),
        Err(Error::InvalidSignal { .. })
    ));
    assert!(matches!(
        other.sample(bus, Time::ZERO),
        Err(Error::InvalidSignal { .. })
    ));
}

#[test]
#[ignore = "requires source-backed local controls in ONDAS_WDB_CONTROLS"]
fn wdb_vhdl_primitive_types_preserve_metadata_and_values() {
    let root = PathBuf::from(
        std::env::var_os("ONDAS_WDB_CONTROLS").expect("ONDAS_WDB_CONTROLS is required"),
    );
    let mut wave = ondas::open(root.join("wdb/vhdl-bit-arrays/waveform.wdb")).unwrap();
    let hierarchy = wave.hierarchy().clone();
    for name in ["top.vector_value", "top.bit_value", "top.binary_matrix"] {
        assert_eq!(
            hierarchy.variable(name).unwrap().logic_domain(),
            Some(ondas::LogicDomain::TwoState)
        );
    }
    for name in ["top.GEN_VALUE", "top.GEN_BITS"] {
        assert!(hierarchy.variable(name).unwrap().is_constant());
    }
    assert_eq!(
        hierarchy.variable("top.matrix").unwrap().logic_domain(),
        Some(ondas::LogicDomain::NineState)
    );
    assert_eq!(
        hierarchy
            .variable("top.integer_value")
            .unwrap()
            .signedness(),
        None
    );
    assert_eq!(hierarchy.variable("top.real_value").unwrap().kind(), "real");
    let integer = hierarchy.signal("top.integer_value").unwrap();
    assert_eq!(
        bits(wave.sample(integer, Time::ZERO).unwrap()),
        "11111111111111111111111111101111"
    );
    assert_eq!(
        bits(wave.sample(integer, Time::from_ticks(4000)).unwrap()),
        "00000000000000000000000000010111"
    );
    for name in ["top.flags[1]", "top.flags[2]"] {
        let variants = hierarchy
            .variable(name)
            .unwrap()
            .enumeration()
            .unwrap()
            .variants()
            .map(|variant| (variant.encoded.to_string(), variant.label.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            variants,
            [
                ("00000000".into(), "FALSE".into()),
                ("00000001".into(), "TRUE".into())
            ]
        );
    }
    for name in ["top.states[1]", "top.states[2]"] {
        let variants = hierarchy
            .variable(name)
            .unwrap()
            .enumeration()
            .unwrap()
            .variants()
            .map(|variant| (variant.encoded.to_string(), variant.label.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            variants,
            [
                ("00000000".into(), "RED".into()),
                ("00000001".into(), "GREEN".into()),
                ("00000010".into(), "BLUE".into())
            ]
        );
    }
}
