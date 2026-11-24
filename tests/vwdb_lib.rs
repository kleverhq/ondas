#![cfg(feature = "vwdb-lib")]

use std::{ops::ControlFlow, panic::AssertUnwindSafe, path::PathBuf};

use ondas::{
    Error, Format, HierarchyPath, InputKind, PathComponent, SampleRef, Time, TimeRange, ValueRef,
};
#[cfg(feature = "shm-lib")]
use std::fs;

#[path = "support/indexed_paths.rs"]
mod indexed_paths;

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(path)
}

fn gap_wave() -> ondas::Waveform {
    ondas::open(fixture("vwdb/vwdb0023-probe-gap/waveform.vwdb")).unwrap()
}

fn bits(sample: SampleRef<'_>) -> Option<String> {
    match sample {
        SampleRef::Value {
            value: ValueRef::Bits(value),
            ..
        } => Some(value.to_string()),
        SampleRef::Missing { .. } => None,
        unexpected => panic!("expected bits or Missing, got {unexpected:?}"),
    }
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn directory_formats_and_pure_vwdb() {
    let pure = fixture("vwdb/vwdb0046-opb-s/waveform.vwdb");
    let hybrid = fixture("vwdb/vwdb0023-probe-gap/waveform.vwdb");
    assert!(!pure.join("waves_ida.shm").exists());
    assert!(hybrid.join("waves_ida.shm").is_dir());
    let mut wave = ondas::open(&pure).unwrap();
    assert_eq!(wave.format(), Format::Vwdb);
    assert_eq!(wave.backend(), "vwdb-lib");
    let marker = wave.hierarchy().signal("tb_opb.end_marker.q").unwrap();
    let sample = wave.sample(marker, Time::from_ticks(100_000_000)).unwrap();
    assert_eq!(bits(sample.as_ref()).as_deref(), Some("1"));
    let metadata = ondas::read_metadata(&pure).unwrap();
    assert_eq!(metadata.timescale(), wave.metadata().timescale());
    assert_eq!(metadata.time_span(), wave.metadata().time_span());
    assert_eq!(ondas::open(&hybrid).unwrap().format(), Format::Vwdb);
    let bytes = b"".as_slice().into();
    assert!(matches!(
        ondas::open_bytes_with("input", bytes, "vwdb-lib"),
        Err(Error::UnsupportedInput {
            input: InputKind::Bytes,
            ..
        })
    ));
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn vhdl_ranges_are_declaration_indices() {
    let wave = ondas::open(fixture(
        "vwdb/vwdb0003-fsdb-nine-state-ranges/waveform.vwdb",
    ))
    .unwrap();
    for (name, msb, lsb) in [("ascending", 3, 11), ("descending", 12, 4)] {
        let variable = wave.hierarchy().variable(&format!(r#""".{name}"#)).unwrap();
        assert_eq!(variable.range(), Some(ondas::BitRange::new(msb, lsb)));
    }
    let wave = ondas::open(fixture("vwdb/vwdb0011-ghw-types/waveform.vwdb")).unwrap();
    for (name, width) in [("i32", 32), ("i64", 32), ("p32", 64), ("p64", 64)] {
        let variable = wave.hierarchy().variable(&format!(r#""".{name}"#)).unwrap();
        assert_eq!(variable.range(), None);
        assert_eq!(variable.signal().unwrap().width(), Some(width));
    }
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn vhdl_enum_reads_recorded_codes_and_labels() {
    let mut wave = ondas::open(fixture("vwdb/vwdb0036-wdb-vhdl-enum/waveform.vwdb")).unwrap();
    let variable = wave.hierarchy().variable("\"\".phase").unwrap();
    let signal = variable.signal().unwrap();
    assert_eq!(signal.width(), Some(8));
    let table = variable.enumeration().unwrap();
    let variants = table
        .variants()
        .map(|variant| (variant.encoded.to_string(), variant.label))
        .collect::<Vec<_>>();
    assert_eq!(
        variants,
        [
            ("00000000".into(), "idle"),
            ("00000001".into(), "busy"),
            ("00000010".into(), "done"),
        ]
    );
    for (tick, expected) in [
        (0, "00000000"),
        (4_000_000, "00000001"),
        (9_000_000, "00000010"),
        (15_000_000, "00000000"),
    ] {
        let sample = wave.sample(signal, Time::from_ticks(tick)).unwrap();
        assert_eq!(bits(sample.as_ref()).as_deref(), Some(expected));
    }
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn exact_paths_and_independent_handles() {
    for (slug, cases) in [
        (
            "0017-logical-names",
            &[("top", false, "0"), ("top", true, "1")][..],
        ),
        ("0018-logical-names-indexed", &[("top", false, "0")][..]),
        ("0019-logical-names-literal", &[("top", true, "1")][..]),
        (
            "0020-logical-names-separated",
            &[("top.u_indexed", false, "0"), ("top.u_literal", true, "1")][..],
        ),
    ] {
        let path = fixture(&format!("vwdb/vwdb{slug}/waveform.vwdb"));
        let mut wave = ondas::open(path).unwrap();
        for &(parent, literal, value) in cases {
            indexed_paths::check_q(&mut wave, parent, literal, false, value);
        }
    }
    let vhdl = ondas::open(fixture(
        "vwdb/vwdb0037-wdb-vhdl-numeric-arrays/waveform.vwdb",
    ))
    .unwrap();
    let path = HierarchyPath::from_components([
        PathComponent::new(""),
        PathComponent::indexed("integers", 1),
    ]);
    assert_eq!(HierarchyPath::parse(&path.to_string()).unwrap(), path);
    assert!(vhdl.hierarchy().signal_path(&path).is_ok());
    let nested = ondas::open(fixture(
        "vwdb/vwdb0088-tb-complex-types-complete/waveform.vwdb",
    ))
    .unwrap();
    let path = HierarchyPath::from_components([
        PathComponent::new("tb_complex_types"),
        PathComponent::indexed("unpacked_2d1", 1),
        PathComponent::new("2"),
    ]);
    assert!(nested.hierarchy().signal_path(&path).is_ok());
    let wave = gap_wave();
    let h = wave.hierarchy();
    let signals = [
        "top.inst.in_bus",
        "top.alias_value",
        "top.ascending",
        "top.equal_value",
    ]
    .map(|path| h.signal(path).unwrap());
    assert_eq!(
        signals
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        signals.len()
    );
    assert!(signals.iter().all(|&s| h.aliases(s).unwrap().count() == 1));
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn exact_real_bits() {
    let mut wave = ondas::open(fixture("vwdb/vwdb0005-fsdb-typed-values/waveform.vwdb")).unwrap();
    let signals = ["values.r", "values.sr"].map(|path| wave.hierarchy().signal(path).unwrap());
    let times = [9, 17].map(Time::from_ticks);
    let samples = wave.samples_at(&signals, &times).unwrap();
    for ((_, row), expected) in samples.iter().zip([
        [0x8000_0000_0000_0000, 0x8000_0000_0000_0000],
        [0x7ff8_0000_0000_0001, 0x7ff8_0000_2000_0000],
    ]) {
        for (actual, bits) in row.iter().zip(expected) {
            let SampleRef::Value {
                value: ValueRef::Real(value),
                ..
            } = actual.as_ref()
            else {
                panic!("expected the recorded real value");
            };
            assert_eq!(value.to_bits(), bits);
        }
    }
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn gap_point_batch_query_and_scan() {
    let mut wave = gap_wave();
    let glitch = wave.hierarchy().signal("top.glitch").unwrap();
    let bus = wave.hierarchy().signal("top.ascending").unwrap();
    let selected = [glitch, glitch];
    let times = [1, 2, 3, 4].map(Time::from_ticks);
    for (time, expected) in times.into_iter().zip([Some("0"), None, None, Some("0")]) {
        assert_eq!(
            bits(wave.sample(glitch, time).unwrap().as_ref()).as_deref(),
            expected
        );
    }
    let observed = wave.samples_at(&selected, &times).unwrap();
    for ((_, samples), expected) in observed.iter().zip([Some("0"), None, None, Some("0")]) {
        assert!(
            samples
                .iter()
                .all(|sample| bits(sample.as_ref()).as_deref() == expected)
        );
    }
    let trace = wave
        .trace(glitch, TimeRange::closed(times[3], Time::from_ticks(8)))
        .unwrap();
    assert!(trace.initial().is_none());
    assert!(matches!(
        trace.sample(times[3]).unwrap(),
        SampleRef::Value {
            changed_at: None,
            ..
        }
    ));
    let crosses_gap = TimeRange::closed(times[0], times[3]);
    assert!(matches!(
        wave.trace(glitch, crosses_gap),
        Err(Error::UnsupportedFeature {
            format: Format::Vwdb,
            operation: "trace",
            ..
        })
    ));
    let mut selection = wave.select(&selected).unwrap();
    assert_eq!(
        selection
            .scan(crosses_gap, |_| ControlFlow::Break("initial"))
            .unwrap(),
        ControlFlow::Break("initial")
    );
    let mut candidates = Vec::new();
    let _ = selection
        .scan_candidate_times(crosses_gap, |time| {
            candidates.push(time);
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(candidates, [times[1], times[3]]);
    assert_eq!(
        selection
            .scan_candidate_times(crosses_gap, ControlFlow::Break)
            .unwrap(),
        ControlFlow::Break(times[1])
    );
    let mut at_loss = false;
    let _ = selection
        .query(crosses_gap, &[0], |ctx| {
            if ctx.time() == times[1] {
                at_loss = true;
                let _ = ctx.visit_samples(ctx.time(), &[0, 1], |_, value| {
                    assert!(matches!(value, SampleRef::Missing { .. }));
                    Ok(ControlFlow::<()>::Continue(()))
                })?;
            }
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert!(at_loss);

    let bit = bus.slice(0, 0).unwrap();
    assert!(matches!(
        wave.sample(bit, times[1]).unwrap().as_ref(),
        SampleRef::Missing { .. }
    ));
    let concat = wave.concat(&[bus, glitch]).unwrap();
    assert!(matches!(
        wave.sample(concat, times[1]).unwrap().as_ref(),
        SampleRef::Missing { .. }
    ));
    let mut selection = wave.select(&[glitch]).unwrap();
    let unwind = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = selection.scan(TimeRange::point(Time::ZERO), |_| -> ControlFlow<()> {
            panic!("stop from callback");
        });
    }));
    assert!(unwind.is_err());
    assert_eq!(
        bits(selection.samples(Time::ZERO).unwrap()[0].as_ref()).as_deref(),
        Some("0")
    );
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires installed SHM and VWDB fixtures"]
fn coexist_in_both_orders_and_resume() {
    let shm = fixture("shm/shm0088-time-scale/waveform.shm");
    let vwdb = fixture("vwdb/vwdb0023-probe-gap/waveform.vwdb");
    let mut a = ondas::open_with(&shm, "shm-lib").unwrap();
    let mut b = ondas::open_with(&vwdb, "vwdb-lib").unwrap();
    let ash = a.hierarchy().signal("top.value").unwrap();
    let bvw = b.hierarchy().signal("top.inst.in_bus").unwrap();
    use ondas::ScanRef;
    let mut observed = Vec::new();
    let _ = a
        .select(&[ash])
        .unwrap()
        .scan(
            TimeRange::closed(Time::ZERO, Time::from_ticks(300)),
            |record| {
                if let ScanRef::Change {
                    time,
                    value: ValueRef::Bits(value),
                    ..
                } = record
                {
                    observed.push((time.ticks(), value.to_string()));
                    assert_eq!(
                        bits(b.sample(bvw, Time::from_ticks(4)).unwrap().as_ref()).as_deref(),
                        Some("000000101")
                    );
                }
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(
        observed,
        [(0, "0".into()), (100, "1".into()), (200, "0".into())]
    );

    drop((a, b));
    let mut b = ondas::open(&vwdb).unwrap();
    let mut a = ondas::open(&shm).unwrap();
    let ash = a.hierarchy().signal("top.value").unwrap();
    let bvw = b.hierarchy().signal("top.inst.in_bus").unwrap();
    let mut observed = Vec::new();
    let _ = b
        .select(&[bvw])
        .unwrap()
        .scan(
            TimeRange::closed(Time::from_ticks(4), Time::from_ticks(8)),
            |record| {
                if let ScanRef::Change {
                    time,
                    value: ValueRef::Bits(value),
                    ..
                } = record
                {
                    observed.push((time.ticks(), value.to_string()));
                    let nested_vwdb = gap_wave();
                    assert_eq!(nested_vwdb.format(), Format::Vwdb);
                    assert_eq!(
                        bits(a.sample(ash, Time::from_ticks(100)).unwrap().as_ref()).as_deref(),
                        Some("1")
                    );
                }
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(
        observed,
        [
            (4, "000000101".into()),
            (5, "111111111".into()),
            (6, "000000000".into())
        ]
    );
    assert_eq!(
        bits(b.sample(bvw, Time::from_ticks(5)).unwrap().as_ref()).as_deref(),
        Some("111111111")
    );
    assert_ne!(a.metadata().timescale(), b.metadata().timescale());

    // Cross the first batch boundary after suspending the native stream.
    for (path, backend) in [
        ("shm/shm0081-history-long/waveform.shm", "shm-lib"),
        ("vwdb/vwdb0013-history-long/waveform.vwdb", "vwdb-lib"),
    ] {
        let mut wave = ondas::open_with(fixture(path), backend).unwrap();
        let phase = wave.hierarchy().signal("top.phase_0").unwrap();
        let mut seen = 0;
        let _ = wave
            .select(&[phase])
            .unwrap()
            .scan(
                TimeRange::closed(Time::ZERO, Time::from_ticks(128)),
                |record| {
                    let ScanRef::Change {
                        time,
                        value: ValueRef::Bits(value),
                        ..
                    } = record
                    else {
                        panic!("expected a recorded phase change");
                    };
                    assert_eq!(time.ticks(), seen);
                    assert_eq!(value.to_string(), (seen % 2).to_string());
                    if seen == 0 || seen == 64 {
                        assert_eq!(
                            bits(a.sample(ash, Time::from_ticks(100)).unwrap().as_ref()).as_deref(),
                            Some("1")
                        );
                        assert_eq!(
                            bits(b.sample(bvw, Time::from_ticks(4)).unwrap().as_ref()).as_deref(),
                            Some("000000101")
                        );
                    }
                    seen += 1;
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
        assert_eq!(seen, 129);
    }

    let from = fs::canonicalize(&vwdb).unwrap();
    let directory = std::env::temp_dir().join(format!("ondas-vwdb-routing-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let renamed_vwdb = directory.join("recording.shm");
    let renamed_shm = directory.join("recording.vwdb");
    std::os::unix::fs::symlink(from, &renamed_vwdb).unwrap();
    std::os::unix::fs::symlink(shm, &renamed_shm).unwrap();
    assert_eq!(ondas::open(&renamed_vwdb).unwrap().format(), Format::Vwdb);
    assert_eq!(ondas::open(&renamed_shm).unwrap().format(), Format::Shm);
    assert!(matches!(
        ondas::open_with(&renamed_vwdb, "shm-lib"),
        Err(Error::BackendDoesNotSupport {
            format: Format::Vwdb,
            ..
        })
    ));
    assert!(matches!(
        ondas::open_with(&renamed_shm, "vwdb-lib"),
        Err(Error::BackendDoesNotSupport {
            format: Format::Shm,
            ..
        })
    ));
    let linked_components = directory.join("linked.vwdb");
    fs::create_dir(&linked_components).unwrap();
    for entry in fs::read_dir(renamed_shm).unwrap() {
        let entry = entry.unwrap();
        std::os::unix::fs::symlink(entry.path(), linked_components.join(entry.file_name()))
            .unwrap();
    }
    assert_eq!(
        ondas::open(&linked_components).unwrap().format(),
        Format::Shm
    );
    let named = ondas::open_with(&linked_components, "shm-lib").unwrap();
    assert_eq!(named.format(), Format::Shm);
    assert_eq!(
        ondas::read_metadata(&linked_components)
            .unwrap()
            .timescale(),
        named.metadata().timescale()
    );
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn waveform_owners_move_across_threads() {
    let mut short = gap_wave();
    let mut long = ondas::open(fixture("vwdb/vwdb0013-history-long/waveform.vwdb")).unwrap();
    let a = short.hierarchy().signal("top.glitch").unwrap();
    let b = long.hierarchy().signal("top.phase_0").unwrap();
    let ready = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let ready = &ready;
        let gap_reader = scope.spawn(move || {
            ready.wait();
            for _ in 0..8 {
                assert!(matches!(
                    short.sample(a, Time::from_ticks(2)).unwrap().as_ref(),
                    SampleRef::Missing { .. }
                ));
                assert_eq!(
                    bits(short.sample(a, Time::from_ticks(4)).unwrap().as_ref()).as_deref(),
                    Some("0")
                );
            }
        });
        let history_reader = scope.spawn(move || {
            ready.wait();
            for tick in 128..144 {
                assert_eq!(
                    bits(long.sample(b, Time::from_ticks(tick)).unwrap().as_ref()).unwrap(),
                    (tick % 2).to_string(),
                );
            }
        });
        gap_reader.join().unwrap();
        history_reader.join().unwrap();
    });
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires installed SHM and VWDB fixtures"]
fn reader_orders_in_fresh_processes() {
    const ORDER: &str = "ONDAS_TEST_VWDB_FIRST_READER";
    let Ok(first) = std::env::var(ORDER) else {
        for backend in ["shm-lib", "vwdb-lib"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", "reader_orders_in_fresh_processes"])
                .env(ORDER, backend)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{backend} first: {stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    };
    let shm =
        || ondas::open_with(fixture("shm/shm0088-time-scale/waveform.shm"), "shm-lib").unwrap();
    let vwdb = || gap_wave();
    let (mut a, mut b) = match first.as_str() {
        "shm-lib" => (shm(), vwdb()),
        "vwdb-lib" => {
            let b = vwdb();
            (shm(), b)
        }
        _ => panic!("unexpected first reader {first}"),
    };
    let signal_a = a.hierarchy().signal("top.value").unwrap();
    let signal_b = b.hierarchy().signal("top.glitch").unwrap();
    for _ in 0..2 {
        assert_eq!(
            bits(a.sample(signal_a, Time::from_ticks(100)).unwrap().as_ref()).as_deref(),
            Some("1")
        );
        assert_eq!(
            bits(b.sample(signal_b, Time::from_ticks(4)).unwrap().as_ref()).as_deref(),
            Some("0")
        );
    }
}

#[test]
#[ignore = "requires installed VWDB fixtures"]
fn window_resumes_after_session_eviction() {
    let mut wave = ondas::open(fixture("vwdb/vwdb0013-history-long/waveform.vwdb")).unwrap();
    let phase = wave.hierarchy().signal("top.phase_0").unwrap();
    let mut observed = Vec::new();
    let mut initial = None;
    let range = TimeRange::closed(Time::from_ticks(17), Time::from_ticks(128));
    let _ = wave
        .select(&[phase])
        .unwrap()
        .scan(range, |record| {
            match record {
                ondas::ScanRef::Initial {
                    value: ValueRef::Bits(bits),
                    changed_at,
                    ..
                } => initial = Some((bits.to_string(), changed_at)),
                ondas::ScanRef::Change {
                    time,
                    value: ValueRef::Bits(bits),
                    ..
                } => {
                    observed.push((time.ticks(), bits.to_string()));
                    if matches!(time.ticks(), 17 | 81) {
                        let mut other = gap_wave();
                        let glitch = other.hierarchy().signal("top.glitch").unwrap();
                        assert!(matches!(
                            other.sample(glitch, Time::from_ticks(2)).unwrap().as_ref(),
                            SampleRef::Missing { .. }
                        ));
                    }
                }
                _ => panic!("expected a phase bit"),
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(initial, Some(("0".to_owned(), Some(Time::from_ticks(16)))));
    assert_eq!(
        observed,
        (17..=128)
            .map(|tick| (tick, (tick % 2).to_string()))
            .collect::<Vec<_>>()
    );
}
