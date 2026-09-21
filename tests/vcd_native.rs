use ondas::{
    Encoding, Error, Format, HierarchyPath, Sample, ScanRef, Time, TimeRange, Value, ValueRef,
};
use std::ops::ControlFlow;

fn source(declarations: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("$timescale 1ns $end $scope module top $end {declarations} $upscope $end $enddefinitions $end ").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn open(declarations: &str, body: &[u8]) -> ondas::Waveform {
    ondas::open_bytes("test.vcd", source(declarations, body).into())
        .unwrap_or_else(|e| panic!("{e}"))
}

fn value(sample: Sample) -> Value {
    match sample {
        Sample::Value { value, .. } => value,
        other => panic!("{other:?}"),
    }
}

fn bits(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Bits(bits) => bits.to_string(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn reused_selection_matches_fresh_replay() {
    let mut wave = open(
        "$var wire 4 ! bus $end $var event 1 e ev $end $var real 64 r real $end $var string 1 s text $end $var wire 1 m missing $end",
        b"#0 b0 ! #3 b11 ! b0 ! 1e 1e r1 r sfirst s #4 1e #10 $dumpoff bx ! 1e $end #12 $dumpon b1 ! 1e $end 1e #12 b10 ! #20 r-0 r slast s #100 b11 !",
    );
    let h = wave.hierarchy();
    let bus = h.signal("top.bus").unwrap();
    let signals = [
        bus,
        bus.slice(1, 0).unwrap(),
        h.signal("top.ev").unwrap(),
        h.signal("top.real").unwrap(),
        h.signal("top.text").unwrap(),
        h.signal("top.missing").unwrap(),
        bus,
    ];
    let ticks = [
        0, 3, 3, 4, 5, 10, 12, 12, 13, 20, 100, 101, 1000, 4, 3, 0, 12,
    ];
    let expected: Vec<_> = ticks
        .iter()
        .map(|&t| wave.samples(&signals, Time::from_ticks(t)).unwrap())
        .collect();
    let mut selection = wave.select(&signals).unwrap();
    for (&tick, expected) in ticks.iter().zip(&expected) {
        assert_eq!(
            format!("{:?}", selection.samples(Time::from_ticks(tick)).unwrap()),
            format!("{expected:?}"),
            "tick {tick}"
        );
    }
    drop(selection);
    for start in [0, 3, 4, 5, 10, 12, 13, 20, 100, 101] {
        let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(start + 3));
        let expected = wave.select(&signals).unwrap().traces(range).unwrap();
        let mut selection = wave.select(&signals).unwrap();
        selection.samples(Time::from_ticks(start)).unwrap();
        for _ in 0..2 {
            for (actual, expected) in selection.traces(range).unwrap().iter().zip(&expected) {
                assert_eq!(
                    format!(
                        "{:?}",
                        actual.initial().map(|v| (v.value(), v.changed_at()))
                    ),
                    format!(
                        "{:?}",
                        expected.initial().map(|v| (v.value(), v.changed_at()))
                    )
                );
                assert_eq!(
                    format!(
                        "{:?}",
                        actual
                            .changes()
                            .iter()
                            .map(|v| (v.time(), v.value()))
                            .collect::<Vec<_>>()
                    ),
                    format!(
                        "{:?}",
                        expected
                            .changes()
                            .iter()
                            .map(|v| (v.time(), v.value()))
                            .collect::<Vec<_>>()
                    )
                );
            }
        }
    }
    // Warm a prefix ending immediately before a session, then verify both sides
    // of every candidate, including repeated t-1 reads and event multiplicity.
    let expected: Vec<_> = (0..=103)
        .map(|t| wave.samples(&signals, Time::from_ticks(t)).unwrap())
        .collect();
    for start in [0_u64, 3, 4, 5, 10, 12, 13, 20, 100] {
        let mut selection = wave.select(&signals).unwrap();
        selection
            .samples(Time::from_ticks(start.saturating_sub(1)))
            .unwrap();
        for _ in 0..2 {
            let _ = selection
                .query(
                    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(103)),
                    &[0, 2, 3, 4],
                    |ctx| {
                        let t = ctx.time().ticks();
                        for at in [t.checked_sub(1), Some(t), t.checked_sub(1)]
                            .into_iter()
                            .flatten()
                        {
                            let _ = ctx.visit_samples(
                                Time::from_ticks(at),
                                &[0, 1, 2, 3, 4, 5, 6],
                                |slot, sample| {
                                    assert_eq!(
                                        format!("{sample:?}"),
                                        format!("{:?}", expected[at as usize][slot].as_ref())
                                    );
                                    Ok(ControlFlow::<()>::Continue(()))
                                },
                            )?;
                        }
                        Ok(ControlFlow::<()>::Continue(()))
                    },
                )
                .unwrap();
        }
    }
}

#[test]
fn checkpoint_preserves_inactive_state_and_previous_tick_events() {
    let mut body = String::from("#0 b0010 ! 0n ");
    for tick in 1..4096 {
        body.push_str(&format!("#{tick} {}n ", tick % 2));
    }
    body.push_str("1e 1e #4096 $dumpall b1010 ! 1e $end 1e #4097 b1110 !");
    let mut wave = open(
        "$var wire 4 ! bus $end $var wire 1 n noise $end $var event 1 e event $end",
        body.as_bytes(),
    );
    let bus = wave.hierarchy().signal("top.bus").unwrap();
    let event = wave.hierarchy().signal("top.event").unwrap();
    let signals = [bus, bus.slice(1, 0).unwrap(), event];
    let mut selection = wave.select(&signals).unwrap();
    let range = TimeRange::closed(Time::from_ticks(4096), Time::from_ticks(4097));
    for _ in 0..3 {
        let traces = selection.traces(range).unwrap();
        assert_eq!(bits(traces[0].initial().unwrap().value()), "0010");
        assert_eq!(traces[0].initial().unwrap().changed_at(), None);
        assert!(traces[1].changes().is_empty());
        let _ = selection
            .query(range, &[0, 2], |ctx| {
                if ctx.time() == Time::from_ticks(4096) {
                    for (tick, events) in [(4095, 2), (4096, 1), (4095, 2)] {
                        let _ =
                            ctx.visit_samples(Time::from_ticks(tick), &[1, 2], |slot, sample| {
                                match (slot, sample) {
                                    (
                                        1,
                                        ondas::SampleRef::Value {
                                            value,
                                            changed_at: None,
                                            ..
                                        },
                                    ) => assert_eq!(bits(value), "10"),
                                    (2, ondas::SampleRef::Event { occurrences, .. }) => {
                                        assert_eq!(occurrences, events)
                                    }
                                    _ => panic!("unexpected {sample:?}"),
                                }
                                Ok(ControlFlow::<()>::Continue(()))
                            })?;
                    }
                }
                Ok(ControlFlow::<()>::Continue(()))
            })
            .unwrap();
    }
}

#[test]
fn events_reals_strings_and_fixed_storage_class() {
    let mut wave = open(
        "$var event 1 ! ev $end $var real 64 r real $end $var real 1 s text $end",
        b"#5 1! 1! r0 r s123 s #8 1! r-0 r s-0 s",
    );
    let event = wave.hierarchy().signal("top.ev").unwrap();
    let real = wave.hierarchy().signal("top.real").unwrap();
    let text = wave.hierarchy().signal("top.text").unwrap();
    assert_eq!(text.encoding(), Encoding::String);
    assert_eq!(
        wave.hierarchy().variable("top.text").unwrap().kind(),
        "real"
    );
    assert!(matches!(
        wave.sample(real, Time::from_ticks(4)).unwrap(),
        Sample::Missing { .. }
    ));
    assert!(matches!(
        wave.sample(event, Time::from_ticks(5)).unwrap(),
        Sample::Event { occurrences: 2, .. }
    ));
    assert!(
        matches!(value(wave.sample(real, Time::from_ticks(8)).unwrap()), Value::Real(v) if v.to_bits() == (-0.0f64).to_bits())
    );
    assert!(
        matches!(value(wave.sample(text, Time::from_ticks(8)).unwrap()), Value::String(v) if v.as_ref() == "-0")
    );
    assert!(matches!(
        wave.sample(event, Time::from_ticks(9)).unwrap(),
        Sample::Event { occurrences: 0, .. }
    ));
}

#[test]
fn dumpall_only_state_and_strict_blackout_boundaries() {
    let mut wave = open(
        "$var wire 1 ! bit $end $var event 1 e ev $end",
        b"$dumpall 1! 1e $end #6 $dumpoff x! xe $end #10 $dumpon 0! 1e $end 1e #12",
    );
    let bit = wave.hierarchy().signal("top.bit").unwrap();
    let event = wave.hierarchy().signal("top.ev").unwrap();
    for (tick, entering, recorded) in [(6, "1", "x"), (10, "x", "0")] {
        let time = Time::from_ticks(tick);
        let trace = wave.trace(bit, TimeRange::closed(time, time)).unwrap();
        assert_eq!(bits(trace.initial().unwrap().value()), entering);
        assert_eq!(trace.changes().len(), 1);
        assert_eq!(bits(trace.changes()[0].value()), recorded);
    }
    assert!(matches!(
        wave.sample(event, Time::ZERO).unwrap(),
        Sample::Event { occurrences: 0, .. }
    ));
    assert!(matches!(
        wave.sample(event, Time::from_ticks(10)).unwrap(),
        Sample::Event { occurrences: 1, .. }
    ));
}

#[test]
fn latin1_octets_escapes_padding_and_literal_bytes() {
    let mut wave = open(
        "$var string 0 ! text $end",
        b"#0 s\\303\\251\\000\\377\\040\\040 ! #1 sA\\tB\\nC\\\\\\\"\\000\\040 ! #2 s\xc3\xa9 !",
    );
    let signal = wave.hierarchy().signal("top.text").unwrap();
    for (tick, expected) in [
        (0, "\u{c3}\u{a9}\0\u{ff}  "),
        (1, "A\tB\nC\\\"\0 "),
        (2, "\u{c3}\u{a9}"),
    ] {
        assert!(
            matches!(value(wave.sample(signal, Time::from_ticks(tick)).unwrap()), Value::String(v) if v.as_ref() == expected)
        );
    }
}

#[test]
fn replay_after_break_eof_and_batched_projections() {
    let mut wave = open(
        "$var wire 4 ! word [3:0] $end $var reg 4 ! alias [0:3] $end",
        b"#5 b0000 ! #8 b1001 ! b1010 ! b1001 ! #10",
    );
    let signal = wave.hierarchy().signal("top.word").unwrap();
    assert_eq!(signal, wave.hierarchy().signal("top.alias").unwrap());
    let slice = signal.slice(1, 0).unwrap();
    let range = TimeRange::closed(Time::ZERO, Time::from_ticks(20));
    let traces = wave.traces(&[signal, slice, signal], range).unwrap();
    assert_eq!(traces[0].changes().len(), 2);
    assert_eq!(bits(traces[1].changes()[1].value()), "01");
    assert_eq!(
        wave.scan(&[signal], range, |_| ControlFlow::Break(7))
            .unwrap(),
        ControlFlow::Break(7)
    );
    assert_eq!(
        bits(value(wave.sample(signal, Time::from_ticks(5)).unwrap()).as_ref()),
        "0000"
    );
    drop(wave);
    assert_eq!(bits(traces[0].changes()[1].value()), "1001");
}

#[test]
fn indexed_scan_identifies_alias_projection_and_event_slots() {
    let mut wave = open(
        "$var wire 4 ! bus [3:0] $end $var wire 4 ! alias [3:0] $end $var event 1 e event $end",
        b"#0 b0000 ! #2 b1011 ! 1e 1e #4",
    );
    let bus = wave.hierarchy().signal("top.bus").unwrap();
    let alias = wave.hierarchy().signal("top.alias").unwrap();
    let event = wave.hierarchy().signal("top.event").unwrap();
    let high = bus.slice(3, 2).unwrap();
    let selected = [
        bus,
        alias,
        high,
        high,
        bus.slice(2, 1).unwrap(),
        event,
        event,
    ];
    let range = TimeRange::closed(Time::from_ticks(1), Time::from_ticks(2));
    let mut selection = wave.select(&selected).unwrap();
    let mut records = Vec::new();
    let _ = selection
        .scan_each(range, |index, record| {
            assert!(index < selected.len());
            let (signal, time, value) = match record {
                ScanRef::Initial { signal, value, .. } => (signal, None, value),
                ScanRef::Change {
                    signal,
                    time,
                    value,
                } => (signal, Some(time), value),
                _ => panic!("unexpected scan variant"),
            };
            assert_eq!(signal, selected[index]);
            records.push((index, time, value.to_owned()));
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.1.is_none())
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    for (index, expected) in ["1011", "1011", "10", "10", "01"].into_iter().enumerate() {
        let changes = records
            .iter()
            .filter(|r| r.0 == index && r.1.is_some())
            .collect::<Vec<_>>();
        assert_eq!(changes.len(), 1);
        assert_eq!(bits(changes[0].2.as_ref()), expected);
    }
    for index in [5, 6] {
        let changes = records.iter().filter(|r| r.0 == index).collect::<Vec<_>>();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].1, Some(Time::from_ticks(2)));
        assert!(matches!(
            changes[0].2.as_ref(),
            ValueRef::Event { occurrences: 2 }
        ));
    }
    let traces = selection.traces(range).unwrap();
    for (index, trace) in traces.iter().enumerate() {
        assert_eq!(
            trace.changes().len(),
            records
                .iter()
                .filter(|r| r.0 == index && r.1.is_some())
                .count()
        );
    }
    drop(wave);
    assert_eq!(bits(records[0].2.as_ref()), "0000");
}

#[test]
fn missing_interpretation_does_not_prevent_queries() {
    let mut wave = open(
        "$var wire 1 ! unsigned_bit $end $var real 1 r real $end",
        b"#0 1! r2 r",
    );
    for variable in wave.hierarchy().variables() {
        assert_eq!(variable.signedness(), None);
        assert_eq!(variable.logic_domain(), None);
    }
    let signal = wave.hierarchy().signal("top.unsigned_bit").unwrap();
    assert_eq!(
        bits(value(wave.sample(signal, Time::ZERO).unwrap()).as_ref()),
        "1"
    );
}

#[test]
fn event_aliases_share_observed_counts_without_multiplication() {
    let mut wave = open(
        "$var event 1 ! trigger $end $var event 1 ! alias $end",
        b"#0 1! 1! #3 1! #5",
    );
    let event = wave.hierarchy().signal("top.trigger").unwrap();
    let alias = wave.hierarchy().signal("top.alias").unwrap();
    assert_eq!(event, alias);
    let traces = wave
        .traces(&[event, alias, event], TimeRange::all())
        .unwrap();
    for trace in traces {
        assert!(trace.initial().is_none());
        assert_eq!(trace.changes().len(), 2);
        assert!(matches!(
            trace.changes()[0].value(),
            ValueRef::Event { occurrences: 2 }
        ));
        assert!(matches!(
            trace.changes()[1].value(),
            ValueRef::Event { occurrences: 1 }
        ));
    }
    for sample in wave.samples(&[event, alias], Time::from_ticks(5)).unwrap() {
        assert!(matches!(sample, Sample::Event { occurrences: 0, .. }));
    }
}

#[test]
fn exact_large_times_and_decimal_timescale() {
    let input = b"$timescale 0.5 ns $end $var wire 1 ! x $end $enddefinitions $end #9007199254740993.0 1! #18446744073709551615";
    let mut wave =
        ondas::open_bytes("x.vcd", input.as_slice().into()).unwrap_or_else(|e| panic!("{e}"));
    let scale = wave.metadata().timescale().unwrap();
    assert_eq!(scale.factor(), 500);
    assert_eq!(scale.unit(), ondas::TimeUnit::Picosecond);
    assert_eq!(
        wave.metadata().time_span().unwrap().first().ticks(),
        9007199254740993
    );
    assert_eq!(
        wave.metadata().time_span().unwrap().last().ticks(),
        u64::MAX
    );
    let signal = wave.hierarchy().signal("x").unwrap();
    assert_eq!(
        bits(value(wave.sample(signal, Time::from_ticks(u64::MAX)).unwrap()).as_ref()),
        "1"
    );
}

#[test]
fn padded_and_truncated_values_keep_all_logic_states() {
    for width in [1, 7, 65] {
        for state in "01XZHUWL-".chars() {
            let mut wave = open(
                &format!("$var wire {width} ! bits $end"),
                format!("#0 b{state} ! #1 b1010XZHUWL- !").as_bytes(),
            );
            let signal = wave.hierarchy().signal("top.bits").unwrap();
            let fill = if matches!(state, '0' | '1') {
                '0'
            } else {
                state.to_ascii_lowercase()
            };
            let expected = format!(
                "{}{}",
                fill.to_string().repeat(width - 1),
                state.to_ascii_lowercase()
            );
            assert_eq!(
                bits(value(wave.sample(signal, Time::ZERO).unwrap()).as_ref()),
                expected
            );
            let digits = "1010xzhuwl-";
            let expected = if width > digits.len() {
                format!("{}{digits}", "0".repeat(width - digits.len()))
            } else {
                digits[digits.len() - width..].to_owned()
            };
            assert_eq!(
                bits(value(wave.sample(signal, Time::from_ticks(1)).unwrap()).as_ref()),
                expected
            );
        }
    }
}

#[test]
fn rejects_malformed_full_body_not_just_selected_values() {
    let declarations = "$var wire 4 ! bits $end $var real 1 r real $end";
    for body in [
        "#1.5",
        "#18446744073709551616",
        "#2 #1",
        "#0 1unknown",
        "bQ0000 !",
        "$dumpvars b0 !",
        "b1",
        "rwat r",
        "r1e999 r",
        "s\\x+1 r",
        "sfoo r r2 r",
        "r2 r sfoo r",
        "$unknown $end",
        "#1 $dumpvars #2 $end",
    ] {
        assert!(
            matches!(
                ondas::open_bytes("bad.vcd", source(declarations, body.as_bytes()).into()),
                Err(Error::Malformed {
                    format: Format::Vcd,
                    ..
                })
            ),
            "{body}"
        );
    }
    for declarations in [
        "$var wire 4 ! a $end $var wire 5 ! b $end",
        "$var wire 1 ! a $end $var real 1 ! b $end",
    ] {
        assert!(matches!(
            ondas::open_bytes("bad.vcd", source(declarations, b"").into()),
            Err(Error::Malformed { .. })
        ));
    }
}

#[test]
fn producer_headers_literal_array_names_and_zero_width() {
    let input = b"$attrbegin misc 02 STRING 1040 $end $scope vhdl_architecture top $end \
        $var wire 1 ! arr[10][0] $end $var wire 1 ! arr[10][0] $end \
        $var string 0 s text[1:50] $end $var wire 0 z empty $end $upscope $end \
        $dumpvars 1! sx s b z $end #3.0";
    let mut wave = ondas::open_bytes("producer.vcd", input.as_slice().into())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        wave.hierarchy().scope("top").unwrap().kind(),
        "architecture"
    );
    assert!(
        wave.hierarchy()
            .variable("top.\"arr[10][0]\"")
            .unwrap()
            .range()
            .is_none()
    );
    assert_eq!(
        wave.hierarchy()
            .variable("top.\"text[1:50]\"")
            .unwrap()
            .signal()
            .unwrap()
            .encoding(),
        Encoding::String
    );
    let empty = wave.hierarchy().signal("top.empty").unwrap();
    assert_eq!(empty.encoding(), Encoding::Unsupported);
    assert!(matches!(
        wave.sample(empty, Time::ZERO),
        Err(Error::UnsupportedSignal { .. })
    ));
}

#[test]
fn metadata_contextual_codes_and_buffer_boundaries() {
    let mut input = b"\xef\xbb\xbf$date $end $version writer\n  exact $end $comment first $end\n$var wire 1 $end top $end $enddefinitions $end ".to_vec();
    input.extend(std::iter::repeat_n(b' ', 20_000));
    input.extend_from_slice(b"#0 1$end $comment second $end #1");
    let mut wave =
        ondas::open_bytes("metadata.vcd", input.into()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(wave.metadata().date(), Some(""));
    assert_eq!(wave.metadata().writer(), Some("writer\n  exact"));
    assert_eq!(
        wave.metadata().comments().collect::<Vec<_>>(),
        ["first", "second"]
    );
    let signal = wave.hierarchy().signal("top").unwrap();
    assert_eq!(
        bits(value(wave.sample(signal, Time::ZERO).unwrap()).as_ref()),
        "1"
    );
    assert_eq!(
        wave.metadata().comments().collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn contradictory_ranges_and_broken_headers_are_malformed() {
    for declarations in [
        "$var wire 4 ! x [5:0] $end",
        "$var wire 1 ! x [-9223372036854775808:9223372036854775807] $end",
        "$var imaginary 1 ! x $end",
        "$scope alien x $end $var wire 1 ! x $end $upscope $end",
        "$var string 1 ! x nonsense $end",
        "$var wire +1 ! x $end",
    ] {
        assert!(
            matches!(
                ondas::open_bytes("bad.vcd", source(declarations, b"").into()),
                Err(Error::Malformed { .. })
            ),
            "{declarations}"
        );
    }
}

#[test]
fn content_detection_crosses_file_buffers_and_accepts_bom() {
    let directory = std::env::temp_dir().join(format!(
        "ondas-vcd-detection-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    for bom in [false, true] {
        let mut input = if bom {
            b"\xef\xbb\xbf".to_vec()
        } else {
            Vec::new()
        };
        input.extend(std::iter::repeat_n(b' ', 20_000));
        input.extend(source("$var wire 1 ! bit $end", b"#0 1!"));
        for name in ["extensionless", "misleading.FST"] {
            let path = directory.join(name);
            std::fs::write(&path, &input).unwrap();
            for explicit in [false, true] {
                let file = if explicit {
                    ondas::open_with(&path, "vcd-native")
                } else {
                    ondas::open(&path)
                };
                let bytes = if explicit {
                    ondas::open_bytes_with(name, input.clone().into(), "vcd-native")
                } else {
                    ondas::open_bytes(name, input.clone().into())
                };
                for result in [file, bytes] {
                    let mut wave = result.unwrap_or_else(|e| {
                        panic!("bom={bom}, name={name}, explicit={explicit}: {e}")
                    });
                    assert_eq!(wave.format(), Format::Vcd);
                    let signal = wave.hierarchy().signal("top.bit").unwrap();
                    assert_eq!(
                        bits(value(wave.sample(signal, Time::ZERO).unwrap()).as_ref()),
                        "1"
                    );
                }
            }
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opens_and_samples_native_vcd_bytes() {
    let bytes = b"$timescale 1 ns $end $scope module top $end\n\
        $var wire 4 ! word [3:0] $end $upscope $end $enddefinitions $end\n\
        $dumpvars b0 ! $end #2 b101 ! #2 b111 ! #9";
    let mut wave = ondas::open_bytes_with("logical.vcd", bytes.as_slice().into(), "vcd-native")
        .unwrap_or_else(|error| panic!("VCD should open: {error}"));
    assert_eq!(wave.format(), Format::Vcd);
    assert_eq!(wave.backend(), "vcd-native");
    assert_eq!(wave.metadata().source_name(), "logical.vcd");
    assert_eq!(
        wave.metadata().time_span().unwrap().last(),
        Time::from_ticks(9)
    );
    let variable = wave
        .hierarchy()
        .variable_path(&HierarchyPath::from_components(["top", "word"]))
        .unwrap();
    let signal = variable.signal().unwrap();
    assert_eq!(signal.encoding(), Encoding::Bits { width: 4 });
    let sample = wave.sample(signal, Time::from_ticks(2)).unwrap();
    match sample {
        Sample::Value {
            value: Value::Bits(bits),
            changed_at,
            ..
        } => {
            assert_eq!(bits.as_ref().to_string(), "0111");
            assert_eq!(changed_at, Some(Time::from_ticks(2)));
        }
        other => panic!("unexpected sample {other:?}"),
    }
}
