use super::{
    malformed,
    types::{Body, Range, Type},
    unsupported,
};
use crate::{BitsRef, Encoding, LogicDomain, Result, Signedness, ValueRef};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Codec {
    Pairs {
        width: u32,
    },
    Integer {
        bytes: usize,
        width: u32,
    },
    Ordinal {
        bytes: usize,
        width: u32,
        count: usize,
        alphabet: Option<Vec<u8>>,
    },
    Vector {
        count: usize,
        bytes: usize,
        alphabet: Vec<u8>,
    },
    Real,
    Unsupported,
}

impl Codec {
    pub fn encoding(&self) -> Encoding {
        match self {
            Self::Pairs { width } | Self::Integer { width, .. } | Self::Ordinal { width, .. } => {
                Encoding::Bits { width: *width }
            }
            Self::Vector { count, .. } => Encoding::Bits {
                width: *count as u32,
            },
            Self::Real => Encoding::Real,
            Self::Unsupported => Encoding::Unsupported,
        }
    }

    pub fn decode<'a>(
        &self,
        data: &[u8],
        shift: usize,
        digits: &'a mut Vec<u8>,
    ) -> Result<ValueRef<'a>> {
        digits.clear();
        let ordinal = |data: &[u8], bytes: usize| -> Result<u64> {
            let raw = data
                .get(..bytes)
                .ok_or_else(|| malformed("short scalar value"))?;
            let mut value = [0; 8];
            if bytes > value.len() {
                return Err(malformed("scalar storage width"));
            }
            value[..bytes].copy_from_slice(raw);
            Ok(u64::from_le_bytes(value))
        };
        let binary = |digits: &mut Vec<u8>, value: u64, width: u32| {
            for bit in (0..width).rev() {
                digits.push(b'0' + ((value >> bit) & 1) as u8);
            }
        };
        match self {
            Self::Pairs { width } => pairs(data, shift, *width, digits)?,
            Self::Integer { bytes, width } => binary(digits, ordinal(data, *bytes)?, *width),
            Self::Ordinal {
                bytes,
                width,
                count,
                alphabet,
            } => {
                let value = ordinal(data, *bytes)?;
                if value >= *count as u64 {
                    return Err(malformed("enumeration literal index"));
                }
                if let Some(alphabet) = alphabet {
                    digits.push(
                        *alphabet
                            .get(value as usize)
                            .ok_or_else(|| malformed("logic literal index"))?,
                    );
                } else {
                    binary(digits, value, *width);
                }
            }
            Self::Vector {
                count,
                bytes,
                alphabet,
            } => {
                for i in 0..*count {
                    let value = ordinal(
                        data.get(i * bytes..)
                            .ok_or_else(|| malformed("short vector"))?,
                        *bytes,
                    )?;
                    digits.push(
                        *alphabet
                            .get(value as usize)
                            .ok_or_else(|| malformed("vector logic literal index"))?,
                    );
                }
            }
            Self::Real => return Ok(ValueRef::Real(f64::from_bits(ordinal(data, 8)?))),
            Self::Unsupported => return Err(unsupported("value representation")),
        }
        Ok(ValueRef::Bits(BitsRef::from_validated_ascii(digits)))
    }
}

pub(super) struct Node {
    pub ty: usize,
    pub units: usize,
    pub align: usize,
    pub codec: Codec,
    pub shape: Shape,
    pub domain: Option<LogicDomain>,
    pub signedness: Option<Signedness>,
}

impl Node {
    pub fn unsupported(ty: usize) -> Self {
        Self {
            ty,
            units: 0,
            align: 1,
            codec: Codec::Unsupported,
            shape: Shape::Leaf,
            domain: None,
            signedness: None,
        }
    }
}

pub(super) enum Shape {
    Leaf,
    Array {
        ranges: Vec<Range>,
        child: Box<Node>,
        stride: usize,
        expand: bool,
    },
    Record(Vec<(String, usize, Node)>),
}

fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or_else(|| malformed("type size overflow"))
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| malformed("type size overflow"))
}
fn round(n: usize, align: usize) -> Result<usize> {
    mul(n.div_ceil(align), align)
}

fn dimensions(ty_ranges: &[Range], dims: usize, rs: &mut &[Range]) -> Result<Vec<Range>> {
    if ty_ranges.len() >= dims && dims > 0 && ty_ranges[0].direction != -2 {
        return Ok(ty_ranges[..dims].to_vec());
    }
    let range = rs
        .get(..dims)
        .ok_or_else(|| malformed("missing type dimensions"))?
        .to_vec();
    *rs = &rs[dims..];
    Ok(range)
}

fn alphabet(literals: &[Vec<u8>]) -> Option<Vec<u8>> {
    literals
        .iter()
        .map(|s| match s.as_slice() {
            [c] | [b'\'', c, b'\''] if b"01xXzZuUwWlLhH-".contains(c) => {
                Some(c.to_ascii_lowercase())
            }
            _ => None,
        })
        .collect()
}

pub(super) fn node(types: &[Type], ty: usize, rs: &[Range]) -> Result<Node> {
    let mut ranges = rs;
    build(types, ty, &mut ranges, &mut Vec::new())
}

fn build(types: &[Type], ty: usize, rs: &mut &[Range], path: &mut Vec<usize>) -> Result<Node> {
    if path.contains(&ty) {
        return Err(malformed("cyclic value type"));
    }
    // A finite source can still request an excessively deep owned value tree.
    if path.len() == 128 {
        return Err(unsupported("value type nesting above 128"));
    }
    let t = &types[ty];
    path.push(ty);
    let sv = t.verilog();
    let mut n = Node::unsupported(ty);
    match &t.body {
        Body::Alias { target, ranges } => {
            n = if ranges.is_empty() {
                build(types, *target, rs, path)?
            } else {
                build(types, *target, &mut ranges.as_slice(), path)?
            };
            n.ty = ty;
        }
        Body::Values { base, ranges, .. } => {
            n = build(types, *base, &mut ranges.as_slice(), path)?;
            n.ty = ty;
            n.shape = Shape::Leaf;
        }
        Body::Enum {
            class,
            literals,
            bytes,
        } => {
            n.units = if sv { 1 } else { *bytes };
            n.align = n.units;
            let alphabet = if sv || matches!(class, 2 | 3) {
                alphabet(literals)
            } else {
                None
            };
            let width = if alphabet.is_some() {
                1
            } else {
                (usize::BITS - literals.len().saturating_sub(1).leading_zeros()).max(1)
            };
            n.domain = if let Some(a) = &alphabet {
                Some(if a.iter().any(|b| b"uwlh-".contains(b)) {
                    LogicDomain::NineState
                } else if a.iter().any(|b| b"xz".contains(b)) {
                    LogicDomain::FourState
                } else {
                    LogicDomain::TwoState
                })
            } else {
                Some(LogicDomain::TwoState)
            };
            n.codec = if sv {
                Codec::Pairs { width: 1 }
            } else {
                Codec::Ordinal {
                    bytes: *bytes,
                    width,
                    count: literals.len(),
                    alphabet,
                }
            };
        }
        Body::Integer => {
            n.units = 4;
            n.align = 4;
            n.codec = Codec::Integer {
                bytes: 4,
                width: 32,
            };
            n.signedness = Some(Signedness::Signed);
            n.domain = Some(LogicDomain::TwoState);
        }
        Body::Physical => {
            n.units = 8;
            n.align = 8;
            n.codec = Codec::Integer {
                bytes: 8,
                width: 64,
            };
            n.signedness = Some(Signedness::Signed);
            n.domain = Some(LogicDomain::TwoState);
        }
        Body::Real => {
            n.units = if sv { 32 } else { 8 };
            n.align = n.units;
            n.codec = Codec::Real;
        }
        Body::Array {
            layout,
            elem,
            dims,
            ranges,
        } => {
            let ranges = dimensions(if sv { ranges } else { &[] }, *dims, rs)?;
            let count = ranges.iter().try_fold(1, |n, r| mul(n, r.len()?))?;
            let child = build(types, *elem, rs, path)?;
            let stride = if sv {
                child.units
            } else {
                round(child.units, child.align)?
            };
            n.units = mul(count, stride)?;
            n.align = child.align;
            n.domain = child.domain;
            n.signedness = if matches!(t.origin, 5 | 13) {
                match t.name.as_str() {
                    "integer" | "int" | "shortint" | "byte" | "longint" => Some(Signedness::Signed),
                    "time" => Some(Signedness::Unsigned),
                    _ => None,
                }
            } else {
                None
            };
            n.codec = if n.units == 0 {
                Codec::Unsupported
            } else if sv && (*layout == 3 || matches!(child.codec, Codec::Pairs { .. })) {
                Codec::Pairs {
                    width: u32::try_from(n.units)
                        .map_err(|_| unsupported("bit width above u32"))?,
                }
            } else if !sv
                && let Codec::Ordinal {
                    bytes,
                    alphabet: Some(a),
                    ..
                } = &child.codec
            {
                if count > u32::MAX as usize {
                    return Err(unsupported("bit width above u32"));
                }
                Codec::Vector {
                    count,
                    bytes: *bytes,
                    alphabet: a.clone(),
                }
            } else {
                Codec::Unsupported
            };
            n.shape = Shape::Array {
                ranges,
                expand: n.codec == Codec::Unsupported || sv && *layout == 2 && child.units > 1,
                child: Box::new(child),
                stride,
            };
        }
        Body::Record { layout, fields } => {
            let mut members = Vec::new();
            for field in fields {
                let child = if !sv && !rs.is_empty() {
                    build(types, field.ty, rs, path)?
                } else {
                    let mut ranges = if matches!(types[field.ty].body, Body::Record { .. }) {
                        &[][..]
                    } else {
                        field.ranges.as_slice()
                    };
                    build(types, field.ty, &mut ranges, path)?
                };
                let size = if sv && *layout == 2 {
                    round(child.units, 32)?
                } else {
                    child.units
                };
                let at = if sv {
                    n.units
                } else {
                    round(n.units, child.align)?
                };
                n.units = if *layout == 6 {
                    n.units.max(size)
                } else {
                    add(at, size)?
                };
                members.push((field.name.clone(), at, child));
            }
            if sv {
                for (_, at, child) in &mut members {
                    let size = if *layout == 2 {
                        round(child.units, 32)?
                    } else {
                        child.units
                    };
                    *at = n.units
                        - if *layout == 6 {
                            child.units
                        } else {
                            add(*at, size)?
                        };
                }
                if matches!(layout, 3 | 6) && n.units > 0 {
                    n.codec = Codec::Pairs {
                        width: u32::try_from(n.units)
                            .map_err(|_| unsupported("record width above u32"))?,
                    };
                }
            } else {
                n.units = round(n.units, 8)?;
            }
            n.align = if sv { 1 } else { 8 };
            if members.iter().any(|(_, _, child)| child.units == 0) {
                n.units = 0;
                n.codec = Codec::Unsupported;
            } else {
                n.shape = Shape::Record(members);
            }
        }
        Body::Unsupported => {}
    }
    path.pop();
    Ok(n)
}

pub(super) fn pairs(data: &[u8], shift: usize, width: u32, output: &mut Vec<u8>) -> Result<()> {
    let bits = add(shift, width as usize)?;
    if data.len() < mul(bits.div_ceil(32), 8)? {
        return Err(malformed("short word-pair value"));
    }
    output.clear();
    output
        .try_reserve(width as usize)
        .map_err(|_| malformed("cannot allocate bit value"))?;
    for bit in (shift..bits).rev() {
        let pair = &data[8 * (bit / 32)..];
        let a = u32::from_le_bytes(pair[..4].try_into().unwrap());
        let b = u32::from_le_bytes(pair[4..8].try_into().unwrap());
        let code = ((a >> (bit % 32)) & 1) | (((b >> (bit % 32)) & 1) << 1);
        output.push(b"01zx"[code as usize]);
    }
    Ok(())
}

pub(super) fn enumeration(
    types: &[Type],
    mut ty: usize,
    codec: &Codec,
) -> Result<Option<crate::hierarchy::EnumerationData>> {
    let mut visited = 0;
    while let Body::Alias { target, .. } = &types[ty].body {
        ty = *target;
        visited += 1;
        if visited > types.len() {
            return Err(malformed("cyclic enum alias"));
        }
    }
    let t = &types[ty];
    let mut variants = Vec::new();
    match (&t.body, codec) {
        (Body::Values { entries, .. }, Codec::Pairs { width }) => {
            for (label, raw) in entries {
                let mut digits = Vec::new();
                pairs(raw, 0, *width, &mut digits)?;
                variants.push((String::from_utf8(digits).unwrap(), label.clone()));
            }
        }
        (
            Body::Enum { literals, .. },
            Codec::Ordinal {
                width,
                alphabet: None,
                ..
            },
        ) => {
            for (ordinal, label) in literals.iter().enumerate() {
                let label = match std::str::from_utf8(label) {
                    Ok(label) => label.to_owned(),
                    Err(_) => match label.as_slice() {
                        [b'\'', value, b'\''] => format!("'{}'", char::from(*value)),
                        _ => return Err(unsupported("non-UTF-8 enum label")),
                    },
                };
                variants.push((
                    format!("{ordinal:0width$b}", width = *width as usize),
                    label,
                ));
            }
        }
        _ => return Ok(None),
    }
    Ok(crate::hierarchy::EnumerationData::for_encoding(
        (!t.name.is_empty()).then(|| t.name.clone()),
        variants,
        codec.encoding(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(value: ValueRef<'_>) -> String {
        let ValueRef::Bits(bits) = value else {
            panic!("expected bits")
        };
        bits.to_string()
    }

    #[test]
    fn wdb_value_cells_preserve_wide_and_shifted_bits() {
        let mut data = vec![0; 40];
        data[0] = 1;
        data[32] = 1;
        data[36] = 1;
        let mut digits = Vec::new();
        let value = Codec::Pairs { width: 129 }
            .decode(&data, 0, &mut digits)
            .unwrap();
        assert_eq!(bits(value), format!("x{}1", "0".repeat(127)));
        let mut pairs = Vec::new();
        for word in [0x8000_0000_u32, 0, u32::MAX, 1] {
            pairs.extend_from_slice(&word.to_le_bytes());
        }
        let value = Codec::Pairs { width: 33 }
            .decode(&pairs, 31, &mut digits)
            .unwrap();
        assert_eq!(bits(value), format!("{}x1", "1".repeat(31)));
        assert!(
            Codec::Pairs { width: 65 }
                .decode(&pairs, 0, &mut digits)
                .is_err()
        );
    }

    #[test]
    fn wdb_ordinals_are_checked_and_logic_literals_are_not_binary_indices() {
        let mut digits = Vec::new();
        let codec = Codec::Vector {
            count: 4,
            bytes: 1,
            alphabet: b"ux01zwlh-".to_vec(),
        };
        let value = codec.decode(&[0, 1, 2, 8], 0, &mut digits).unwrap();
        assert_eq!(bits(value), "ux0-");
        assert!(codec.decode(&[0, 1, 2, 9], 0, &mut digits).is_err());
        let codec = Codec::Ordinal {
            bytes: 4,
            width: 9,
            count: 300,
            alphabet: None,
        };
        let value = codec
            .decode(&299_u32.to_le_bytes(), 0, &mut digits)
            .unwrap();
        assert_eq!(bits(value), "100101011");
        assert!(
            codec
                .decode(&300_u32.to_le_bytes(), 0, &mut digits)
                .is_err()
        );
        let cyclic = [Type {
            name: String::new(),
            origin: 1,
            body: Body::Alias {
                target: 0,
                ranges: Vec::new(),
            },
        }];
        assert!(matches!(
            node(&cyclic, 0, &[]),
            Err(crate::Error::Malformed { .. })
        ));
        let character_enum = [Type {
            name: "custom".into(),
            origin: 2,
            body: Body::Enum {
                class: 4,
                literals: vec![b"'0'".to_vec(), b"'X'".to_vec()],
                bytes: 1,
            },
        }];
        let n = node(&character_enum, 0, &[]).unwrap();
        assert_eq!(bits(n.codec.decode(&[1], 0, &mut digits).unwrap()), "1");
        assert!(enumeration(&character_enum, 0, &n.codec).unwrap().is_some());
    }
}
