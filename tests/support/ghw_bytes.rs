// Independent test-only bytes: no production decoder/encoder helpers or producer.
pub fn recording(noise: usize) -> Vec<u8> {
    let strings = [
        "std_ulogic",
        "'U'",
        "'X'",
        "'0'",
        "'1'",
        "'Z'",
        "'W'",
        "'L'",
        "'H'",
        "'-'",
        "integer",
        "vector",
        "top",
        "wide",
        "alias",
        "reversed",
        "control",
        "noise",
    ];
    let mut bytes = b"GHDLwave\n\x10\0\x01\x01\x04\x01\0STR\0\0\0\0\0".to_vec();
    bytes.extend((strings.len() as u32).to_le_bytes());
    bytes.extend((strings.iter().map(|s| s.len()).sum::<usize>() as u32).to_le_bytes());
    for string in strings {
        bytes.extend(string.as_bytes());
        bytes.push(0);
    }
    bytes.extend(b"EOS\0TYP\0\0\0\0\0\x04\0\0\0");
    // E8 std_ulogic, I32 integer, unbounded 1-D vector, and its 2 downto 0 bound.
    bytes.extend([
        23, 1, 9, 2, 3, 4, 5, 6, 7, 8, 9, 10, 25, 11, 31, 12, 1, 1, 2, 35, 0, 3, 153, 2, 0, 0,
    ]);
    bytes.extend(b"WKT\0\0\0\0\0\x03\x01\0HIE\0\0\0\0\0");
    // WKT marks type 1 as std_ulogic. HIE declares top and five variables.
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(5u32.to_le_bytes());
    bytes.extend(5u32.to_le_bytes());
    bytes.extend([
        3, 13, 16, 14, 4, 3, 1, 4, 16, 15, 4, 3, 1, 4, 16, 16, 4, 4, 1, 3, 16, 17, 1, 4, 16, 18, 1,
        5, 15, 0,
    ]);
    bytes.extend(b"EOH\0SNP\0\0\0\0\0");
    bytes.extend(0i64.to_le_bytes());
    // Active IDs are 1,3,4,5; the whole recipe is [3,1,4] = 100.
    bytes.extend([2, 3, 2, 2]);
    bytes.extend(b"ESN\0CYC\0");
    bytes.extend(1i64.to_le_bytes());
    bytes.extend([3, 3, 0]); // low bit becomes 1: 101.
    if noise != 0 {
        for i in 0..noise {
            bytes.extend([1, 4, if i % 2 == 0 { 3 } else { 2 }, 0]);
        }
    } else {
        bytes.extend([1, 2, 2, 0, 0, 2, 3, 0]); // tick 2 excursion cancels.
        bytes.extend([1, 1, 3, 0]); // tick 3: middle bit -> 111.
        bytes.extend([1, 3, 2, 0]); // tick 4: low bit -> 110.
    }
    bytes.push(127);
    bytes.extend(b"ECY\0");
    bytes
}

/// Add independent static sources so selected vectors exercise selective reads.
pub fn padded_recording(noise: usize) -> Vec<u8> {
    let mut bytes = recording(noise);
    let strings_end = bytes.windows(4).position(|b| b == b"EOS\0").unwrap();
    let mut names = Vec::new();
    for i in 0..8 {
        names.extend(format!("pad{i}\0").as_bytes());
    }
    let added_size = names.len() as u32 - 8; // Exclude each string terminator.
    bytes.splice(strings_end..strings_end, names);
    let count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    bytes[24..28].copy_from_slice(&(count + 8).to_le_bytes());
    let size = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
    bytes[28..32].copy_from_slice(&(size + added_size).to_le_bytes());
    let hie = bytes.windows(4).position(|b| b == b"HIE\0").unwrap();
    for field in [hie + 12, hie + 16] {
        let count = u32::from_le_bytes(bytes[field..field + 4].try_into().unwrap());
        bytes[field..field + 4].copy_from_slice(&(count + 8).to_le_bytes());
    }
    let end = bytes.windows(4).position(|b| b == b"EOH\0").unwrap() - 2;
    assert_eq!(&bytes[end..end + 2], &[15, 0]); // Scope pop and hierarchy end.
    // Signal tag, string ID, std_ulogic type ID, and new source scalar ID.
    let declarations = (0..8)
        .flat_map(|i| [16, 19 + i, 1, 6 + i])
        .collect::<Vec<_>>();
    bytes.splice(end..end, declarations);
    let snapshot_end = bytes.windows(4).position(|b| b == b"ESN\0").unwrap();
    bytes.splice(snapshot_end..snapshot_end, [2; 8]); // std_ulogic ordinal for zero.
    bytes
}

pub fn real_recording(big: bool, version: u8, payload: u64) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, n: u32, big: bool) {
        bytes.extend(if big {
            n.to_be_bytes()
        } else {
            n.to_le_bytes()
        });
    }
    fn wide(bytes: &mut Vec<u8>, n: u64, big: bool) {
        bytes.extend(if big {
            n.to_be_bytes()
        } else {
            n.to_le_bytes()
        });
    }
    let mut bytes = b"GHDLwave\n\x10\0\0\x01\x04\x01\0STR\0\0\0\0\0".to_vec();
    bytes[11] = version;
    bytes[12] = if big { 2 } else { 1 };
    word(&mut bytes, 3, big);
    word(&mut bytes, 8, big);
    bytes.extend(b"real\0top\0r\0EOS\0TYP\0\0\0\0\0");
    word(&mut bytes, 1, big);
    bytes.extend([27, 1, 0]);
    bytes.extend(b"HIE\0\0\0\0\0");
    for _ in 0..3 {
        word(&mut bytes, 1, big);
    }
    bytes.extend([3, 2, 16, 3, 1, 1, 15, 0]);
    bytes.extend(b"EOH\0SNP\0\0\0\0\0");
    wide(&mut bytes, 3, big);
    wide(&mut bytes, 0x8000000000000000, big);
    bytes.extend(b"ESN\0CYC\0");
    wide(&mut bytes, 4, big);
    bytes.push(1);
    wide(&mut bytes, payload, big);
    bytes.extend([0, 127]);
    bytes.extend(b"ECY\0");
    bytes
}
