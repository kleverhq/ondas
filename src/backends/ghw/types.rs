//! Source type graph and anonymous composite constraints.
use super::wire::Wire;
use crate::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Storage {
    B2,
    E8,
    I32,
    I64,
    F64,
    P32,
    P64,
}

impl Storage {
    pub(super) fn tag(self) -> u8 {
        match self {
            Self::B2 => 22,
            Self::E8 => 23,
            Self::I32 => 25,
            Self::I64 => 26,
            Self::F64 => 27,
            Self::P32 => 28,
            Self::P64 => 29,
        }
    }

    fn from_tag(tag: u8, wire: &Wire) -> Result<Self> {
        match tag {
            22 => Ok(Self::B2),
            23 => Ok(Self::E8),
            25 => Ok(Self::I32),
            26 => Ok(Self::I64),
            27 => Ok(Self::F64),
            28 => Ok(Self::P32),
            29 => Ok(Self::P64),
            _ => Err(wire.unsupported(format_args!("range/storage tag {tag}"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bound {
    Integer(i64),
    Real(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Range {
    pub(super) storage: Storage,
    pub(super) downto: bool,
    pub(super) left: Bound,
    pub(super) right: Bound,
}

impl Range {
    fn read(wire: &mut Wire) -> Result<Self> {
        let tag = wire.byte()?;
        let storage = Storage::from_tag(tag & 0x7f, wire)?;
        let mut bound = || -> Result<Bound> {
            Ok(match storage {
                Storage::B2 | Storage::E8 => Bound::Integer(i64::from(wire.byte()?)),
                Storage::I32 | Storage::P32 => Bound::Integer(wire.sleb(32)?),
                Storage::I64 | Storage::P64 => Bound::Integer(wire.sleb(64)?),
                Storage::F64 => Bound::Real(wire.real_bits()?),
            })
        };
        Ok(Self {
            storage,
            downto: tag & 0x80 != 0,
            left: bound()?,
            right: bound()?,
        })
    }

    pub(super) fn integer_bounds(&self) -> Option<(i64, i64)> {
        match (self.left, self.right) {
            (Bound::Integer(left), Bound::Integer(right)) => Some((left, right)),
            _ => None,
        }
    }

    fn extent(&self, wire: &Wire) -> Result<u128> {
        let (left, right) = self
            .integer_bounds()
            .ok_or_else(|| wire.malformed("real array index range"))?;
        let distance = if self.downto {
            i128::from(left) - i128::from(right)
        } else {
            i128::from(right) - i128::from(left)
        };
        Ok(if distance < 0 {
            0
        } else {
            distance as u128 + 1
        })
    }
}

#[derive(Clone, Debug)]
pub(super) enum Shape {
    Scalar {
        storage: Storage,
        literals: Vec<usize>,
        units: Vec<(usize, i64)>,
        wkt: u8,
    },
    Alias {
        base: usize,
        range: Option<Range>,
    },
    Array {
        element: usize,
        indices: Vec<usize>,
        bounds: Option<Vec<Range>>,
    },
    Record {
        fields: Vec<(usize, usize)>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Type {
    pub(super) name: usize,
    pub(super) base: Option<usize>,
    pub(super) shape: Shape,
    pub(super) scalars: Option<usize>,
}

pub(super) struct Types {
    pub(super) strings: Vec<String>,
    pub(super) nodes: Vec<Type>,
    source_ids: Vec<usize>,
}

impl Types {
    pub(super) fn read(wire: &mut Wire, strings: Vec<String>) -> Result<Self> {
        wire.marker(b"TYP\0")?;
        wire.zeros()?;
        let count = wire.count("type")?;
        if count as u64 > wire.len.saturating_sub(wire.offset) / 2 {
            return Err(wire.malformed("type count exceeds remaining input"));
        }
        let mut types = Self {
            strings,
            nodes: Vec::new(),
            source_ids: Vec::new(),
        };
        for _ in 0..count {
            let tag = wire.byte()?;
            if !matches!(tag, 22 | 23 | 25..=29 | 31 | 32 | 34 | 35 | 37..=39) {
                return Err(wire.unsupported(format_args!("type tag {tag}")));
            }
            let name = types.string_id(wire)?;
            let node = match tag {
                22 | 23 | 25..=29 => {
                    let storage = Storage::from_tag(tag, wire)?;
                    let mut literals = Vec::new();
                    let mut units = Vec::new();
                    if matches!(storage, Storage::B2 | Storage::E8) {
                        let n = wire.uleb()?;
                        if n == 0 || n > 256 || (storage == Storage::B2 && n != 2) {
                            return Err(
                                wire.malformed("enum literal count does not fit its storage")
                            );
                        }
                        for _ in 0..n {
                            literals.push(types.string_id(wire)?);
                        }
                    } else if matches!(storage, Storage::P32 | Storage::P64) && wire.version == 1 {
                        let n = types.list_count(wire, "physical unit", 2)?;
                        for _ in 0..n {
                            units.push((types.string_id(wire)?, wire.sleb(64)?));
                        }
                    }
                    types.push(
                        name,
                        None,
                        Shape::Scalar {
                            storage,
                            literals,
                            units,
                            wkt: 0,
                        },
                        wire,
                    )?
                }
                34 => {
                    let base = types.type_id(wire)?;
                    let range = Range::read(wire)?;
                    types.validate_range(base, &range, wire)?;
                    types.push(
                        name,
                        Some(base),
                        Shape::Alias {
                            base,
                            range: Some(range),
                        },
                        wire,
                    )?
                }
                31 => {
                    let element = types.type_id(wire)?;
                    let n = types.list_count(wire, "array dimension", 1)?;
                    if n == 0 {
                        return Err(wire.malformed("array without dimensions"));
                    }
                    let mut indices = Vec::new();
                    for _ in 0..n {
                        let index = types.type_id(wire)?;
                        match types.scalar(index).map(|s| s.0) {
                            Some(Storage::B2 | Storage::E8 | Storage::I32 | Storage::I64) => (),
                            _ => return Err(wire.malformed("non-discrete array index type")),
                        }
                        indices.push(index);
                    }
                    types.push(
                        name,
                        None,
                        Shape::Array {
                            element,
                            indices,
                            bounds: None,
                        },
                        wire,
                    )?
                }
                32 => {
                    let n = types.list_count(wire, "record field", 2)?;
                    let mut fields = Vec::new();
                    for _ in 0..n {
                        fields.push((types.string_id(wire)?, types.type_id(wire)?));
                    }
                    types.push(name, None, Shape::Record { fields }, wire)?
                }
                35 | 38 => {
                    let base = types.type_id(wire)?;
                    let resolved = types.effective(base);
                    let matches = matches!(
                        (&types.nodes[resolved].shape, tag),
                        (Shape::Array { .. }, 35) | (Shape::Record { .. }, 38)
                    );
                    if !matches {
                        return Err(wire.malformed("composite subtype base has wrong shape"));
                    }
                    let node = types.constrain(base, wire)?;
                    types.nodes[node].name = name;
                    node
                }
                37 | 39 => {
                    let base = types.type_id(wire)?;
                    let resolved = types.effective(base);
                    if !matches!(
                        (&types.nodes[resolved].shape, tag),
                        (Shape::Array { .. }, 37) | (Shape::Record { .. }, 39)
                    ) || types.nodes[base].scalars.is_some()
                    {
                        return Err(wire.malformed("unbounded alias base has wrong shape"));
                    }
                    types.push(name, Some(base), Shape::Alias { base, range: None }, wire)?
                }
                _ => return Err(wire.unsupported(format_args!("type tag {tag}"))),
            };
            types.source_ids.push(node);
        }
        if wire.byte()? != 0 {
            return Err(wire.malformed("missing type table terminator"));
        }
        Ok(types)
    }

    fn list_count(&self, wire: &mut Wire, context: &str, min_bytes: u64) -> Result<usize> {
        let count = wire.uleb()? as usize;
        if count as u64 > wire.len.saturating_sub(wire.offset) / min_bytes {
            return Err(wire.malformed(format_args!("{context} count exceeds remaining input")));
        }
        Ok(count)
    }

    pub(super) fn string_id(&self, wire: &mut Wire) -> Result<usize> {
        let id = wire.uleb()? as usize;
        if id >= self.strings.len() {
            return Err(wire.malformed("invalid string reference"));
        }
        Ok(id)
    }

    pub(super) fn type_id(&self, wire: &mut Wire) -> Result<usize> {
        let id = wire.uleb()? as usize;
        self.source_ids
            .get(id.wrapping_sub(1))
            .copied()
            .ok_or_else(|| wire.malformed("invalid or forward type reference"))
    }

    pub(super) fn effective(&self, mut id: usize) -> usize {
        while let Shape::Alias { base, .. } = &self.nodes[id].shape {
            id = *base;
        }
        id
    }

    pub(super) fn scalar(&self, id: usize) -> Option<(Storage, &[usize], u8)> {
        match &self.nodes[self.effective(id)].shape {
            Shape::Scalar {
                storage,
                literals,
                wkt,
                ..
            } => Some((*storage, literals, *wkt)),
            _ => None,
        }
    }

    pub(super) fn name(&self, mut id: usize) -> Option<&str> {
        loop {
            let node = &self.nodes[id];
            if node.name != 0 {
                return Some(&self.strings[node.name]);
            }
            id = node.base?;
        }
    }

    fn validate_range(&self, base: usize, range: &Range, wire: &Wire) -> Result<()> {
        let (storage, literals, _) = self
            .scalar(base)
            .ok_or_else(|| wire.malformed("range base is not scalar"))?;
        if storage != range.storage {
            return Err(wire.malformed("range storage differs from base type"));
        }
        if matches!(storage, Storage::B2 | Storage::E8) {
            let (left, right) = range
                .integer_bounds()
                .ok_or_else(|| wire.malformed("non-integral enum range"))?;
            if left < 0
                || right < 0
                || left as usize >= literals.len()
                || right as usize >= literals.len()
            {
                return Err(wire.malformed("enum range endpoint exceeds base literals"));
            }
        }
        Ok(())
    }

    fn push(
        &mut self,
        name: usize,
        base: Option<usize>,
        shape: Shape,
        wire: &Wire,
    ) -> Result<usize> {
        let scalars = match &shape {
            Shape::Scalar { .. } => Some(1),
            Shape::Alias { base, .. } => self.nodes[*base].scalars,
            Shape::Array { bounds: None, .. } => None,
            Shape::Array {
                element,
                bounds: Some(bounds),
                ..
            } => {
                let extents = bounds
                    .iter()
                    .map(|r| r.extent(wire))
                    .collect::<Result<Vec<_>>>()?;
                if extents.contains(&0) {
                    Some(0)
                } else if let Some(n) = self.nodes[*element].scalars {
                    let total = extents
                        .into_iter()
                        .try_fold(n as u128, |total, extent| total.checked_mul(extent))
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| wire.malformed("array scalar count overflow"))?;
                    Some(total)
                } else {
                    None
                }
            }
            Shape::Record { fields } => {
                let mut count = Some(0usize);
                for (_, field) in fields {
                    count = match (count, self.nodes[*field].scalars) {
                        (Some(a), Some(b)) => Some(
                            a.checked_add(b)
                                .ok_or_else(|| wire.malformed("record scalar count overflow"))?,
                        ),
                        _ => None,
                    };
                }
                count
            }
        };
        let id = self.nodes.len();
        self.nodes.push(Type {
            name,
            base,
            shape,
            scalars,
        });
        Ok(id)
    }

    /// Consume nested constraints with a work stack, rather than Rust recursion.
    fn constrain(&mut self, base: usize, wire: &mut Wire) -> Result<usize> {
        enum Frame {
            Start(usize),
            Array {
                base: usize,
                indices: Vec<usize>,
                bounds: Vec<Range>,
            },
            Record {
                base: usize,
                fields: Vec<(usize, usize)>,
                next: usize,
            },
        }
        let mut work = vec![Frame::Start(base)];
        let mut result = None;
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Start(base) => match self.nodes[self.effective(base)].shape.clone() {
                    Shape::Array {
                        element, indices, ..
                    } => {
                        let mut bounds = Vec::new();
                        for &index in &indices {
                            let range = Range::read(wire)?;
                            self.validate_range(index, &range, wire)?;
                            bounds.push(range);
                        }
                        if self.nodes[element].scalars.is_some() {
                            result = Some(self.push(
                                0,
                                Some(base),
                                Shape::Array {
                                    element,
                                    indices,
                                    bounds: Some(bounds),
                                },
                                wire,
                            )?);
                        } else {
                            work.push(Frame::Array {
                                base,
                                indices,
                                bounds,
                            });
                            work.push(Frame::Start(element));
                        }
                    }
                    Shape::Record { fields } => work.push(Frame::Record {
                        base,
                        fields,
                        next: 0,
                    }),
                    _ => return Err(wire.malformed("composite bounds have non-composite base")),
                },
                Frame::Array {
                    base,
                    indices,
                    bounds,
                } => {
                    let element = result
                        .take()
                        .ok_or_else(|| wire.malformed("missing nested array constraints"))?;
                    result = Some(self.push(
                        0,
                        Some(base),
                        Shape::Array {
                            element,
                            indices,
                            bounds: Some(bounds),
                        },
                        wire,
                    )?);
                }
                Frame::Record {
                    base,
                    mut fields,
                    mut next,
                } => {
                    if next != 0
                        && let Some(child) = result.take()
                    {
                        fields[next - 1].1 = child;
                    }
                    while next < fields.len() && self.nodes[fields[next].1].scalars.is_some() {
                        next += 1;
                    }
                    if next == fields.len() {
                        result = Some(self.push(0, Some(base), Shape::Record { fields }, wire)?);
                    } else {
                        let child = fields[next].1;
                        work.push(Frame::Record {
                            base,
                            fields,
                            next: next + 1,
                        });
                        work.push(Frame::Start(child));
                    }
                }
            }
        }
        result.ok_or_else(|| wire.malformed("missing composite constraints"))
    }

    pub(super) fn wkt(&mut self, wire: &mut Wire) -> Result<()> {
        wire.zeros()?;
        loop {
            let marker = wire.byte()?;
            if marker == 0 {
                return Ok(());
            }
            if marker > 3 {
                return Err(wire.unsupported(format_args!("well-known type marker {marker}")));
            }
            let id = self.type_id(wire)?;
            let node = &mut self.nodes[id];
            let Shape::Scalar {
                storage,
                literals,
                wkt,
                ..
            } = &mut node.shape
            else {
                return Err(wire.malformed("well-known marker on non-enum type"));
            };
            let expected: &[&str] = match marker {
                1 => &["false", "true"],
                2 => &["'0'", "'1'"],
                3 => &[
                    "'U'", "'X'", "'0'", "'1'", "'Z'", "'W'", "'L'", "'H'", "'-'",
                ],
                _ => return Err(wire.unsupported(format_args!("well-known type marker {marker}"))),
            };
            if *storage
                != if marker == 3 {
                    Storage::E8
                } else {
                    Storage::B2
                }
                || literals
                    .iter()
                    .map(|&i| self.strings[i].as_str())
                    .ne(expected.iter().copied())
                || (*wkt != 0 && *wkt != marker)
            {
                return Err(wire.malformed("well-known marker contradicts enum storage/literals"));
            }
            *wkt = marker;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn parse(entries: &[u8], count: u32) -> (Wire, Types) {
        let mut bytes = b"TYP\0\0\0\0\0".to_vec();
        bytes.extend(count.to_le_bytes());
        bytes.extend(entries);
        bytes.push(0);
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(1, Cursor::new(bytes)))).unwrap();
        let types = Types::read(
            &mut wire,
            vec!["".into(), "integer".into(), "vector".into(), "field".into()],
        )
        .unwrap();
        (wire, types)
    }

    #[test]
    fn nested_array_constraints_and_following_entry() {
        // integer, unconstrained vector, unbounded alias, outer array of alias,
        // outer(0 to 0)(0 downto 0), then an unrelated physical32 type.
        let entries = [
            25, 1, 31, 2, 1, 1, 1, 37, 2, 2, 31, 2, 3, 1, 1, 35, 0, 4, 25, 0, 0, 153, 0, 0, 28, 1,
        ];
        let (_, types) = parse(&entries, 6);
        let node = &types.nodes[types.source_ids[4]];
        assert_eq!(node.scalars, Some(1));
        assert_eq!(types.name(types.source_ids[4]), Some("vector"));
        assert_eq!(types.scalar(types.source_ids[5]).unwrap().0, Storage::P32);
        let Shape::Array { element, .. } = node.shape else {
            panic!("array");
        };
        assert_eq!(types.nodes[element].scalars, Some(1));
    }

    #[test]
    fn unbounded_record_alias_and_named_bounded_record() {
        let entries = [
            25, 1, 31, 2, 1, 1, 1, 32, 3, 1, 3, 2, 39, 3, 3, 38, 3, 4, 153, 2, 0, 38, 0, 5,
        ];
        let (_, types) = parse(&entries, 6);
        assert_eq!(types.nodes[types.source_ids[2]].scalars, None);
        assert_eq!(types.nodes[types.source_ids[4]].scalars, Some(3));
        assert_eq!(types.nodes[types.source_ids[5]].scalars, Some(3));
        assert_eq!(types.name(types.source_ids[5]), Some("field"));
    }

    #[test]
    fn multidimensional_null_range_does_not_become_absolute_width() {
        let entries = [25, 1, 31, 2, 1, 2, 1, 1, 35, 0, 2, 25, 0, 2, 153, 0, 1];
        let (_, types) = parse(&entries, 3);
        assert_eq!(types.nodes[types.source_ids[2]].scalars, Some(0));
        assert_eq!(Storage::I64.tag(), 26);
    }

    #[test]
    fn physical_versions_and_real_constraints() {
        let (_, types) = parse(&[28, 1, 29, 1, 27, 1], 3);
        assert!(matches!(&types.nodes[0].shape, Shape::Scalar { units, .. } if units.is_empty()));
        let mut bytes = b"TYP\0\0\0\0\0\x01\0\0\0\x1c\x01\x01\x03\x7e\0".to_vec();
        let mut wire = Wire::new(Box::new(Cursor::new(bytes.clone()))).unwrap();
        wire.version = 1;
        let types = Types::read(
            &mut wire,
            vec!["".into(), "physical".into(), "".into(), "unit".into()],
        )
        .unwrap();
        assert!(
            matches!(&types.nodes[0].shape, Shape::Scalar { units, .. } if units == &[(3, -2)])
        );
        bytes = vec![27];
        bytes.extend(0x8000000000000000u64.to_le_bytes());
        bytes.extend(0x3ff0000000000000u64.to_le_bytes());
        let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
        let range = Range::read(&mut wire).unwrap();
        assert_eq!(range.left, Bound::Real(0x8000000000000000));
        assert_eq!(range.integer_bounds(), None);
    }

    #[test]
    fn enum_constraints_preserve_base_ordinals_and_markers() {
        let (_, mut types) = parse(&[23, 2, 3, 1, 2, 3, 34, 0, 1, 23, 1, 2], 2);
        assert_eq!(types.scalar(types.source_ids[1]).unwrap().1, [1, 2, 3]);
        assert!(matches!(
            &types.nodes[1].shape,
            Shape::Alias {
                range: Some(Range {
                    left: Bound::Integer(1),
                    ..
                }),
                ..
            }
        ));
        let mut wire = Wire::new(Box::new(Cursor::new(vec![0, 0, 0, 0, 3, 1, 0]))).unwrap();
        assert!(types.wkt(&mut wire).is_err());
    }

    #[test]
    fn invalid_and_unknown_type_forms_do_not_panic() {
        for entries in [
            &[31, 0, 0, 1, 1][..],
            &[34, 0, 1, 25, 0, 0],
            &[25, 1, 34, 0, 1, 23, 0, 1],
        ] {
            let mut bytes = b"TYP\0\0\0\0\0\x02\0\0\0".to_vec();
            bytes.extend(entries);
            bytes.push(0);
            let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
            assert!(Types::read(&mut wire, vec!["".into(), "name".into()]).is_err());
        }
    }

    #[test]
    fn deep_composite_constraints_use_an_explicit_stack() {
        fn unsigned(mut value: u32, bytes: &mut Vec<u8>) {
            loop {
                let mut digit = (value & 127) as u8;
                value >>= 7;
                if value != 0 {
                    digit |= 128;
                }
                bytes.push(digit);
                if value == 0 {
                    break;
                }
            }
        }
        let depth = 2048;
        let mut bytes = b"TYP\0\0\0\0\0".to_vec();
        bytes.extend((depth + 2u32).to_le_bytes());
        bytes.extend([25, 1]);
        for index in 0..depth {
            bytes.extend([31, 0]);
            unsigned(index + 1, &mut bytes);
            bytes.extend([1, 1]);
        }
        bytes.extend([35, 0]);
        unsigned(depth + 1, &mut bytes);
        for _ in 0..depth {
            bytes.extend([25, 0, 0]);
        }
        bytes.push(0);
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(2, Cursor::new(bytes)))).unwrap();
        let types = Types::read(&mut wire, vec!["".into(), "integer".into()]).unwrap();
        let last = *types.source_ids.last().unwrap();
        assert_eq!(types.nodes[last].scalars, Some(1));
        assert_eq!(types.nodes.len(), depth as usize * 2 + 1);
    }

    #[test]
    #[ignore = "requires the independently installed GHW diagnostic fixtures"]
    fn source_headers_match_the_supported_grammar() {
        use std::fs::File;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ghw");
        let names = [
            "ghw0000-bcd-counter-debug",
            "ghw0001-wellen-ghw-bug",
            "ghw0003-wellen-issue-32",
            "ghw0004-wellen-issue-34",
            "ghw0005-wellen-issue-35",
            "ghw0006-wellen-issue-6",
            "ghw0093-wellen-issue-90",
            "ghw0094-surfer-issue-547",
        ];
        for name in names {
            let mut wire = Wire::new(Box::new(BufReader::new(
                File::open(root.join(name).join("waveform.ghw")).unwrap(),
            )))
            .unwrap();
            wire.header().unwrap();
            let strings = wire.strings().unwrap();
            let mut types =
                Types::read(&mut wire, strings).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut section = wire.fixed::<4>().unwrap();
            if &section == b"WKT\0" {
                types.wkt(&mut wire).unwrap();
                section = wire.fixed().unwrap();
            }
            assert_eq!(&section, b"HIE\0", "{name}");
            assert!(!types.source_ids.is_empty());
            eprintln!(
                "{name}: {} source types, {} layouts",
                types.source_ids.len(),
                types.nodes.len()
            );
        }
    }
}
