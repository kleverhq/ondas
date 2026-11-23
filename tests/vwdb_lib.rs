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
            operation: "scan",
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
}
