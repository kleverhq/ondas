use super::{container::Container, debug, types, wire::Source};
use crate::Error;
use std::io::Cursor;

fn put32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(data: &mut [u8], at: usize, value: u64) {
    data[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

// A minimal simulation with no declarations, pages or native dependencies.
fn empty(version: u8) -> Vec<u8> {
    let mut data = vec![0; 0x400];
    data[..23].copy_from_slice(b"Xilinx WAVE DATABASE 01");
    data[24..40].copy_from_slice(b"Xilinx Simulator");
    for (i, (name, offset, len)) in [
        ("WDB.Event", 0xc8, 0x48),
        ("Xilinx RTTI", 0x200, 40),
        ("Xilinx DBG", 0x240, 0xb8),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = 0x110 + i * 48;
        put64(&mut data, 0x48 + 8 * i, entry as u64);
        data[entry..entry + name.len()].copy_from_slice(name.as_bytes());
        put64(&mut data, entry + 24, 1);
        put64(&mut data, entry + 32, offset);
        put64(&mut data, entry + 40, len);
    }
    put32(&mut data, 0xc8 + 64, 10240);
    data[0x200..0x219].copy_from_slice(b"Xilinx ISim TYPE FILE 001");
    put32(&mut data, 0x200 + 36, 40);
    let magic = format!("Xilinx ISim DBG 00{version}");
    data[0x240..0x240 + magic.len()].copy_from_slice(magic.as_bytes());
    put32(&mut data, 0x240 + 24, (-12_i32) as u32);
    for i in 0..18 {
        put32(&mut data, 0x240 + 28 + i * 4, 0xb8);
    }
    data
}

fn append_words(bytes: &mut Vec<u8>, words: &[u32]) {
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.resize(bytes.len().next_multiple_of(8), 0);
}

// Metadata has real region boundaries; the referenced page stays opaque here.
fn populated(version: u8) -> Vec<u8> {
    let mut data = vec![0; 0x14c1];
    data[..23].copy_from_slice(b"Xilinx WAVE DATABASE 01");
    data[24..40].copy_from_slice(b"Xilinx Simulator");
    put64(&mut data, 0xc8, 0x1000);
    put64(&mut data, 0xd0, 10);
    put32(&mut data, 0xd0 + 12, 1);
    put64(&mut data, 0xd0 + 24, 2048);
    put64(&mut data, 0xd0 + 48, 1);
    put64(&mut data, 0xd0 + 56, 0x220);
    put32(&mut data, 0xd0 + 64, 10240);
    put64(&mut data, 0x1000 + 8, 0x14c0);
    put32(&mut data, 0x1000 + 0x328, 1);
    put32(&mut data, 0x1000 + 0x4b8, 1);

    let mut rtti = vec![0; 40];
    rtti[..25].copy_from_slice(b"Xilinx ISim TYPE FILE 001");
    let mut body = b"BIT\0".to_vec();
    for word in [2_u32, 2, 2, 2] {
        body.extend_from_slice(&word.to_le_bytes());
    }
    body.extend_from_slice(b"'0'\0'1'\0");
    body.extend_from_slice(&1_u32.to_le_bytes());
    rtti.extend_from_slice(&(8_u32 + body.len() as u32).to_le_bytes());
    rtti.extend_from_slice(&0xa000_0003_u32.to_le_bytes());
    rtti.extend_from_slice(&body);
    let end = rtti.len() as u32;
    put32(&mut rtti, 32, 1);
    put32(&mut rtti, 36, end);
    rtti.extend_from_slice(&40_u64.to_le_bytes());

    let mut dbg = vec![0; 184];
    let magic = format!("Xilinx ISim DBG 00{version}");
    dbg[..magic.len()].copy_from_slice(magic.as_bytes());
    put32(&mut dbg, 24, (-12_i32) as u32);
    let mut offsets = [0; 18];
    offsets[0] = dbg.len();
    for row in [
        [0, u32::MAX, 0, 1, 1, u32::MAX, 0, 0, 0],
        [5, 0, 0, 0, u32::MAX, 0, 0, 0, 1],
    ] {
        for word in row {
            dbg.extend_from_slice(&word.to_le_bytes());
        }
    }
    offsets[1] = dbg.len();
    for mut row in [
        vec![u32::MAX, u32::MAX, 0x13, 0, 0, 0, 0, 0, 0],
        vec![5, u32::MAX, 9, 1, 0, 0, 0, 0, 0],
    ] {
        if version == 4 {
            row.remove(4);
        }
        for word in row {
            dbg.extend_from_slice(&word.to_le_bytes());
        }
    }
    dbg.resize(dbg.len().next_multiple_of(8), 0);
    offsets[3] = dbg.len();
    let mut decl = vec![0, 0, 0, 0, 1, 0, 0, 0, 0xe, 5];
    if version != 4 {
        decl.push(0);
    }
    append_words(&mut dbg, &decl);
    offsets[4] = dbg.len();
    offsets[5..10].fill(dbg.len());
    dbg.extend_from_slice(b"_top\0top\0");
    dbg.resize(dbg.len().next_multiple_of(8), 0);
    offsets[10] = dbg.len();
    dbg.extend_from_slice(b"flag\0");
    dbg.resize(dbg.len().next_multiple_of(8), 0);
    offsets[11..18].fill(dbg.len());
    append_words(&mut dbg, &[0, 0, 0]);
    offsets[2] = dbg.len();
    let mut object = vec![
        0;
        match version {
            4 => 40,
            5 => 48,
            _ => 56,
        }
    ];
    put32(&mut object, 16, 1);
    dbg.extend_from_slice(&object);
    for (i, offset) in offsets.into_iter().enumerate() {
        put32(&mut dbg, 28 + 4 * i, offset as u32);
    }
    for (i, count) in [2, 2, 1, 1].into_iter().enumerate() {
        put32(&mut dbg, 100 + 4 * i, count);
    }
    put32(&mut dbg, 116 + 4 * 5, 9);
    put32(&mut dbg, 116 + 4 * 6, 5);
    put32(&mut dbg, 116 + 4 * 13, 1);

    for (i, (name, offset, section)) in [
        ("WDB.Event", 0xd0, data[0xd0..0x118].to_vec()),
        ("Xilinx RTTI", 0x300, rtti),
        ("Xilinx DBG", 0x400, dbg),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = 0x180 + 48 * i;
        put64(&mut data, 0x48 + 8 * i, entry as u64);
        data[entry..entry + name.len()].copy_from_slice(name.as_bytes());
        put64(&mut data, entry + 24, 1);
        put64(&mut data, entry + 32, offset as u64);
        put64(&mut data, entry + 40, section.len() as u64);
        data[offset..offset + section.len()].copy_from_slice(&section);
    }
    data
}

fn recorded(version: u8, records: &[(u64, u32, &[u8])]) -> Vec<u8> {
    let mut page = Vec::new();
    page.extend_from_slice(&records.first().unwrap().0.to_le_bytes());
    page.extend_from_slice(&(records.last().unwrap().0 - records.first().unwrap().0).to_le_bytes());
    page.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for &(time, key, value) in records {
        page.extend_from_slice(&time.to_le_bytes());
        page.extend_from_slice(&key.to_le_bytes());
        page.extend_from_slice(&(value.len() as u32).to_le_bytes());
        page.extend_from_slice(value);
    }
    page.resize(10240, 0);
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&page, 6);
    let mut bytes = populated(version);
    bytes.truncate(0x14c0);
    put32(&mut bytes, 0x1000 + 0x328, compressed.len() as u32);
    bytes.extend_from_slice(&compressed);
    bytes
}

fn recorded_integer(records: &[(u64, u32, &[u8])]) -> Vec<u8> {
    let mut data = recorded(6, records);
    let mut rtti = vec![0; 40];
    rtti[..25].copy_from_slice(b"Xilinx ISim TYPE FILE 001");
    let mut body = b"INTEGER\0".to_vec();
    for word in [2, i32::MIN as u32, i32::MAX as u32, 1] {
        body.extend_from_slice(&word.to_le_bytes());
    }
    rtti.extend_from_slice(&(8 + body.len() as u32).to_le_bytes());
    rtti.extend_from_slice(&0xa000_0005_u32.to_le_bytes());
    rtti.extend_from_slice(&body);
    let end = rtti.len() as u32;
    put32(&mut rtti, 32, 1);
    put32(&mut rtti, 36, end);
    rtti.extend_from_slice(&40_u64.to_le_bytes());
    data[0x300..0x300 + rtti.len()].copy_from_slice(&rtti);
    put64(&mut data, 0x180 + 48 + 40, rtti.len() as u64);
    let declarations = u32::from_le_bytes(data[0x428..0x42c].try_into().unwrap()) as usize;
    put32(&mut data, 0x400 + declarations + 16, 4);
    data
}

#[test]
fn wdb_cold_points_whole_writes_partial_fallback_and_page_boundaries() {
    use crate::{Sample, Time, Value};
    let zero = 0_u32.to_le_bytes();
    let one = 1_u32.to_le_bytes();
    let value256 = 256_u32.to_le_bytes();
    let full = recorded_integer(&[
        (2, 0, &zero),
        (3, 0, &one),
        (3, 0, &zero),
        (7, 0, &one),
        (8, 0, &one),
    ]);
    let partial = recorded_integer(&[
        (2, 0, &zero),
        (7, 1, &[1]),
        (7, 0, &[1]),
        (8, 0, &[0]),
        (9, 0, &value256),
    ]);
    let initial_parts =
        recorded_integer(&[(2, 1, &[1]), (3, 0, &[2]), (4, 2, &[0, 0]), (5, 0, &zero)]);
    let mut pages = recorded_integer(&[(2, 0, &zero), (7, 0, &one)]);
    let second = recorded(6, &[(7, 0, &zero), (8, 0, &zero)]).split_off(0x14c0);
    let offset = pages.len() as u64;
    put64(&mut pages, 0x1000 + 16, offset);
    put32(&mut pages, 0x1000 + 0x32c, second.len() as u32);
    put32(&mut pages, 0x1000 + 0x4b8, 2);
    pages.extend_from_slice(&second);
    type Point = (u64, Option<(u32, Option<u64>)>);
    let cases: &[(&[u8], &[Point])] = &[
        (
            &full,
            &[
                (0, None),
                (2, Some((0, None))),
                (3, Some((0, None))),
                (7, Some((1, Some(7)))),
                (100, Some((1, Some(7)))),
                (u64::MAX, Some((1, Some(7)))),
            ],
        ),
        (
            &partial,
            &[
                (0, None),
                (7, Some((257, Some(7)))),
                (8, Some((256, Some(8)))),
                (100, Some((256, Some(8)))),
            ],
        ),
        (
            &initial_parts,
            &[
                (2, None),
                (3, None),
                (4, Some((258, None))),
                (5, Some((0, Some(5)))),
            ],
        ),
        (
            &pages,
            &[
                (2, Some((0, None))),
                (7, Some((0, None))),
                (8, Some((0, None))),
                (100, Some((0, None))),
            ],
        ),
    ];
    for (bytes, cases) in cases {
        for &(tick, expected) in *cases {
            // Fresh selections exercise the cold path, without a primed replay.
            let mut wave = crate::open_bytes("point.wdb", (*bytes).into()).unwrap();
            let signal = wave.hierarchy().signal("top.flag").unwrap();
            let actual = wave.sample(signal, Time::from_ticks(tick)).unwrap();
            match (actual, expected) {
                (Sample::Missing { .. }, None) => (),
                (
                    Sample::Value {
                        value: Value::Bits(bits),
                        changed_at,
                        ..
                    },
                    Some((value, changed)),
                ) => {
                    assert_eq!(
                        bits.as_ref().to_string(),
                        format!("{value:032b}"),
                        "t{tick}"
                    );
                    assert_eq!(changed_at.map(Time::ticks), changed, "t{tick}");
                }
                _ => panic!("wrong observation at {tick}"),
            }
        }
    }
    for bytes in [&full, &pages] {
        let mut wave = crate::open_bytes("native.wdb", bytes.clone().into()).unwrap();
        let signal = wave.hierarchy().signal("top.flag").unwrap();
        let crate::backends::Reader::Wdb(reader) = &mut wave.reader else {
            unreachable!()
        };
        assert!(
            reader
                .sample_bits(&[signal], Time::from_ticks(100))
                .unwrap()
                .is_some()
        );
    }
    let mut wave = crate::open_bytes("partial.wdb", partial.into()).unwrap();
    let signal = wave.hierarchy().signal("top.flag").unwrap();
    let crate::backends::Reader::Wdb(reader) = &mut wave.reader else {
        unreachable!()
    };
    assert!(
        reader
            .sample_bits(&[signal], Time::from_ticks(100))
            .unwrap()
            .is_none()
    );
}

#[test]
fn wdb_cold_points_preserve_retained_order_and_reject_probed_bad_boundaries() {
    use crate::{Sample, Time, Value};
    let mut data = recorded_integer(&[
        (2, 0, &[0, 0, 0, 0]),
        (2, 8, &[1, 0, 0, 0]),
        (7, 0, &[2, 0, 0, 0]),
        (7, 8, &[3, 0, 0, 0]),
    ]);
    let len = u64::from_le_bytes(data[0x208..0x210].try_into().unwrap()) as usize;
    let instances = u32::from_le_bytes(data[0x424..0x428].try_into().unwrap()) as usize;
    let mut object = data[0x400 + instances..0x400 + instances + 56].to_vec();
    put64(&mut object, 0, 8);
    data[0x400 + len..0x400 + len + 56].copy_from_slice(&object);
    put64(&mut data, 0x208, (len + 56) as u64);
    put32(&mut data, 0x400 + 108, 2);
    let mut wave = crate::open_bytes("order.wdb", data.into()).unwrap();
    let signals: Vec<_> = wave
        .hierarchy()
        .variables()
        .map(|v| v.signal().unwrap())
        .collect();
    assert_eq!(signals.len(), 2);
    let crate::backends::Reader::Wdb(reader) = &mut wave.reader else {
        unreachable!()
    };
    assert!(
        reader
            .sample_bits(&[signals[1], signals[0]], Time::from_ticks(7))
            .unwrap()
            .is_some()
    );
    let actual = wave
        .samples(&[signals[1], signals[0], signals[1]], Time::from_ticks(7))
        .unwrap();
    assert_eq!(actual.len(), 3);
    for (sample, expected) in actual.iter().zip([3, 2, 3]) {
        let Sample::Value {
            value: Value::Bits(bits),
            changed_at,
            ..
        } = sample
        else {
            panic!("expected bits")
        };
        assert_eq!(bits.as_ref().to_string(), format!("{expected:032b}"));
        assert_eq!(*changed_at, Some(Time::from_ticks(7)));
    }
    let mut data = recorded_integer(&[(2, 0, &[0, 0, 0, 0]), (7, 0, &[1, 0, 0, 0])]);
    let second = recorded(6, &[(3, 0, &[0, 0, 0, 0]), (8, 0, &[1, 0, 0, 0])]).split_off(0x14c0);
    let offset = data.len() as u64;
    put64(&mut data, 0x1000 + 16, offset);
    put32(&mut data, 0x1000 + 0x32c, second.len() as u32);
    put32(&mut data, 0x1000 + 0x4b8, 2);
    data.extend_from_slice(&second);
    let mut wave = crate::open_bytes("bad-order.wdb", data.into()).unwrap();
    let signal = wave.hierarchy().signal("top.flag").unwrap();
    assert!(matches!(
        wave.sample(signal, Time::from_ticks(2)),
        Err(Error::Malformed { .. })
    ));
}

#[test]
fn wdb_cold_point_combines_final_bytes_across_arenas() {
    use crate::{Sample, Time, Value};
    let mut data = recorded_integer(&[(2, 2046, &[0, 0]), (7, 2046, &[1, 0]), (7, 2046, &[0, 0])]);
    let event = data[0xd0..0x118].to_vec();
    data[0xe0..0x128].copy_from_slice(&event);
    put64(&mut data, 0x180 + 32, 0xe0);
    put32(&mut data, 0xe0 + 12, 2);
    put64(&mut data, 0xe0 + 24, 4096);
    put64(&mut data, 0xd0, 0x1000);
    put64(&mut data, 0xd8, 0x2000);
    let instances = u32::from_le_bytes(data[0x424..0x428].try_into().unwrap()) as usize;
    put64(&mut data, 0x400 + instances, 2046);
    let second = recorded(6, &[(2, 0, &[0, 0]), (7, 0, &[1, 0])]).split_off(0x14c0);
    data.resize(0x24c0, 0);
    put64(&mut data, 0x2000 + 8, 0x24c0);
    put32(&mut data, 0x2000 + 0x328, second.len() as u32);
    put32(&mut data, 0x2000 + 0x4b8, 1);
    data.extend_from_slice(&second);
    let mut wave = crate::open_bytes("arenas.wdb", data.into()).unwrap();
    let signal = wave.hierarchy().signal("top.flag").unwrap();
    let crate::backends::Reader::Wdb(reader) = &mut wave.reader else {
        unreachable!()
    };
    assert!(
        reader
            .sample_bits(&[signal], Time::from_ticks(7))
            .unwrap()
            .is_some()
    );
    let Sample::Value {
        value: Value::Bits(bits),
        changed_at,
        ..
    } = wave.sample(signal, Time::from_ticks(7)).unwrap()
    else {
        panic!("expected bits")
    };
    assert_eq!(bits.as_ref().to_string(), format!("{:032b}", 65536));
    assert_eq!(changed_at, Some(Time::from_ticks(7)));
}

#[test]
fn wdb_public_values_stop_repeat_and_composition() {
    use crate::{Sample, ScanRef, Time, TimeRange, Value};
    use std::ops::ControlFlow;
    for version in [4, 5, 6] {
        let bytes: std::sync::Arc<[u8]> = recorded(
            version,
            &[(0, 0, &[0]), (3, 0, &[1]), (3, 0, &[0]), (7, 0, &[1])],
        )
        .into();
        let mut wave = crate::open_bytes_with("source.bin", bytes.clone(), "wdb-native").unwrap();
        assert_eq!(wave.format(), crate::Format::Wdb);
        assert_eq!(wave.backend(), "wdb-native");
        let signal = wave.hierarchy().signal("top.flag").unwrap();
        let assert_value = |sample: Sample, expected: &str| {
            let Sample::Value {
                value: Value::Bits(bits),
                ..
            } = sample
            else {
                panic!("expected bits")
            };
            assert_eq!(bits.as_ref().to_string(), expected);
        };
        for (tick, expected) in [(3, "0"), (7, "1"), (0, "0"), (100, "1")] {
            assert_value(
                wave.sample(signal, Time::from_ticks(tick)).unwrap(),
                expected,
            );
        }
        let joined = wave.concat(&[signal, signal]).unwrap();
        assert_value(wave.sample(joined, Time::from_ticks(7)).unwrap(), "11");
        let range = TimeRange::closed(Time::ZERO, Time::from_ticks(10));
        let mut changes = Vec::new();
        let _ = wave
            .select(&[signal])
            .unwrap()
            .scan(range, |record| {
                if let ScanRef::Change { time, value, .. } = record {
                    changes.push((time.ticks(), value.to_owned()));
                }
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].0, 0);
        assert_eq!(changes[1].0, 7);
        let outcome = wave
            .select(&[signal])
            .unwrap()
            .scan(range, |_| ControlFlow::Break(42))
            .unwrap();
        assert_eq!(outcome, ControlFlow::Break(42));
        assert_value(wave.sample(signal, Time::from_ticks(7)).unwrap(), "1");
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = wave
                .select(&[signal])
                .unwrap()
                .scan(range, |_| -> ControlFlow<()> { panic!("consumer") });
        }));
        assert!(panic.is_err());
        assert_value(wave.sample(signal, Time::ZERO).unwrap(), "0");
        let mut nested = crate::open_bytes("nested.wdb", bytes).unwrap();
        let other = nested.hierarchy().signal("top.flag").unwrap();
        let _ = wave
            .select(&[signal])
            .unwrap()
            .scan(range, |_| {
                assert_value(nested.sample(other, Time::from_ticks(7)).unwrap(), "1");
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
    }
}

#[test]
fn wdb_prepared_repeat_avoids_source_reads_and_invalidates_on_stop() {
    use crate::{Sample, ScanRef, Time, TimeRange, Value, ValueRef};
    use std::{
        io::{BufRead, Read, Seek, SeekFrom},
        ops::ControlFlow,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    struct Counted {
        cursor: std::io::Cursor<Arc<[u8]>>,
        reads: Arc<AtomicUsize>,
    }
    impl Read for Counted {
        fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.cursor.read(data)
        }
    }
    impl BufRead for Counted {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            self.cursor.fill_buf()
        }
        fn consume(&mut self, n: usize) {
            self.cursor.consume(n);
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, at: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(at)
        }
    }
    let bytes: Arc<[u8]> =
        recorded(6, &[(2, 0, &[0]), (3, 0, &[1]), (3, 0, &[0]), (7, 0, &[1])]).into();
    let mut wave = crate::open_bytes("repeat.wdb", bytes.clone()).unwrap();
    let crate::backends::Reader::Wdb(reader) = &mut wave.reader else {
        unreachable!()
    };
    let reads = Arc::new(AtomicUsize::new(0));
    reader.source = Source::new(Box::new(Counted {
        cursor: std::io::Cursor::new(bytes),
        reads: reads.clone(),
    }))
    .unwrap();
    let s = wave.hierarchy().signal("top.flag").unwrap();
    let joined = wave.concat(&[s, s]).unwrap();
    let mut selection = wave.select(&[s, joined, s]).unwrap();
    let check = |samples: Vec<Sample>, values: &[&str], changed: Option<u64>| {
        assert_eq!(samples.len(), values.len());
        for (sample, expected) in samples.iter().zip(values) {
            let Sample::Value {
                value: Value::Bits(bits),
                changed_at,
                ..
            } = sample
            else {
                panic!("expected bits")
            };
            assert_eq!(bits.as_ref().to_string(), *expected);
            assert_eq!(changed_at.map(Time::ticks), changed);
        }
    };
    let check_window = |selection: &mut crate::Selection<'_>, start, end| {
        let mut rows = Vec::new();
        let _ = selection
            .scan_each(
                TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end)),
                |index, record| {
                    let (time, value, changed_at) = match record {
                        ScanRef::Initial {
                            value, changed_at, ..
                        } => (None, value, changed_at.map(Time::ticks)),
                        ScanRef::Change { time, value, .. } => (Some(time.ticks()), value, None),
                    };
                    let ValueRef::Bits(bits) = value else {
                        panic!("expected bits")
                    };
                    rows.push((index, time, bits.to_string(), changed_at));
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (0, None, "0".into(), None),
                (1, None, "00".into(), None),
                (2, None, "0".into(), None),
                (0, Some(7), "1".into(), None),
                (1, Some(7), "11".into(), None),
                (2, Some(7), "1".into(), None),
            ]
        );
    };
    check(
        selection.samples(Time::from_ticks(5)).unwrap(),
        &["0", "00", "0"],
        None,
    );
    let before = reads.load(Ordering::Relaxed);
    check(
        selection.samples(Time::from_ticks(5)).unwrap(),
        &["0", "00", "0"],
        None,
    );
    assert_eq!(
        reads.load(Ordering::Relaxed),
        before,
        "prepared repeat must not read the source"
    );
    check(
        selection.samples(Time::from_ticks(7)).unwrap(),
        &["1", "11", "1"],
        Some(7),
    );
    assert!(reads.load(Ordering::Relaxed) > before);
    // A point snapshot still holds the entering state and pending boundary tick.
    check_window(&mut selection, 7, 100);
    // The second window must use the inclusive checkpoint: replay starts at 7.
    check_window(&mut selection, 5, 10);
    check_window(&mut selection, 5, 10);
    assert!(
        selection
            .samples(Time::ZERO)
            .unwrap()
            .iter()
            .all(|s| matches!(s, Sample::Missing { .. }))
    );
    check(
        selection.samples(Time::from_ticks(100)).unwrap(),
        &["1", "11", "1"],
        Some(7),
    );
    assert_eq!(
        selection
            .visit_samples(Time::from_ticks(100), |_| ControlFlow::Break(42))
            .unwrap(),
        ControlFlow::Break(42)
    );
    let before = reads.load(Ordering::Relaxed);
    check(
        selection.samples(Time::from_ticks(100)).unwrap(),
        &["1", "11", "1"],
        Some(7),
    );
    assert!(
        reads.load(Ordering::Relaxed) > before,
        "Break must discard provisional replay"
    );
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = selection.visit_samples(Time::from_ticks(100), |_| -> ControlFlow<()> {
            panic!("consumer")
        });
    }));
    assert!(panic.is_err());
    let before = reads.load(Ordering::Relaxed);
    check(
        selection.samples(Time::from_ticks(100)).unwrap(),
        &["1", "11", "1"],
        Some(7),
    );
    assert!(
        reads.load(Ordering::Relaxed) > before,
        "panic must discard provisional replay"
    );
}

#[test]
fn wdb_read_errors_and_selected_prefixes() {
    use crate::{Time, TimeRange};
    use std::ops::ControlFlow;
    let mut unmarked = recorded(6, &[(0, 0, &[1])]);
    put64(&mut unmarked, 0xd0 + 48, 0);
    put64(&mut unmarked, 0xd0 + 56, 0);
    let mut wave = crate::open_bytes("unmarked.wdb", unmarked.into()).unwrap();
    let s = wave.hierarchy().signal("top.flag").unwrap();
    assert!(matches!(
        wave.sample(s, Time::ZERO).unwrap(),
        crate::Sample::Value { .. }
    ));
    let mut corrupt = recorded(6, &[(0, 0, &[0])]);
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    let mut wave = crate::open_bytes("bad.wdb", corrupt.into()).unwrap();
    let s = wave.hierarchy().signal("top.flag").unwrap();
    assert!(matches!(
        wave.sample(s, Time::ZERO),
        Err(Error::Malformed { .. })
    ));
    let mut backwards = crate::open_bytes(
        "bad.wdb",
        recorded(6, &[(0, 0, &[0]), (7, 0, &[1]), (3, 0, &[0])]).into(),
    )
    .unwrap();
    let s = backwards.hierarchy().signal("top.flag").unwrap();
    assert!(matches!(
        backwards.sample(s, Time::from_ticks(10)),
        Err(Error::Malformed { .. })
    ));
    let mut unrelated =
        crate::open_bytes("other.wdb", recorded(6, &[(0, 1, &[0])]).into()).unwrap();
    let s = unrelated.hierarchy().signal("top.flag").unwrap();
    let mut count = 0;
    let _ = unrelated
        .select(&[s])
        .unwrap()
        .scan(TimeRange::closed(Time::ZERO, Time::from_ticks(10)), |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn wdb_mismatched_declaration_is_checked_for_each_object() {
    for marked in [0, 1] {
        let mut bytes = populated(6);
        let debug_len = u64::from_le_bytes(bytes[0x208..0x210].try_into().unwrap()) as usize;
        let instances = u32::from_le_bytes(bytes[0x424..0x428].try_into().unwrap()) as usize;
        let declarations = u32::from_le_bytes(bytes[0x428..0x42c].try_into().unwrap()) as usize;
        let mut second = bytes[0x400 + instances..0x400 + instances + 56].to_vec();
        put64(&mut second, 0, 8);
        bytes[0x400 + debug_len..0x400 + debug_len + 56].copy_from_slice(&second);
        put64(&mut bytes, 0x208, (debug_len + 56) as u64);
        put32(&mut bytes, 0x400 + 108, 2);
        put32(&mut bytes, 0x400 + declarations + 16, 2);
        put64(&mut bytes, 0x220, marked);
        put64(&mut bytes, 0x228, marked);
        assert!(
            matches!(
                crate::open_bytes("mismatch.wdb", bytes.into()),
                Err(Error::Malformed { .. })
            ),
            "marked object {marked}"
        );
    }
}

fn structure(data: Vec<u8>) -> crate::Result<debug::Debug> {
    metadata(Box::new(Cursor::new(data)))
}

fn metadata(input: Box<dyn crate::backends::Input>) -> crate::Result<debug::Debug> {
    let mut source = Source::new(input)?;
    let container = Container::read(&mut source)?;
    let types = types::read(&mut source, container.rtti)?;
    debug::read(&mut source, &container, types.len())
}

#[test]
#[ignore = "requires the complete installed WDB corpus"]
fn wdb_metadata_corpus() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/wdb");
    let mut directories = std::fs::read_dir(root)
        .unwrap()
        .map(|d| d.unwrap().path())
        .collect::<Vec<_>>();
    directories.sort();
    let mut versions = [0; 3];
    let mut unsupported = 0;
    let mut failures = Vec::new();
    for directory in &directories {
        let file = std::fs::File::open(directory.join("waveform.wdb")).unwrap();
        let name = directory.file_name().unwrap().to_string_lossy();
        match metadata(Box::new(std::io::BufReader::new(file))) {
            Ok(debug) => {
                versions[(debug.version - 4) as usize] += 1;
                eprintln!(
                    "PASS {name}: DBG00{} objects={}",
                    debug.version,
                    debug.objects.len()
                );
            }
            Err(Error::UnsupportedFeature { message, .. }) => {
                unsupported += 1;
                eprintln!("UNSUPPORTED {name}: {message}");
            }
            Err(error) => {
                failures.push(name.into_owned());
                eprintln!("FAIL {}: {error}", failures.last().unwrap());
            }
        }
    }
    eprintln!(
        "WDB metadata only: DBG004/005/006={versions:?}, unsupported={unsupported}, failed={}",
        failures.len()
    );
    assert!(!directories.is_empty());
    assert!(versions.iter().all(|&n| n > 0));
    assert!(failures.is_empty(), "failed inputs: {failures:?}");
}

#[test]
fn wdb_versions_and_directory_order() {
    for version in 4..=6 {
        let mut data = empty(version);
        let debug = structure(data.clone()).unwrap();
        assert_eq!(debug.version, version);
        assert_eq!(debug.precision, -12);
        assert!(debug.objects.is_empty());
        // Directories are identified by name, not by pointer position.
        data[0x48..0x60].rotate_left(8);
        assert_eq!(structure(data).unwrap().version, version);
    }
}

#[test]
fn wdb_truncation_and_bad_structure_are_errors() {
    let bytes = empty(6);
    for len in 0..0x2f8 {
        assert!(structure(bytes[..len].to_vec()).is_err(), "length {len}");
    }
    for (at, value) in [(0x48, u64::MAX), (0x50, 0x110), (0xc8 + 48, u64::MAX)] {
        let mut data = bytes.clone();
        put64(&mut data, at, value);
        assert!(matches!(structure(data), Err(Error::Malformed { .. })));
    }
    let mut data = bytes;
    put32(&mut data, 0x240 + 28 + 2 * 4, u32::MAX);
    assert!(matches!(structure(data), Err(Error::Malformed { .. })));
}

#[test]
fn wdb_unknown_version_is_a_support_limit() {
    assert!(matches!(
        structure(empty(7)),
        Err(Error::UnsupportedFeature { .. })
    ));
}

#[test]
fn wdb_nonempty_metadata_and_invalid_references() {
    for version in 4..=6 {
        let bytes = populated(version);
        let mut source = Source::new(Box::new(Cursor::new(bytes.clone()))).unwrap();
        let container = Container::read(&mut source).unwrap();
        assert_eq!(container.end, 10);
        assert_eq!(container.logged.len(), 1);
        assert_eq!(container.logged[0], 0..=0);
        assert_eq!(container.arenas.len(), 1);
        assert_eq!(container.arenas[0][0].offset, 0x14c0);
        let debug = structure(bytes.clone()).unwrap();
        assert_eq!(debug.scopes[1].parent, Some(0));
        assert_eq!(debug.scopes[1].name, "top");
        assert_eq!(debug.units[1].kind, 9);
        assert_eq!(debug.decls[0].name, "flag");
        assert_eq!(debug.objects[0].scope, 1);
        assert!(debug.objects[0].logged);
        for (at, value) in [(0x1000 + 8, u64::MAX), (0x1000, 0x1000), (0x220 + 8, 1)] {
            let mut bad = bytes.clone();
            put64(&mut bad, at, value);
            assert!(matches!(structure(bad), Err(Error::Malformed { .. })));
        }
        for (at, value) in [
            (0x1000 + 0x4b8, 101),
            (0x400 + 184 + 36 + 4, 1),
            (0x400 + 100 + 12, 2),
        ] {
            let mut bad = bytes.clone();
            put32(&mut bad, at, value);
            assert!(matches!(structure(bad), Err(Error::Malformed { .. })));
        }
    }
}

#[test]
fn wdb_type_tag_uses_the_qualified_prefix() {
    let bytes = populated(6);
    assert!(structure(bytes.clone()).is_ok());
    for tag in [0x103, 0xa000_0103, 0xb000_0003] {
        let mut bad = bytes.clone();
        put32(&mut bad, 0x300 + 44, tag);
        assert!(
            matches!(structure(bad), Err(Error::UnsupportedFeature { .. })),
            "tag {tag:#x}"
        );
    }
}

#[test]
#[ignore = "requires installed published WDB fixtures"]
fn wdb_published_metadata() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/wdb");
    for name in [
        "wdb0305-enum-core",
        "wdb0306-enum-xz",
        "wdb0307-enum-wide-constants",
        "wdb0300-opb-s",
    ] {
        let data = std::fs::read(root.join(name).join("waveform.wdb")).unwrap();
        let debug = structure(data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!debug.objects.is_empty());
        assert!(debug.objects.iter().any(|o| o.logged));
        eprintln!(
            "{name}: DBG00{} scopes={} declarations={} objects={}",
            debug.version,
            debug.scopes.len(),
            debug.decls.len(),
            debug.objects.len()
        );
    }
}
