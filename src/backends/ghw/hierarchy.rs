use std::collections::HashMap;

#[cfg(test)]
use super::types::Bound;
use super::{
    types::{Shape, Storage, Types},
    wire::Wire,
};
use crate::{
    BitRange, Direction, Encoding, Hierarchy, LogicDomain, Packing, Result, Signedness,
    hierarchy::{EnumerationData, ScopeData, VariableData},
};

// Empty layouts can encode huge derived trees without any scalar-ID payload.
// Bound that amplification, rather than reject compact valid empty records.
const EMPTY_SCOPE_LIMIT: usize = 1_000_000;

#[derive(Clone, Debug)]
pub(super) struct Recipe {
    pub(super) ids: Vec<u32>,
    pub(super) encoding: Encoding,
    pub(super) scalar_type: usize,
    pub(super) vector: bool,
}

pub(super) struct Declarations {
    pub(super) hierarchy: Hierarchy,
    pub(super) recipes: Vec<Recipe>,
    pub(super) active: Vec<(u32, usize)>,
}

struct Mapper<'t> {
    types: &'t Types,
    scopes: Vec<ScopeData>,
    variables: Vec<VariableData>,
    recipes: Vec<Recipe>,
    keys: HashMap<(Vec<u32>, Encoding), usize>,
    scalar_types: HashMap<u32, usize>,
    max_id: u32,
    empty_scopes: usize,
}

impl Mapper<'_> {
    fn scope(
        &mut self,
        name: String,
        parent: Option<usize>,
        kind: &str,
        empty: bool,
        wire: &Wire,
    ) -> Result<usize> {
        if empty {
            if self.empty_scopes == EMPTY_SCOPE_LIMIT {
                return Err(wire.unsupported("empty hierarchy expansion above 1000000 scopes"));
            }
            self.empty_scopes += 1;
        }
        let index = self.scopes.len();
        self.scopes.push(ScopeData {
            name,
            name_was_escaped: false,
            parent,
            kind: kind.into(),
            definition_name: None,
            packing: matches!(kind, "array" | "record").then_some(Packing::Unpacked),
            is_hidden: false,
        });
        Ok(index)
    }

    fn width(&self, id: usize) -> u32 {
        let (storage, literals, wkt) = self.types.scalar(id).expect("leaf has scalar type");
        match storage {
            Storage::I32 | Storage::P32 => 32,
            Storage::I64 | Storage::P64 => 64,
            Storage::F64 => 0,
            _ if wkt != 0 => 1,
            _ => (usize::BITS - literals.len().saturating_sub(1).leading_zeros()).max(1),
        }
    }

    fn leaf(
        &mut self,
        location: (String, Option<usize>),
        id: usize,
        ids: Vec<u32>,
        direction: Direction,
        vector_range: Option<BitRange>,
        wire: &Wire,
    ) -> Result<()> {
        let (name, parent) = location;
        let vector = self.types.scalar(id).is_none();
        let scalar_type = if vector {
            let Shape::Array { element, .. } = &self.types.nodes[self.types.effective(id)].shape
            else {
                return Err(wire.malformed("vector without array type"));
            };
            *element
        } else {
            id
        };
        let (storage, literals, wkt) = self
            .types
            .scalar(scalar_type)
            .ok_or_else(|| wire.malformed("non-scalar leaf"))?;
        for &source_id in &ids {
            if source_id == 0 || source_id > self.max_id {
                return Err(wire.malformed("hierarchy scalar ID exceeds table boundary"));
            }
            if let Some(&previous) = self.scalar_types.get(&source_id) {
                let (previous_storage, previous_literals, _) = self
                    .types
                    .scalar(previous)
                    .expect("established source scalar");
                if storage != previous_storage || literals.len() != previous_literals.len() {
                    return Err(wire.malformed("incompatible scalar alias storage"));
                }
            } else {
                self.scalar_types.insert(source_id, scalar_type);
            }
        }
        let width = if vector {
            u32::try_from(ids.len()).map_err(|_| wire.unsupported("vector width above u32"))?
        } else {
            self.width(scalar_type)
        };
        let encoding = if storage == Storage::F64 {
            Encoding::Real
        } else {
            Encoding::Bits { width }
        };
        let signal = if ids.is_empty() {
            None
        } else {
            let key = (ids.clone(), encoding);
            Some(if let Some(&index) = self.keys.get(&key) {
                index
            } else {
                let index = self.recipes.len();
                self.recipes.push(Recipe {
                    ids,
                    encoding,
                    scalar_type,
                    vector,
                });
                self.keys.insert(key, index);
                index
            })
        };
        let mut enumeration = None;
        if !vector && matches!(storage, Storage::B2 | Storage::E8) && wkt == 0 {
            let mut allowed = vec![true; literals.len()];
            let mut node = scalar_type;
            while let Shape::Alias { base, range } = &self.types.nodes[node].shape {
                if let Some(range) = range {
                    let (left, right) = range
                        .integer_bounds()
                        .ok_or_else(|| wire.malformed("non-integral enum subtype"))?;
                    let null = if range.downto {
                        left < right
                    } else {
                        left > right
                    };
                    for (ordinal, allowed) in allowed.iter_mut().enumerate() {
                        *allowed &= !null
                            && (left.min(right)..=left.max(right)).contains(&(ordinal as i64));
                    }
                }
                node = *base;
            }
            let variants = literals
                .iter()
                .enumerate()
                .filter(|(i, _)| allowed[*i])
                .map(|(ordinal, &label)| {
                    (
                        format!("{ordinal:0width$b}", width = width as usize),
                        self.types.strings[label].clone(),
                    )
                })
                .collect();
            enumeration = EnumerationData::for_encoding(
                self.types.name(scalar_type).map(str::to_owned),
                variants,
                encoding,
            );
        }
        self.variables.push(VariableData {
            name,
            reader_name: None,
            name_was_escaped: false,
            parent,
            kind: if direction == Direction::Implicit {
                "signal"
            } else {
                "port"
            }
            .into(),
            direction,
            range: vector_range,
            is_constant: false,
            type_name: self.types.name(id).map(str::to_owned),
            signedness: matches!(
                storage,
                Storage::I32 | Storage::I64 | Storage::P32 | Storage::P64
            )
            .then_some(Signedness::Signed),
            logic_domain: match wkt {
                1 | 2 => Some(LogicDomain::TwoState),
                3 => Some(LogicDomain::NineState),
                _ => None,
            },
            enumeration,
            signal,
        });
        Ok(())
    }

    fn object(
        &mut self,
        name: String,
        parent: Option<usize>,
        id: usize,
        direction: Direction,
        wire: &mut Wire,
    ) -> Result<()> {
        // Work items retain source order; child expansion never uses the call stack.
        enum Work {
            Object {
                name: String,
                parent: Option<usize>,
                id: usize,
            },
            Dimension {
                parent: usize,
                array: usize,
                dimension: usize,
                next: i64,
                remaining: u128,
            },
        }
        let mut work = vec![Work::Object { name, parent, id }];
        while let Some(item) = work.pop() {
            match item {
                Work::Object { name, parent, id } => {
                    if name.is_empty() {
                        return Err(wire.malformed("empty object or field name"));
                    }
                    let resolved = self.types.effective(id);
                    match &self.types.nodes[resolved].shape {
                        Shape::Scalar { .. } => {
                            let source_id = wire.uleb()?;
                            self.leaf((name, parent), id, vec![source_id], direction, None, wire)?;
                        }
                        Shape::Record { fields } => {
                            let scope = self.scope(
                                name,
                                parent,
                                "record",
                                self.types.nodes[resolved].scalars == Some(0),
                                wire,
                            )?;
                            for &(field_name, field) in fields.iter().rev() {
                                work.push(Work::Object {
                                    name: self.types.strings[field_name].clone(),
                                    parent: Some(scope),
                                    id: field,
                                });
                            }
                        }
                        Shape::Array {
                            element,
                            indices,
                            bounds: Some(bounds),
                        } => {
                            let vector = indices.len() == 1
                                && matches!(
                                    self.types.scalar(indices[0]).map(|s| s.0),
                                    Some(Storage::I32 | Storage::I64)
                                )
                                && self
                                    .types
                                    .scalar(*element)
                                    .is_some_and(|(_, _, wkt)| matches!(wkt, 2 | 3));
                            if vector {
                                let n = self.types.nodes[resolved]
                                    .scalars
                                    .ok_or_else(|| wire.malformed("unbounded vector"))?;
                                if n as u64 > wire.len.saturating_sub(wire.offset) {
                                    return Err(
                                        wire.malformed("vector scalar IDs exceed remaining input")
                                    );
                                }
                                let mut ids = Vec::new();
                                for _ in 0..n {
                                    ids.push(wire.uleb()?);
                                }
                                let range = if n == 0 {
                                    None
                                } else {
                                    let (left, right) =
                                        bounds[0].integer_bounds().ok_or_else(|| {
                                            wire.malformed("non-integral vector bounds")
                                        })?;
                                    Some(BitRange::new(left, right))
                                };
                                self.leaf((name, parent), id, ids, direction, range, wire)?;
                            } else {
                                let empty = self.types.nodes[resolved].scalars == Some(0);
                                let scope = self.scope(name, parent, "array", empty, wire)?;
                                let extents = bounds
                                    .iter()
                                    .map(|range| dimension_bounds(range, wire).map(|(_, n)| n))
                                    .collect::<Result<Vec<_>>>()?;
                                if extents.contains(&0) {
                                    continue;
                                }
                                if empty {
                                    let budget = (EMPTY_SCOPE_LIMIT - self.empty_scopes) as u128;
                                    let count = extents.iter().try_fold(1u128, |count, &extent| {
                                        count.checked_mul(extent)
                                    });
                                    if count.is_none_or(|n| n > budget) {
                                        return Err(wire.unsupported(
                                            "empty hierarchy expansion above 1000000 scopes",
                                        ));
                                    }
                                }
                                let (next, remaining) = dimension_bounds(&bounds[0], wire)?;
                                work.push(Work::Dimension {
                                    parent: scope,
                                    array: resolved,
                                    dimension: 0,
                                    next,
                                    remaining,
                                });
                            }
                        }
                        Shape::Array { bounds: None, .. } => {
                            return Err(wire.malformed("unbounded hierarchy array"));
                        }
                        Shape::Alias { .. } => unreachable!("effective type removes aliases"),
                    }
                }
                Work::Dimension {
                    parent,
                    array,
                    dimension,
                    next,
                    remaining,
                } => {
                    if remaining == 0 {
                        continue;
                    }
                    let Shape::Array {
                        element,
                        indices,
                        bounds: Some(bounds),
                    } = &self.types.nodes[array].shape
                    else {
                        return Err(wire.malformed("missing array bounds"));
                    };
                    let (storage, labels, _) = self
                        .types
                        .scalar(indices[dimension])
                        .ok_or_else(|| wire.malformed("invalid array index type"))?;
                    let component = if matches!(storage, Storage::B2 | Storage::E8) {
                        let label = usize::try_from(next)
                            .ok()
                            .and_then(|i| labels.get(i))
                            .ok_or_else(|| wire.malformed("enum array index out of range"))?;
                        format!("[{}]", self.types.strings[*label])
                    } else {
                        format!("[{next}]")
                    };
                    if remaining > 1 {
                        let following = if bounds[dimension].downto {
                            next.checked_sub(1)
                        } else {
                            next.checked_add(1)
                        }
                        .ok_or_else(|| wire.malformed("array index overflow"))?;
                        work.push(Work::Dimension {
                            parent,
                            array,
                            dimension,
                            next: following,
                            remaining: remaining - 1,
                        });
                    }
                    if dimension + 1 == indices.len() {
                        work.push(Work::Object {
                            name: component,
                            parent: Some(parent),
                            id: *element,
                        });
                    } else {
                        let scope = self.scope(
                            component,
                            Some(parent),
                            "array",
                            self.types.nodes[array].scalars == Some(0),
                            wire,
                        )?;
                        let (next, remaining) = dimension_bounds(&bounds[dimension + 1], wire)?;
                        work.push(Work::Dimension {
                            parent: scope,
                            array,
                            dimension: dimension + 1,
                            next,
                            remaining,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn dimension_bounds(range: &super::types::Range, wire: &Wire) -> Result<(i64, u128)> {
    let (left, right) = range
        .integer_bounds()
        .ok_or_else(|| wire.malformed("non-integral array dimension"))?;
    let distance = if range.downto {
        i128::from(left) - i128::from(right)
    } else {
        i128::from(right) - i128::from(left)
    };
    Ok((
        left,
        if distance < 0 {
            0
        } else {
            distance as u128 + 1
        },
    ))
}

pub(super) fn read(wire: &mut Wire, types: &Types) -> Result<Declarations> {
    wire.zeros()?;
    let _scopes = wire.count("hierarchy scope")?;
    let _variables = wire.count("hierarchy variable")?;
    let max_id = wire.count("scalar table")? as u32;
    let mut mapper = Mapper {
        types,
        scopes: Vec::new(),
        variables: Vec::new(),
        recipes: Vec::new(),
        keys: HashMap::new(),
        scalar_types: HashMap::new(),
        max_id,
        empty_scopes: 0,
    };
    let mut stack = Vec::new();
    loop {
        let tag = wire.byte()?;
        if tag == 0 {
            break;
        }
        if tag == 15 {
            if stack.pop().is_none() {
                return Err(wire.malformed("hierarchy scope underflow"));
            }
            continue;
        }
        if !matches!(tag, 3..=7 | 13 | 14 | 16..=21) {
            return Err(wire.unsupported(format_args!("hierarchy tag {tag}")));
        }
        let name_id = types.string_id(wire)?;
        if name_id == 0 || types.strings[name_id].is_empty() {
            return Err(wire.malformed("empty hierarchy name"));
        }
        let mut name = types.strings[name_id].clone();
        let parent = stack.last().copied();
        if matches!(tag, 16..=21) {
            let id = types.type_id(wire)?;
            if types.nodes[id].scalars.is_none() {
                return Err(wire.malformed("unbounded hierarchy object type"));
            }
            let direction = match tag {
                17 => Direction::Input,
                18 => Direction::Output,
                19 => Direction::InOut,
                20 => Direction::Buffer,
                21 => Direction::Linkage,
                _ => Direction::Implicit,
            };
            mapper.object(name, parent, id, direction, wire)?;
        } else {
            let kind = match tag {
                3 => "block",
                4 => "if-generate",
                5 => "for-generate",
                6 => "instance",
                7 => "package",
                13 => "process",
                14 => "ghw:generic",
                _ => unreachable!(),
            };
            if tag == 5 {
                let id = types.type_id(wire)?;
                let (storage, literals, _) = types
                    .scalar(id)
                    .ok_or_else(|| wire.malformed("non-discrete generate iterator"))?;
                let iterator = match storage {
                    Storage::B2 | Storage::E8 => {
                        let ordinal = wire.byte()? as usize;
                        let label = literals
                            .get(ordinal)
                            .ok_or_else(|| wire.malformed("generate enum iterator out of range"))?;
                        types.strings[*label].clone()
                    }
                    Storage::I32 => wire.sleb(32)?.to_string(),
                    Storage::I64 => wire.sleb(64)?.to_string(),
                    _ => return Err(wire.malformed("non-discrete generate iterator storage")),
                };
                name.push('(');
                name.push_str(&iterator);
                name.push(')');
            }
            let scope = mapper.scope(name, parent, kind, false, wire)?;
            if tag != 13 {
                stack.push(scope);
            }
        }
    }
    if !stack.is_empty() {
        return Err(wire.malformed("unclosed hierarchy scopes"));
    }
    let mut active = mapper
        .scalar_types
        .into_iter()
        .map(|(id, ty)| (id, types.effective(ty)))
        .collect::<Vec<_>>();
    active.sort_unstable_by_key(|&(id, _)| id);
    let encodings = mapper.recipes.iter().map(|r| r.encoding).collect();
    Ok(Declarations {
        hierarchy: Hierarchy::new(mapper.scopes, mapper.variables, encodings),
        recipes: mapper.recipes,
        active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    #[ignore = "requires the independently installed GHW diagnostic fixtures"]
    fn source_hierarchy_preserves_issue_paths_and_ordered_recipes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ghw");
        for (name, path, expected) in [
            (
                "ghw0001-wellen-ghw-bug",
                "test_rom_tb.soc_inst.core_inst.state",
                vec![330, 331, 332],
            ),
            (
                "ghw0094-surfer-issue-547",
                "cpu_core_tb.uut.reg_file.addr_c",
                vec![828, 829, 830, 831, 832],
            ),
        ] {
            let file = std::fs::File::open(root.join(name).join("waveform.ghw")).unwrap();
            let mut wire = Wire::new(Box::new(BufReader::new(file))).unwrap();
            wire.header().unwrap();
            let strings = wire.strings().unwrap();
            let mut types = Types::read(&mut wire, strings).unwrap();
            let mut tag = wire.fixed::<4>().unwrap();
            if &tag == b"WKT\0" {
                types.wkt(&mut wire).unwrap();
                tag = wire.fixed().unwrap();
            }
            assert_eq!(&tag, b"HIE\0");
            let declarations = read(&mut wire, &types).unwrap();
            wire.marker(b"EOH\0").unwrap();
            let signal = declarations.hierarchy.signal(path).unwrap();
            assert_eq!(declarations.recipes[signal.index()].ids, expected);
            assert!(declarations.recipes[signal.index()].vector);
            assert_eq!(
                types
                    .scalar(declarations.recipes[signal.index()].scalar_type)
                    .unwrap()
                    .2,
                3
            );
            assert!(!declarations.active.is_empty());
            eprintln!(
                "{name}: {} variables, {} recipes",
                declarations.hierarchy.variables().count(),
                declarations.recipes.len()
            );
        }
    }

    #[test]
    fn ordered_overlap_ports_and_enum_generate_traversal() {
        let strings = [
            "",
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
            "array",
            "top",
            "scalar",
            "whole",
            "alias",
            "reversed",
            "subset",
            "overlap",
            "repeated",
            "input",
            "output",
            "inout",
            "buffer",
            "linkage",
            "gen",
            "color",
            "red",
            "blue",
            "integer",
        ];
        let sid = |name| strings.iter().position(|&s| s == name).unwrap() as u8;
        let mut bytes = b"TYP\0\0\0\0\0\x06\0\0\0".to_vec();
        bytes.extend([23, sid("std_ulogic"), 9, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        // I32 index, unbounded logic array, 2 downto 0 and 1 downto 0 bounds.
        bytes.extend([
            25,
            sid("integer"),
            31,
            sid("array"),
            1,
            1,
            2,
            35,
            0,
            3,
            153,
            2,
            0,
            35,
            0,
            3,
            153,
            1,
            0,
        ]);
        bytes.extend([23, sid("color"), 2, sid("red"), sid("blue"), 0]);
        let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
        let mut types =
            Types::read(&mut wire, strings.into_iter().map(str::to_owned).collect()).unwrap();
        let mut wire = Wire::new(Box::new(Cursor::new(vec![0, 0, 0, 0, 3, 1, 0]))).unwrap();
        types.wkt(&mut wire).unwrap();
        let mut bytes = vec![0; 4];
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(13u32.to_le_bytes());
        bytes.extend(4u32.to_le_bytes());
        bytes.extend([3, sid("top")]);
        for (name, ty, ids) in [
            ("scalar", 1, vec![1]),
            ("whole", 4, vec![3, 1, 4]),
            ("alias", 4, vec![3, 1, 4]),
            ("reversed", 4, vec![4, 1, 3]),
            ("subset", 5, vec![3, 1]),
            ("overlap", 5, vec![1, 4]),
            ("repeated", 4, vec![1, 1, 4]),
        ] {
            bytes.extend([16, sid(name), ty]);
            bytes.extend(ids);
        }
        for (tag, name) in [
            (17, "input"),
            (18, "output"),
            (19, "inout"),
            (20, "buffer"),
            (21, "linkage"),
        ] {
            bytes.extend([tag, sid(name), 1, 1]);
        }
        bytes.extend([5, sid("gen"), 6, 1, 16, sid("scalar"), 1, 1, 15, 15, 0]);
        let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
        let declarations = read(&mut wire, &types).unwrap();
        let h = &declarations.hierarchy;
        assert_eq!(
            h.signal("top.whole").unwrap(),
            h.signal("top.alias").unwrap()
        );
        let recipes = [
            "scalar", "whole", "reversed", "subset", "overlap", "repeated",
        ]
        .map(|name| h.signal(&format!("top.{name}")).unwrap());
        assert_eq!(
            recipes
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            6
        );
        assert_eq!(h.signals().count(), 6);
        for (name, direction) in [
            ("input", Direction::Input),
            ("output", Direction::Output),
            ("inout", Direction::InOut),
            ("buffer", Direction::Buffer),
            ("linkage", Direction::Linkage),
        ] {
            assert_eq!(
                h.variable(&format!("top.{name}")).unwrap().direction(),
                direction
            );
        }
        assert_eq!(h.signal("top.gen(blue).scalar").unwrap(), recipes[0]);
        let mut stack = h.roots().collect::<Vec<_>>();
        let mut paths = Vec::new();
        while let Some(item) = stack.pop() {
            match item {
                crate::Item::Scope(scope) => stack.extend(scope.children()),
                crate::Item::Variable(variable) => paths.push(variable.path()),
            }
        }
        paths.sort_by_key(ToString::to_string);
        let mut expected = h.variables().map(|v| v.path()).collect::<Vec<_>>();
        expected.sort_by_key(ToString::to_string);
        assert_eq!(paths, expected);
    }

    #[test]
    fn extent_respects_direction_including_i64_limits() {
        let wire = Wire::new(Box::new(Cursor::new(Vec::new()))).unwrap();
        let range = super::super::types::Range {
            storage: Storage::I64,
            downto: false,
            left: Bound::Integer(i64::MIN),
            right: Bound::Integer(i64::MAX),
        };
        assert_eq!(
            dimension_bounds(&range, &wire).unwrap(),
            (i64::MIN, 1u128 << 64)
        );
        let null = super::super::types::Range {
            downto: true,
            left: Bound::Integer(0),
            right: Bound::Integer(1),
            ..range
        };
        assert_eq!(dimension_bounds(&null, &wire).unwrap(), (0, 0));
    }

    #[test]
    fn ordered_aliases_scalar_holes_processes_and_null_vectors() {
        let strings = [
            "",
            "integer",
            "std_ulogic",
            "vector",
            "top",
            "a",
            "alias",
            "reversed",
            "one",
            "'U'",
            "'X'",
            "'0'",
            "'1'",
            "'Z'",
            "'W'",
            "'L'",
            "'H'",
            "'-'",
            "process",
            "gen",
            "empty",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let entries = [
            25, 1, 23, 2, 9, 9, 10, 11, 12, 13, 14, 15, 16, 17, 31, 3, 2, 1, 1, 35, 3, 3, 153, 1,
            0, 35, 3, 3, 153, 0, 1,
        ];
        let mut bytes = b"TYP\0\0\0\0\0\x05\0\0\0".to_vec();
        bytes.extend(entries);
        bytes.push(0);
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(1, Cursor::new(bytes)))).unwrap();
        let mut types = Types::read(&mut wire, strings).unwrap();
        let mut wire = Wire::new(Box::new(Cursor::new(vec![0, 0, 0, 0, 3, 2, 0]))).unwrap();
        types.wkt(&mut wire).unwrap();
        let mut bytes = vec![0; 4];
        bytes.extend(99u32.to_le_bytes());
        bytes.extend(6u32.to_le_bytes());
        bytes.extend(3u32.to_le_bytes());
        bytes.extend([
            3, 4, 16, 5, 4, 3, 1, 16, 6, 4, 3, 1, 16, 7, 4, 1, 3, 16, 8, 2, 1, 16, 20, 5, 13, 18,
            5, 19, 1, 0, 17, 8, 2, 1, 15, 15, 0,
        ]);
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(2, Cursor::new(bytes)))).unwrap();
        let declarations = read(&mut wire, &types).unwrap();
        let h = &declarations.hierarchy;
        let a = h.signal("top.a").unwrap();
        assert_eq!(a, h.signal("top.alias").unwrap());
        assert_ne!(a, h.signal("top.reversed").unwrap());
        assert_eq!(
            h.signal("top.one").unwrap(),
            h.signal("top.\"gen(0)\".one").unwrap()
        );
        assert_eq!(declarations.recipes[a.index()].ids, [3, 1]);
        assert_eq!(
            declarations
                .active
                .iter()
                .map(|&(id, _)| id)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(h.signals().count(), 3);
        assert_eq!(h.variables().count(), 6);
        let empty = h.variable("top.empty").unwrap();
        assert!(empty.signal().is_none());
        assert!(empty.range().is_none());
        assert_eq!(empty.type_name(), Some("vector"));
        assert!(h.scope("top.process").is_ok());
    }

    #[test]
    fn records_enum_subranges_and_mixed_direction_dimensions() {
        let strings = [
            "", "integer", "color", "red", "green", "blue", "white", "black", "subset", "packet",
            "value", "state", "matrix", "top", "empty",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        // integer, color, subset(green..white), record, unconstrained matrix,
        // matrix(1 downto 0, green to blue), and a matrix with a null dimension.
        let entries = [
            25, 1, 23, 2, 5, 3, 4, 5, 6, 7, 34, 8, 2, 23, 1, 3, 32, 9, 2, 10, 1, 11, 3, 31, 12, 3,
            2, 1, 2, 35, 0, 5, 153, 1, 0, 23, 1, 2, 35, 0, 5, 25, 0, 1, 23, 3, 1,
        ];
        let mut bytes = b"TYP\0\0\0\0\0\x07\0\0\0".to_vec();
        bytes.extend(entries);
        bytes.push(0);
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(2, Cursor::new(bytes)))).unwrap();
        let types = Types::read(&mut wire, strings).unwrap();
        let mut bytes = vec![0; 4];
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(10u32.to_le_bytes());
        bytes.extend([
            3, 13, 16, 9, 4, 5, 6, 16, 12, 6, 7, 8, 9, 10, 16, 14, 7, 15, 0,
        ]);
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(1, Cursor::new(bytes)))).unwrap();
        let declarations = read(&mut wire, &types).unwrap();
        let h = &declarations.hierarchy;
        let value = h.variable("top.packet.value").unwrap();
        assert_eq!(value.signedness(), Some(Signedness::Signed));
        assert_eq!(value.logic_domain(), None);
        assert_eq!(
            value.signal().unwrap().encoding(),
            Encoding::Bits { width: 32 }
        );
        let state = h.variable("top.packet.state").unwrap();
        assert_eq!(state.type_name(), Some("subset"));
        let variants = state
            .enumeration()
            .unwrap()
            .variants()
            .map(|v| (v.encoded.to_string(), v.label.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            variants,
            [
                ("001".into(), "green".into()),
                ("010".into(), "blue".into()),
                ("011".into(), "white".into())
            ]
        );
        for (path, source_id) in [
            ("top.matrix.\"[1]\".\"[green]\"", 7),
            ("top.matrix.\"[1]\".\"[blue]\"", 8),
            ("top.matrix.\"[0]\".\"[green]\"", 9),
            ("top.matrix.\"[0]\".\"[blue]\"", 10),
        ] {
            let signal = h.signal(path).unwrap();
            assert_eq!(declarations.recipes[signal.index()].ids, [source_id]);
            assert_eq!(signal.encoding(), Encoding::Bits { width: 3 });
        }
        let empty = h.scope("top.empty").unwrap();
        assert_eq!(empty.kind(), "array");
        assert_eq!(empty.packing(), Some(Packing::Unpacked));
        assert_eq!(empty.children().count(), 0);
        assert_eq!(h.variables().count(), 6);
    }

    #[test]
    fn compact_empty_records_are_supported_and_huge_expansion_fails() {
        // An array of empty records carries no scalar-ID payload at all.
        for (right, supported) in [(vec![1], true), (vec![0xff, 0xff, 0xff, 0xff, 7], false)] {
            let mut entries = vec![25, 1, 32, 2, 0, 31, 3, 2, 1, 1, 35, 3, 3, 25, 0];
            entries.extend(right);
            let mut bytes = b"TYP\0\0\0\0\0\x04\0\0\0".to_vec();
            bytes.extend(entries);
            bytes.push(0);
            let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
            let types = Types::read(
                &mut wire,
                ["", "integer", "empty_record", "array", "top"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            )
            .unwrap();
            let mut bytes = vec![0; 4];
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend([3, 4, 16, 3, 4, 15, 0]);
            let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
            let result = read(&mut wire, &types);
            if supported {
                let declarations = result.unwrap();
                assert_eq!(
                    declarations
                        .hierarchy
                        .scope("top.array")
                        .unwrap()
                        .children()
                        .count(),
                    2
                );
                assert_eq!(declarations.hierarchy.signals().count(), 0);
            } else {
                assert!(matches!(
                    result,
                    Err(crate::Error::UnsupportedFeature {
                        operation: "decode",
                        ..
                    })
                ));
            }
        }
    }
}
