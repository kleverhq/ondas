use super::*;
use crate::{ScanRef, TimeRange};

#[test]
fn indexed_scalar_stops_before_later_selected_values_and_skips_noise() {
    for noise in [1, 10000] {
        let mut wave =
            crate::open_bytes("indexed.ghw", test_bytes::recording(noise).into()).unwrap();
        let control = wave.hierarchy().signal("top.control").unwrap();
        let flow = wave
            .scan(&[control], TimeRange::all(), |record| {
                if let ScanRef::Change { time, value, .. } = record {
                    assert_eq!(time.ticks(), 0);
                    assert!(matches!(value, ValueRef::Bits(bits) if bits.to_string() == "0"));
                    ControlFlow::Break(7)
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert_eq!(flow, ControlFlow::Break(7));
        let crate::backends::Reader::Ghw(reader) = &wave.reader else {
            panic!("GHW")
        };
        assert!(reader.index.is_some());
        assert_eq!(reader.records_read, 1);
        assert_eq!(reader.assembly_bytes, 1);
        let sample = wave.sample(control, Time::from_ticks(1)).unwrap();
        assert!(
            matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), changed_at: Some(time), .. }
            if bits.as_ref().to_string() == "1" && time.ticks() == 1)
        );
    }
}

#[test]
fn indexed_values_preserve_final_tick_states_across_tiny_buffers() {
    use std::collections::BTreeMap;
    for capacity in [1, 2] {
        let data = test_bytes::recording(0);
        let (mut reader, _, _) =
            Reader::open(Box::new(std::io::Cursor::new(data)), "index.ghw".into()).unwrap();
        reader.wire = reader.wire.with_buffer_capacity(capacity);
        reader.wire.seek(reader.body_start).unwrap();
        let mut expected = BTreeMap::new();
        let (_, _) = body::walk(
            &mut reader.wire,
            &reader.types,
            &reader.active,
            Time::from_ticks(4),
            |event| {
                if let body::Event::Scalar {
                    id, time, value, ..
                } = event
                {
                    expected.insert((time, id), value);
                }
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
        reader.wire.seek(reader.body_start).unwrap();
        let ids = reader.active.iter().map(|&(id, _)| id).collect::<Vec<_>>();
        let mut actual = BTreeMap::new();
        let _ = reader
            .index
            .as_ref()
            .unwrap()
            .read(
                &mut reader.wire,
                &reader.types,
                &reader.active,
                &ids,
                Time::from_ticks(4),
                |event| {
                    if let body::Event::Scalar {
                        id, time, value, ..
                    } = event
                    {
                        actual.insert((time, id), value);
                    }
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual[&(Time::from_ticks(2), 3)], body::Scalar::Ordinal(3));
    }
}

#[test]
fn partial_repeated_and_reversed_recipes_preserve_missing_and_projection_time() {
    for selective in [false, true] {
        let mut bytes = if selective {
            test_bytes::padded_recording(0)
        } else {
            test_bytes::recording(0)
        };
        for (declaration, ids) in [
            ([16, 14, 4, 3, 1, 4], [3, 3, 4]),
            ([16, 15, 4, 3, 1, 4], [3, 3, 4]),
            ([16, 16, 4, 4, 1, 3], [4, 3, 3]),
        ] {
            let at = bytes
                .windows(declaration.len())
                .position(|b| b == declaration)
                .unwrap();
            bytes[at + 3..at + 6].copy_from_slice(&ids);
        }
        let body = bytes.windows(4).position(|b| b == b"SNP\0").unwrap();
        bytes.truncate(body);
        bytes.extend(b"CYC\0");
        bytes.extend(0i64.to_le_bytes());
        // Active source IDs start 3, 4, 5. Source 4 appears only at tick 2.
        bytes.extend([1, 3, 0, 1, 1, 2, 0, 1, 2, 3, 0, 1, 2, 2, 0, 127]);
        bytes.extend(b"ECY\0");
        let mut wave = crate::open_bytes("indexed-partial.ghw", bytes.into()).unwrap();
        let wide = wave.hierarchy().signal("top.wide").unwrap();
        let reversed = wave.hierarchy().signal("top.reversed").unwrap();
        let low = wide.slice(0, 0).unwrap();
        for tick in [0, 1, 2, 3, 1, 0, 3, 2] {
            let samples = wave
                .samples(&[wide, reversed, low], Time::from_ticks(tick))
                .unwrap();
            if tick < 2 {
                assert!(
                    samples
                        .iter()
                        .all(|sample| matches!(sample, crate::Sample::Missing { .. }))
                );
            } else {
                let expected = if tick == 2 {
                    ["001", "100", "1"]
                } else {
                    ["000", "000", "0"]
                };
                for (sample, expected) in samples.iter().zip(expected) {
                    assert!(
                        matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), changed_at, .. }
                    if bits.as_ref().to_string() == expected && changed_at.map(|time| time.ticks()) == (tick > 2).then_some(3)),
                        "{sample:?}; expected {expected} at tick{tick}"
                    );
                }
            }
        }
        let crate::backends::Reader::Ghw(reader) = &wave.reader else {
            panic!("GHW")
        };
        assert!(reader.index.is_some());
        assert_eq!(
            2 < reader.active.len().div_ceil(2),
            selective,
            "exercise both selective and broad routing with an unused source and ID holes"
        );
    }
}

#[test]
fn indexed_equal_ticks_across_sections_cancel_excursions() {
    let original = test_bytes::padded_recording(0);
    let start = original.windows(4).position(|b| b == b"SNP\0").unwrap();
    let end = original.windows(4).position(|b| b == b"ESN\0").unwrap();
    let values = &original[start + 16..end];
    let mut bytes = original[..start].to_vec();
    for (time, cycle) in [(0i64, true), (0, false), (1, true)] {
        bytes.extend(b"SNP\0\0\0\0\0");
        bytes.extend(time.to_le_bytes());
        bytes.extend(values);
        bytes.extend(b"ESN\0");
        if !cycle {
            continue;
        }
        bytes.extend(b"CYC\0");
        bytes.extend(time.to_le_bytes());
        bytes.extend([3, 3, 0, 127]);
        bytes.extend(b"ECY\0");
    }
    // The final SNP at tick1 cancels its low-bit excursion.
    bytes.extend(b"SNP\0\0\0\0\0");
    bytes.extend(1i64.to_le_bytes());
    bytes.extend(values);
    bytes.extend(b"ESN\0CYC\0");
    bytes.extend(2i64.to_le_bytes());
    bytes.extend([1, 3, 0, 127]);
    bytes.extend(b"ECY\0");
    let mut wave = crate::open_bytes("indexed-sections.ghw", bytes.into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let reversed = wave.hierarchy().signal("top.reversed").unwrap();
    let low = wide.slice(0, 0).unwrap();
    let samples = wave
        .samples(&[wide, reversed, low], Time::from_ticks(2))
        .unwrap();
    for (sample, (bits_expected, time_expected)) in
        samples
            .iter()
            .zip([("110", Some(2)), ("011", Some(2)), ("0", None)])
    {
        assert!(
            matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), changed_at, .. }
            if bits.as_ref().to_string() == bits_expected && changed_at.map(|time| time.ticks()) == time_expected),
            "{sample:?}; expected {bits_expected} at{time_expected:?}"
        );
    }
    let trace = wave.trace(wide, TimeRange::all()).unwrap();
    assert_eq!(
        trace
            .changes()
            .iter()
            .map(|change| change.time().ticks())
            .collect::<Vec<_>>(),
        [0, 2]
    );
}

#[test]
fn early_break_has_a_scalar_read_budget_and_bounded_assembly() {
    for length in [10, 10000] {
        let mut wave =
            crate::open_bytes("budget.ghw", test_bytes::recording(length).into()).unwrap();
        let wide = wave.hierarchy().signal("top.wide").unwrap();
        let low = wide.slice(0, 0).unwrap();
        let signals = [wide, low, wide, low];
        let result = wave
            .scan(&signals, TimeRange::all(), |record| {
                if let ScanRef::Change { time, .. } = record
                    && time.ticks() == 1
                {
                    ControlFlow::Break(7)
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert_eq!(result, ControlFlow::Break(7));
        let crate::backends::Reader::Ghw(reader) = &wave.reader else {
            panic!("GHW");
        };
        assert_eq!(
            reader.records_read, 5,
            "four snapshot scalars and one selected change; no unrelated suffix"
        );
        assert_eq!(
            reader.assembly_bytes, 3,
            "one whole recipe despite projections and duplicates"
        );
    }
}

#[test]
fn binary64_reads_cross_tiny_wire_buffers() {
    use std::io::{BufReader, Cursor};
    for capacity in [1, 2] {
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::with_capacity(
                capacity,
                Cursor::new(test_bytes::real_recording(true, 1, 0x7ff8000000001234)),
            )),
            "real.ghw".into(),
        )
        .unwrap();
        reader.wire = reader.wire.with_buffer_capacity(capacity);
        let r = hierarchy.signal("top.r").unwrap();
        let mut values = Vec::new();
        let flow = reader
            .read(&[r], Time::from_ticks(4), |event| {
                if let Notification::Value {
                    time,
                    value: ValueRef::Real(real),
                    ..
                } = event
                {
                    values.push((time.ticks(), real.to_bits()));
                }
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert!(flow.is_continue());
        assert_eq!(values, [(3, 0x8000000000000000), (4, 0x7ff8000000001234)]);
    }
}

#[test]
fn batch_shares_decoding_and_fresh_reads_survive_unwinding() {
    let mut wave = crate::open_bytes("reuse.ghw", test_bytes::recording(0).into()).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let low = wide.slice(0, 0).unwrap();
    let _ = wave
        .samples(&[wide, low, wide], Time::from_ticks(4))
        .unwrap();
    let crate::backends::Reader::Ghw(reader) = &wave.reader else {
        panic!("GHW");
    };
    assert_eq!(reader.records_read, 9);
    assert_eq!(reader.assembly_bytes, 3);
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = wave.scan(&[wide], TimeRange::all(), |_| -> ControlFlow<()> {
            panic!("visitor");
        });
    }));
    assert!(stopped.is_err());
    let sample = wave.sample(wide, Time::from_ticks(1)).unwrap();
    assert!(
        matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), .. } if bits.as_ref().to_string()=="101")
    );
}

#[test]
fn late_io_failure_does_not_publish_an_unfinished_tick() {
    use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    struct Failing {
        input: Cursor<Vec<u8>>,
        cutoff: Arc<AtomicU64>,
    }
    impl Read for Failing {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let bytes = self.fill_buf()?;
            let n = output.len().min(bytes.len());
            output[..n].copy_from_slice(&bytes[..n]);
            self.consume(n);
            Ok(n)
        }
    }
    impl BufRead for Failing {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            let remaining = self
                .cutoff
                .load(Ordering::Relaxed)
                .checked_sub(self.input.position())
                .ok_or_else(|| io::Error::other("injected read failure"))?;
            if remaining == 0 {
                return Err(io::Error::other("injected read failure"));
            }
            let bytes = self.input.fill_buf()?;
            Ok(&bytes[..bytes
                .len()
                .min(usize::try_from(remaining).unwrap_or(usize::MAX))])
        }
        fn consume(&mut self, n: usize) {
            self.input.consume(n);
        }
    }
    impl Seek for Failing {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.input.seek(from)
        }
    }
    let data = test_bytes::recording(0);
    let mut wave = crate::open_bytes("failing.ghw", data.clone().into()).unwrap();
    let cutoff = Arc::new(AtomicU64::new(u64::MAX));
    let fail_at = data.windows(4).position(|b| b == b"ESN\0").unwrap() + 4 + 4 + 2;
    let selected_fail_at = data.windows(4).position(|b| b == b"CYC\0").unwrap() + 4 + 8 + 1;
    let (reader, _, _) = Reader::open(
        Box::new(Failing {
            input: Cursor::new(data),
            cutoff: cutoff.clone(),
        }),
        "failing.ghw".into(),
    )
    .unwrap();
    // The diagnostic input has exactly the same declared recipes.
    wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    cutoff.store(fail_at as u64, Ordering::Relaxed);
    let mut delivered = 0;
    let result = wave.scan(&[wide], TimeRange::all(), |_| {
        delivered += 1;
        ControlFlow::<()>::Continue(())
    });
    assert!(matches!(result, Err(crate::Error::Io(_))));
    assert_eq!(delivered, 0);
    cutoff.store(u64::MAX, Ordering::Relaxed);
    let sample = wave.sample(wide, Time::from_ticks(3)).unwrap();
    assert!(
        matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), .. } if bits.as_ref().to_string()=="111")
    );

    // Indexed reads skip unrelated headers but propagate required payload failures.
    let control = wave.hierarchy().signal("top.control").unwrap();
    cutoff.store(selected_fail_at as u64, Ordering::Relaxed);
    let mut delivered_times = Vec::new();
    let result = wave.scan(&[control], TimeRange::all(), |record| {
        if let ScanRef::Change { time, .. } = record {
            delivered_times.push(time.ticks());
        }
        ControlFlow::<()>::Continue(())
    });
    assert!(matches!(result, Err(crate::Error::Io(_))));
    assert_eq!(delivered_times, [0]);
    assert!(matches!(
        wave.traces(&[control], TimeRange::all()),
        Err(crate::Error::Io(_))
    ));
    cutoff.store(u64::MAX, Ordering::Relaxed);
    let sample = wave.sample(control, Time::from_ticks(1)).unwrap();
    assert!(
        matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), changed_at: Some(time), .. }
        if bits.as_ref().to_string() == "1" && time.ticks() == 1)
    );

    let mut data = test_bytes::padded_recording(0);
    let cycle = data.windows(4).position(|b| b == b"CYC\0").unwrap();
    // Two selected sources change at tick1. Fail on the second payload.
    data.splice(cycle + 12..cycle + 15, [1, 3, 2, 3, 0]);
    let mut wave = crate::open_bytes("partial-failure.ghw", data.clone().into()).unwrap();
    let (reader, _, _) = Reader::open(
        Box::new(Failing {
            input: Cursor::new(data),
            cutoff: cutoff.clone(),
        }),
        "partial-failure.ghw".into(),
    )
    .unwrap();
    wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let low = wide.slice(0, 0).unwrap();
    cutoff.store((cycle + 15) as u64, Ordering::Relaxed);
    let mut times = Vec::new();
    let result = wave.scan(&[wide, low], TimeRange::all(), |record| {
        if let ScanRef::Change { time, .. } = record {
            times.push(time.ticks());
        }
        ControlFlow::<()>::Continue(())
    });
    assert!(matches!(result, Err(crate::Error::Io(_))));
    assert_eq!(
        times,
        [0, 0],
        "the partially assembled tick must not publish"
    );
    assert!(matches!(
        wave.traces(&[wide, low], TimeRange::all()),
        Err(crate::Error::Io(_))
    ));
    cutoff.store(u64::MAX, Ordering::Relaxed);
    let sample = wave.sample(wide, Time::from_ticks(1)).unwrap();
    assert!(
        matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), .. } if bits.as_ref().to_string() == "111")
    );
}
