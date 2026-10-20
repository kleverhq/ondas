use ondas::{
    Encoding, Error, Format, Sample, SampleRef, ScanRef, Time, TimeRange, Value, ValueRef,
};
use std::ops::ControlFlow;

fn sample_bits(sample: Sample) -> String {
    match sample {
        Sample::Value {
            value: Value::Bits(bits),
            ..
        } => bits.as_ref().to_string(),
        other => panic!("{other:?}"),
    }
}

#[path = "support/ghw_bytes.rs"]
mod ghw_bytes;
use ghw_bytes::{padded_recording, recording};

fn enum_recording() -> Vec<u8> {
    let strings = [
        "state", "red", "grün", "blue", "white", "black", "subset", "top", "mode", "alias",
    ];
    let mut bytes = b"GHDLwave\n\x10\0\x01\x01\x04\x01\0STR\0\0\0\0\0".to_vec();
    bytes.extend((strings.len() as u32).to_le_bytes());
    bytes.extend((strings.iter().map(|s| s.chars().count()).sum::<usize>() as u32).to_le_bytes());
    for string in strings {
        bytes.extend(string.chars().map(|c| c as u8));
        bytes.push(0);
    }
    bytes.extend(b"EOS\0TYP\0\0\0\0\0\x02\0\0\0");
    // Five base ordinals; subtype retains ordinals 1 through 3.
    bytes.extend([23, 1, 5, 2, 3, 4, 5, 6, 34, 7, 1, 23, 1, 3, 0]);
    bytes.extend(b"HIE\0\0\0\0\0");
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend([3, 8, 16, 9, 2, 1, 16, 10, 2, 1, 15, 0]);
    bytes.extend(b"EOH\0SNP\0\0\0\0\0");
    bytes.extend(0i64.to_le_bytes());
    bytes.push(1); // grün.
    bytes.extend(b"ESN\0CYC\0");
    bytes.extend(2i64.to_le_bytes());
    bytes.extend([1, 2, 0, 3, 1, 3, 0, 127]); // blue at 2, white at 5.
    bytes.extend(b"ECY\0");
    bytes
}

#[test]
fn a_common_consumer_matches_enum_labels_to_whole_values() {
    fn labels<'h>(variable: &ondas::Variable<'h>, bits: ondas::BitsRef<'_>) -> Vec<&'h str> {
        let key = bits.to_string();
        variable
            .enumeration()
            .map(|table| {
                table
                    .variants()
                    .filter(|v| v.encoded == key)
                    .map(|v| v.label)
                    .collect()
            })
            .unwrap_or_default()
    }
    let mut wave = ondas::open_bytes("enum.ghw", enum_recording().into()).unwrap();
    let hierarchy = wave.hierarchy().clone();
    let variable = hierarchy.variable("top.mode").unwrap();
    let signal = variable.signal().unwrap();
    assert_eq!(signal.width(), Some(3));
    assert_eq!(variable.range(), None);
    assert_eq!(variable.signedness(), None);
    assert_eq!(variable.logic_domain(), None);
    assert_eq!(signal, hierarchy.signal("top.alias").unwrap());
    assert_eq!(wave.metadata().timescale(), None);
    for (tick, key, label) in [(0, "001", "grün"), (2, "010", "blue"), (5, "011", "white")] {
        let Sample::Value {
            value: Value::Bits(bits),
            ..
        } = wave.sample(signal, Time::from_ticks(tick)).unwrap()
        else {
            panic!("expected enum bits")
        };
        assert_eq!(bits.as_ref().to_string(), key);
        assert_eq!(labels(&variable, bits.as_ref()), [label]);
    }
}

#[test]
fn public_opening_aliases_slices_batches_and_replay() {
    for explicit in [false, true] {
        let data = recording(0).into();
        let mut wave = if explicit {
            ondas::open_bytes_with("input.GHW", data, "ghw-native")
        } else {
            ondas::open_bytes("input.bin", data)
        }
        .unwrap();
        assert_eq!(wave.format(), Format::Ghw);
        assert_eq!(wave.backend(), "ghw-native");
        assert_eq!(wave.metadata().timescale(), None);
        assert_eq!(wave.metadata().time_span().unwrap().last().ticks(), 4);
        let wide = wave.hierarchy().signal("top.wide").unwrap();
        assert_eq!(wide, wave.hierarchy().signal("top.alias").unwrap());
        let reversed = wave.hierarchy().signal("top.reversed").unwrap();
        assert_ne!(wide, reversed);
        assert_eq!(wide.encoding(), Encoding::Bits { width: 3 });
        let low = wide.slice(0, 0).unwrap();
        let high = wide.slice(2, 2).unwrap();
        let handles = [wide, low, high, wide, reversed];
        let mut selection = wave.select(&handles).unwrap();
        for tick in [0, 1, 2, 3, 4, 100, 3, 0, 2, 2, 4] {
            let values = selection
                .samples(Time::from_ticks(tick))
                .unwrap()
                .into_iter()
                .map(sample_bits)
                .collect::<Vec<_>>();
            let whole = match tick {
                0 => "100",
                1 | 2 => "101",
                3 => "111",
                _ => "110",
            };
            assert_eq!(
                values,
                [
                    whole,
                    &whole[2..],
                    "1",
                    whole,
                    &whole.chars().rev().collect::<String>()
                ]
            );
        }
        let traces = selection.traces(TimeRange::all()).unwrap();
        assert_eq!(
            traces[0]
                .changes()
                .iter()
                .map(|c| c.time().ticks())
                .collect::<Vec<_>>(),
            [0, 1, 3, 4]
        );
        assert_eq!(traces[2].changes().len(), 1);
        let low_state = selection.samples(Time::from_ticks(3)).unwrap();
        assert!(matches!(low_state[1], Sample::Value { changed_at: Some(t), .. } if t.ticks()==1));
        assert_eq!(
            selection.samples(Time::from_ticks(4)).unwrap().len(),
            handles.len()
        );
    }
}

#[test]
fn early_break_before_unrelated_suffix_and_queries_after_stop() {
    let mut wave = ondas::open_bytes("early.ghw", recording(10000).into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let result = wave
        .scan(&[wide], TimeRange::all(), |record| {
            if let ScanRef::Change { time, .. } = record
                && time.ticks() == 1
            {
                ControlFlow::Break(41)
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Break(41));
    assert_eq!(
        sample_bits(wave.sample(wide, Time::from_ticks(10000)).unwrap()),
        "101"
    );
    assert_eq!(
        sample_bits(wave.sample(wide, Time::from_ticks(0)).unwrap()),
        "100"
    );
}

#[test]
fn composed_reads_see_entering_and_final_state() {
    let mut wave = ondas::open_bytes("composed.ghw", recording(0).into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let control = wave.hierarchy().signal("top.control").unwrap();
    let mut selection = wave.select(&[control, wide]).unwrap();
    let mut rows = Vec::new();
    let result = selection
        .query(TimeRange::all(), &[0], |context| {
            let time = context.time();
            let now = context_bits(context, time, 1)?;
            let before = if time.ticks() == 0 {
                None
            } else {
                Some(context_bits(
                    context,
                    Time::from_ticks(time.ticks() - 1),
                    1,
                )?)
            };
            rows.push((time.ticks(), now, before));
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert!(result.is_continue());
    assert_eq!(
        rows,
        [
            (0, "100".into(), None),
            (1, "101".into(), Some("100".into())),
            (4, "110".into(), Some("111".into()))
        ]
    );
}

#[test]
fn malformed_unselected_values_and_unsupported_layouts_fail_opening() {
    let mut bytes = recording(2);
    // Last noise list: time delta 1, active-rank delta 4, ordinal, list end 0.
    let noise = bytes
        .windows(4)
        .rposition(|list| list[0] == 1 && list[1] == 4 && list[3] == 0)
        .unwrap();
    bytes[noise + 2] = 255;
    assert!(matches!(
        ondas::open_bytes("bad.ghw", bytes.into()),
        Err(Error::Malformed {
            format: Format::Ghw,
            ..
        })
    ));
    let mut bytes = recording(0);
    const VERSION_BYTE: usize = 11; // After magic, header length and reserved byte.
    bytes[VERSION_BYTE] = 2;
    assert!(matches!(
        ondas::open_bytes("future.ghw", bytes.into()),
        Err(Error::Backend {
            operation: "open",
            ..
        })
    ));
    let compressed = vec![0x1f, 0x8b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(matches!(
        ondas::open_bytes_with("compressed.ghw", compressed.into(), "ghw-native"),
        Err(Error::Backend {
            operation: "open",
            ..
        })
    ));
    assert!(matches!(
        ondas::open_bytes_with(
            "text.vcd",
            b"$enddefinitions $end".as_slice().into(),
            "ghw-native"
        ),
        Err(Error::BackendDoesNotSupport {
            format: Format::Vcd,
            ..
        })
    ));
}

#[test]
fn file_and_metadata_opening_match_bytes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join(format!("ghw-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("recording.ghw");
    std::fs::write(&path, recording(0)).unwrap();
    let mut wave = ondas::open_with(&path, "ghw-native").unwrap();
    let metadata = ondas::read_metadata(&path).unwrap();
    assert_eq!(metadata.time_span(), wave.metadata().time_span());
    assert_eq!(metadata.timescale(), wave.metadata().timescale());
    let signal = wave.hierarchy().signal("top.wide").unwrap();
    assert_eq!(
        sample_bits(wave.sample(signal, Time::from_ticks(3)).unwrap()),
        "111"
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn partial_cycle_values_wait_for_every_recipe_position() {
    let mut bytes = padded_recording(0);
    // Use shared source IDs inside each three-bit recipe, including its reverse.
    for (declaration, ids) in [
        ([16, 14, 4, 3, 1, 4], [3, 3, 4]),
        ([16, 15, 4, 3, 1, 4], [3, 3, 4]),
        ([16, 16, 4, 4, 1, 3], [4, 3, 3]),
    ] {
        let offset = bytes
            .windows(declaration.len())
            .position(|entry| entry == declaration)
            .unwrap();
        bytes[offset + 3..offset + 6].copy_from_slice(&ids);
    }
    let body = bytes.windows(4).position(|tag| tag == b"SNP\0").unwrap();
    bytes.truncate(body);
    bytes.extend(b"CYC\0");
    bytes.extend(0i64.to_le_bytes());
    // Active IDs start 3, 4, 5, followed by unused padding sources.
    // ID 3 changes at ticks 0 and 1; ID 4 starts at 2.
    bytes.extend([1, 3, 0, 1, 1, 2, 0, 1, 2, 3, 0, 127]);
    bytes.extend(b"ECY\0");
    let mut wave = ondas::open_bytes("partial.ghw", bytes.into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let reversed = wave.hierarchy().signal("top.reversed").unwrap();
    for tick in [0, 1, 2, 1, 0, 2] {
        let samples = wave
            .samples(&[wide, reversed], Time::from_ticks(tick))
            .unwrap();
        if tick < 2 {
            assert!(samples.iter().all(|s| matches!(s, Sample::Missing { .. })));
        } else {
            assert_eq!(
                samples.into_iter().map(sample_bits).collect::<Vec<_>>(),
                ["001", "100"]
            );
        }
    }
}

#[test]
fn empty_delayed_and_equal_timestamp_sections_preserve_recorded_state() {
    let original = recording(0);
    let body_start = original.windows(4).position(|b| b == b"SNP\0").unwrap();
    let mut empty = ondas::open_bytes("empty.ghw", original[..body_start].into()).unwrap();
    let wide = empty.hierarchy().signal("top.wide").unwrap();
    assert!(empty.metadata().time_span().is_none());
    assert!(matches!(
        empty.sample(wide, Time::from_ticks(100)).unwrap(),
        Sample::Missing { .. }
    ));
    assert!(
        empty
            .trace(wide, TimeRange::all())
            .unwrap()
            .changes()
            .is_empty()
    );
    let mut delayed = original.clone();
    // SNP: marker + four reserved bytes + absolute i64 time.
    delayed[body_start + 8..body_start + 16].copy_from_slice(&5i64.to_le_bytes());
    let cycle = delayed.windows(4).position(|b| b == b"CYC\0").unwrap();
    // CYC: marker + absolute i64 time.
    delayed[cycle + 4..cycle + 12].copy_from_slice(&6i64.to_le_bytes());
    let mut wave = ondas::open_bytes("delayed.ghw", delayed.into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    assert!(matches!(
        wave.sample(wide, Time::from_ticks(4)).unwrap(),
        Sample::Missing { .. }
    ));
    assert_eq!(
        sample_bits(wave.sample(wide, Time::from_ticks(5)).unwrap()),
        "100"
    );
    assert_eq!(
        sample_bits(wave.sample(wide, Time::from_ticks(6)).unwrap()),
        "101"
    );
    let mut combined = original[..body_start].to_vec();
    combined.extend(b"SNP\0\0\0\0\0");
    combined.extend(0i64.to_le_bytes());
    // Active source IDs 1,3,4,5: ordinal 3 = '1', ordinal 2 = '0'.
    combined.extend([3, 2, 3, 2]);
    combined.extend(b"ESN\0CYC\0");
    combined.extend(0i64.to_le_bytes());
    // Active-rank delta 2 changes source ID 3 to '1'; 0 ends the list, -1 the cycle.
    combined.extend([2, 3, 0, 127]);
    combined.extend(b"ECY\0");
    combined.extend(&original[body_start..]);
    let mut wave = ondas::open_bytes("sections.ghw", combined.into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    assert_eq!(
        sample_bits(wave.sample(wide, Time::from_ticks(0)).unwrap()),
        "100"
    );
    let trace = wave.trace(wide, TimeRange::all()).unwrap();
    assert_eq!(
        trace
            .changes()
            .iter()
            .map(|c| c.time().ticks())
            .collect::<Vec<_>>(),
        [0, 1, 3, 4]
    );
}

fn context_bits(
    context: &ondas::QueryContext<'_>,
    time: Time,
    slot: usize,
) -> ondas::Result<String> {
    let mut result = None;
    let _ = context.visit_samples(time, &[slot], |_, sample| {
        if let SampleRef::Value {
            value: ValueRef::Bits(bits),
            ..
        } = sample
        {
            result = Some(bits.to_string());
        }
        Ok(ControlFlow::<()>::Continue(()))
    })?;
    Ok(result.expect("recorded bit value"))
}

#[test]
fn both_versions_and_byte_orders_preserve_binary64_payloads() {
    for big in [false, true] {
        for (version, payload) in [
            (0, 0x7ff8000000001234_u64),
            (1, 0x7ff8000000001234),
            (0, f64::INFINITY.to_bits()),
            (1, f64::NEG_INFINITY.to_bits()),
        ] {
            let mut wave = ondas::open_bytes(
                "real.ghw",
                ghw_bytes::real_recording(big, version, payload).into(),
            )
            .unwrap();
            let r = wave.hierarchy().signal("top.r").unwrap();
            assert_eq!(r.encoding(), Encoding::Real);
            for (time, bits) in [(3, 0x8000000000000000u64), (4, payload)] {
                let value = wave.sample(r, Time::from_ticks(time)).unwrap();
                assert!(
                    matches!(value,Sample::Value { value: Value::Real(real),.. } if real.to_bits()==bits)
                );
            }
            assert!(matches!(
                wave.sample(r, Time::from_ticks(2)).unwrap(),
                Sample::Missing { .. }
            ));
        }
    }
}
