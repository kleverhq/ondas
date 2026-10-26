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
