use super::{
    container::Span,
    malformed, unsupported,
    wire::{Cursor, Source, at},
};
use crate::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Range {
    pub left: i64,
    pub right: i64,
    pub direction: i32,
}

impl Range {
    fn read(c: &mut Cursor<'_>) -> Result<Self> {
        let left = i64::from(c.i32()?);
        let right = i64::from(c.i32()?);
        let direction = c.i32()?;
        if !matches!(direction, -2 | -1 | 1) {
            return Err(unsupported("range direction"));
        }
        Ok(Self {
            left,
            right,
            direction,
        })
    }

    pub fn len(self) -> Result<usize> {
        if self.direction == -2 {
            return Err(unsupported("unconstrained value dimension"));
        }
        let n = if self.direction == 1 {
            i128::from(self.right) - i128::from(self.left) + 1
        } else {
            i128::from(self.left) - i128::from(self.right) + 1
        };
        usize::try_from(n.max(0)).map_err(|_| malformed("dimension size overflow"))
    }
}

fn ranges(c: &mut Cursor<'_>) -> Result<Vec<Range>> {
    let n = c.count(12)?;
    (0..n).map(|_| Range::read(c)).collect()
}

pub(super) struct Field {
    pub name: String,
    pub ty: usize,
    pub ranges: Vec<Range>,
}

pub(super) enum Body {
    Enum {
        class: u32,
        literals: Vec<Vec<u8>>,
        bytes: usize,
        variant: u32,
    },
    Values {
        base: usize,
        entries: Vec<(String, Vec<u8>)>,
        ranges: Vec<Range>,
    },
    Integer {
        low: i32,
        high: i32,
    },
    Real,
    Alias {
        target: usize,
        ranges: Vec<Range>,
    },
    Physical,
    Array {
        layout: u16,
        elem: usize,
        dims: usize,
        ranges: Vec<Range>,
    },
    Record {
        layout: u16,
        fields: Vec<Field>,
    },
    Unsupported(u8),
}

pub(super) struct Type {
    pub name: String,
    pub origin: u32,
    pub body: Body,
}

impl Type {
    pub fn verilog(&self) -> bool {
        self.origin & 1 != 0
    }
}

fn layout(c: &mut Cursor<'_>, high: u16) -> Result<u16> {
    let layout = c.u16()?;
    if !matches!(layout, 1 | 2 | 3 | 6) || c.u16()? != high {
        return Err(unsupported("type packing layout"));
    }
    Ok(layout)
}

fn fields(c: &mut Cursor<'_>, class: bool) -> Result<Vec<Field>> {
    let n = c.count(9)?;
    let mut fields = Vec::new();
    for _ in 0..n {
        let name = c.string()?;
        let ty = c.u32()? as usize;
        fields.push(Field {
            name,
            ty,
            ranges: ranges(c)?,
        });
        if class {
            c.u32()?;
        }
    }
    Ok(fields)
}

fn read_type(kind: u8, bytes: &[u8]) -> Result<Type> {
    let mut c = Cursor::new(bytes);
    let name = c.string()?;
    let origin = c.u32()?;
    if !matches!(origin, 1 | 2 | 5 | 10 | 13) {
        return Err(unsupported(format!("type origin {origin:#x}")));
    }
    let body = match kind {
        3 => {
            let variant = c.u32()?;
            let class = c.u32()?;
            let n = c.count(1)?;
            // VHDL CHARACTER literals can contain one non-UTF-8 character byte.
            // Preserve the source bytes until the value/type adapter interprets them.
            let literals = (0..n)
                .map(|_| c.terminated().map(<[u8]>::to_vec))
                .collect::<Result<_>>()?;
            let bytes = match c.u32()? {
                0 | 1 => 1,
                4 => 4,
                _ => return Err(unsupported("enumeration storage size")),
            };
            Body::Enum {
                class,
                literals,
                bytes,
                variant,
            }
        }
        4 => {
            let base = c.u32()? as usize;
            let n = c.count(1)?;
            let size = c.u32()? as usize;
            if size == 0 || !size.is_multiple_of(8) {
                return Err(unsupported("named enumeration cell size"));
            }
            let mut entries = Vec::new();
            for _ in 0..n {
                entries.push((c.string()?, c.take(size)?.to_vec()));
            }
            Body::Values {
                base,
                entries,
                ranges: ranges(&mut c)?,
            }
        }
        5 => {
            let low = c.i32()?;
            let high = c.i32()?;
            c.expect(1)?;
            Body::Integer { low, high }
        }
        6 => {
            c.u32()?;
            c.take(16)?;
            if !matches!(c.u32()?, 0 | 1) {
                return Err(unsupported("real trailer"));
            }
            Body::Real
        }
        7 => Body::Alias {
            target: c.u32()? as usize,
            ranges: ranges(&mut c)?,
        },
        8 | 12 => {
            c.take(12)?;
            Body::Unsupported(kind)
        }
        13 => {
            let n = c.count(9)?;
            for _ in 0..n {
                c.string()?;
                c.u64()?;
            }
            Body::Physical
        }
        16 => {
            let layout = layout(&mut c, 0xa0)?;
            let elem = c.u32()? as usize;
            let dims = c.count(4)?;
            c.take(4 * dims)?;
            let ranges = ranges(&mut c)?;
            c.i32()?;
            Body::Array {
                layout,
                elem,
                dims,
                ranges,
            }
        }
        17 => {
            let layout = layout(&mut c, 0xb)?;
            let fields = fields(&mut c, false)?;
            c.u32()?;
            Body::Record { layout, fields }
        }
        19 | 20 => {
            c.take(8)?;
            Body::Unsupported(kind)
        }
        21 => {
            c.take(12)?;
            Body::Unsupported(kind)
        }
        11 => {
            c.expect(0)?;
            Body::Unsupported(kind)
        }
        23 => {
            c.take(8)?;
            fields(&mut c, true)?;
            Body::Unsupported(kind)
        }
        24 => Body::Unsupported(kind),
        _ => return Err(unsupported(format!("RTTI type kind {kind:#x}"))),
    };
    c.finish()?;
    Ok(Type { name, origin, body })
}

pub(super) fn read(source: &mut Source, span: Span) -> Result<Vec<Type>> {
    let bytes = source.bytes(span.offset, span.len)?;
    if Cursor::new(&bytes).string()? != "Xilinx ISim TYPE FILE 001" {
        return Err(unsupported("RTTI signature"));
    }
    let mut c = at(&bytes, 32)?;
    let n = c.u32()? as usize;
    let end = c.u32()? as usize;
    if end < 40 || end > bytes.len() || n > (end - 40) / 8 || n > (bytes.len() - end) / 8 {
        return Err(malformed("RTTI entry/offset table size"));
    }
    let mut offsets = Cursor::new(&bytes[end..]);
    let mut entries = at(&bytes[..end], 40)?;
    let mut types = Vec::new();
    for _ in 0..n {
        if offsets.u64()? != (entries.at + 40) as u64 {
            return Err(malformed("RTTI entry offset mismatch"));
        }
        let len = entries.u32()? as usize;
        let tag = entries.u32()?;
        // The qualified RTTI tag has a layout prefix and a low-byte kind.
        if tag & 0xffff_ff00 != 0xa000_0000 {
            return Err(unsupported(format!("RTTI type tag {tag:#x}")));
        }
        let len = len
            .checked_sub(8)
            .ok_or_else(|| malformed("short RTTI entry"))?;
        types.push(read_type((tag & 0xff) as u8, entries.take(len)?)?);
    }
    entries.finish()?;
    offsets.finish()?;
    for ty in &types {
        let mut references = Vec::new();
        match &ty.body {
            Body::Alias { target, .. } => references.push(*target),
            Body::Array { elem, .. } => references.push(*elem),
            Body::Values { base, .. } => references.push(*base),
            Body::Record { fields, .. } => references.extend(fields.iter().map(|f| f.ty)),
            _ => {}
        }
        if references.iter().any(|&i| i >= types.len()) {
            return Err(malformed("RTTI type reference outside table"));
        }
    }
    Ok(types)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(entries: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = vec![0; 40];
        bytes[..25].copy_from_slice(b"Xilinx ISim TYPE FILE 001");
        let mut offsets = Vec::new();
        for (kind, body) in entries {
            offsets.push(bytes.len() as u64);
            bytes.extend_from_slice(&(8 + body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(0xa000_0000 | u32::from(*kind)).to_le_bytes());
            bytes.extend_from_slice(body);
        }
        let end = bytes.len() as u32;
        bytes[32..36].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        bytes[36..40].copy_from_slice(&end.to_le_bytes());
        for offset in offsets {
            bytes.extend_from_slice(&offset.to_le_bytes());
        }
        bytes
    }

    fn read_table(bytes: Vec<u8>) -> Result<Vec<Type>> {
        let span = Span {
            offset: 0,
            len: bytes.len() as u64,
        };
        let mut source = Source::new(Box::new(std::io::Cursor::new(bytes)))?;
        read(&mut source, span)
    }

    #[test]
    fn wdb_rtti_references_and_table_boundary() {
        let mut base = b"bit\0".to_vec();
        for word in [1_u32, 1, 1, 4] {
            base.extend_from_slice(&word.to_le_bytes());
        }
        for literal in *b"0100" {
            base.extend_from_slice(&[literal, 0]);
        }
        base.extend_from_slice(&0_u32.to_le_bytes());
        for kind in [4, 7, 16, 17] {
            for target in [1_u32, 2] {
                let mut body = b"ref\0".to_vec();
                body.extend_from_slice(&1_u32.to_le_bytes());
                let words = match kind {
                    4 => vec![target, 1, 8],
                    7 => vec![target, 0],
                    16 => vec![
                        0x00a0_0003,
                        target,
                        1,
                        1,
                        1,
                        0,
                        0,
                        (-1_i32) as u32,
                        (-99_i32) as u32,
                    ],
                    17 => vec![0x000b_0003, 1],
                    _ => unreachable!(),
                };
                for word in words {
                    body.extend_from_slice(&word.to_le_bytes());
                }
                if kind == 4 {
                    body.extend_from_slice(b"READY\0");
                    body.extend_from_slice(&0_u64.to_le_bytes());
                    body.extend_from_slice(&0_u32.to_le_bytes());
                } else if kind == 17 {
                    body.extend_from_slice(b"field\0");
                    for word in [target, 0, 0] {
                        body.extend_from_slice(&word.to_le_bytes());
                    }
                }
                let bytes = table(&[(kind, body), (3, base.clone())]);
                let result = read_table(bytes.clone());
                if target == 2 {
                    assert!(
                        matches!(result, Err(crate::Error::Malformed { .. })),
                        "kind {kind}"
                    );
                } else {
                    assert_eq!(result.unwrap().len(), 2, "kind {kind}");
                    let mut trailing = bytes;
                    trailing.push(0);
                    assert!(
                        read_table(trailing).is_err(),
                        "kind {kind} accepted extra RTTI bytes"
                    );
                }
            }
        }
    }

    #[test]
    fn wdb_named_cells_preserve_all_bytes_and_reject_truncation() {
        for size in [0_u32, 12] {
            let mut bytes = b"state\0".to_vec();
            for word in [1_u32, 0, 1, size] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            bytes.extend_from_slice(b"never_selected\0");
            bytes.extend_from_slice(&[0; 16]);
            assert!(read_type(4, &bytes).is_err(), "size={size}");
        }
        for size in [8_u32, 16, 24, 40] {
            let mut bytes = b"state\0".to_vec();
            for word in [1_u32, 0, 1, size] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            bytes.extend_from_slice(b"never_selected\0");
            let key: Vec<_> = (0..size as u8).collect();
            bytes.extend_from_slice(&key);
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            let ty = read_type(4, &bytes).unwrap();
            let Body::Values { entries, .. } = ty.body else {
                panic!("named values")
            };
            assert_eq!(entries, [("never_selected".into(), key)]);
            for end in 0..bytes.len() {
                assert!(
                    read_type(4, &bytes[..end]).is_err(),
                    "size={size}, end={end}"
                );
            }
        }
    }

    #[test]
    fn wdb_dimension_arithmetic_does_not_wrap() {
        assert!(
            Range {
                left: i64::MIN,
                right: i64::MAX,
                direction: 1
            }
            .len()
            .is_err()
        );
        assert_eq!(
            Range {
                left: 7,
                right: 0,
                direction: -1
            }
            .len()
            .unwrap(),
            8
        );
        assert_eq!(
            Range {
                left: 0,
                right: 7,
                direction: -1
            }
            .len()
            .unwrap(),
            0
        );
        assert!(
            Range {
                left: 0,
                right: 0,
                direction: -2
            }
            .len()
            .is_err()
        );
    }

    #[test]
    fn wdb_character_literal_bytes_do_not_require_utf8_names() {
        let mut bytes = b"CHARACTER\0".to_vec();
        for word in [2_u32, 2, 4, 2] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(b"'A'\0'\xa0'\0");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let ty = read_type(3, &bytes).unwrap();
        let Body::Enum { literals, .. } = ty.body else {
            panic!("enum")
        };
        assert_eq!(literals, [b"'A'".to_vec(), b"'\xa0'".to_vec()]);
        bytes[0] = 0xff;
        assert!(matches!(
            read_type(3, &bytes),
            Err(crate::Error::UnsupportedFeature { .. })
        ));
    }
}
