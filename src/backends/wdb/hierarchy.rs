use super::{
    debug::{Debug, Decl, Object},
    malformed,
    model::{self, Codec, Node, Shape},
    types::Type,
    unsupported,
};
use crate::{
    BitRange, Direction, Encoding, Hierarchy, Result,
    hierarchy::{ScopeData, VariableData},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Recipe {
    pub start: u64,
    pub len: usize,
    pub shift: usize,
    pub codec: Codec,
    pub time_parameter: bool,
}

pub(super) fn name(raw: &str, verilog: bool) -> (String, bool) {
    if verilog && raw.starts_with('\\') && raw.ends_with(char::is_whitespace) {
        let identifier = raw[1..].trim_end_matches(char::is_whitespace);
        if !identifier.is_empty() && identifier.bytes().all(|b| (33..=126).contains(&b)) {
            return (identifier.to_owned(), true);
        }
    }
    (raw.to_owned(), false)
}

fn kind(code: u32, scope: bool) -> String {
    let known = if scope {
        match code {
            0 => "module",
            1 => "interface",
            2 => "modport",
            3 | 0x12 => "task",
            4 | 0x11 => "function",
            5 => "block",
            7 | 0xd => "process",
            8 | 0xa => "package",
            9 => "entity",
            0xc => "generate",
            _ => "",
        }
    } else {
        match code {
            0 => "variable",
            1 => "parameter",
            3 => "wire",
            4 => "wand",
            5 => "wor",
            6 => "tri",
            7 => "triand",
            8 => "trior",
            9 => "tri0",
            0xa => "tri1",
            0xc => "supply0",
            0xd => "supply1",
            0xe => "signal",
            0xf => "variable",
            0x12 => "generic",
            0x13 => "constant",
            0x14 => "variable",
            0x15 => "signal",
            _ => "",
        }
    };
    if known.is_empty() {
        format!("wdb:{code:#x}")
    } else {
        known.into()
    }
}

struct Mapper<'a> {
    types: &'a [Type],
    variables: Vec<VariableData>,
    recipes: Vec<Recipe>,
    keys: HashMap<Recipe, usize>,
    derived: usize,
}

impl Mapper<'_> {
    fn leaf(
        &mut self,
        location: (String, Option<usize>),
        n: &Node,
        dc: &Decl,
        object: &Object,
        offset: usize,
        time_parameter: bool,
    ) -> Result<()> {
        let (raw_name, parent) = location;
        let sv = self.types[dc.ty].verilog();
        let native = raw_name == dc.name;
        if !native {
            if self.derived == 2_000_000 {
                return Err(unsupported("derived variable count above 2000000"));
            }
            self.derived += 1;
        }
        let (name, escaped) = if native {
            name(&raw_name, sv)
        } else {
            (raw_name.clone(), false)
        };
        let mut codec = n.codec.clone();
        if time_parameter {
            codec = Codec::Real;
        }
        let encoding = codec.encoding();
        let total_offset = u64::from(object.offset)
            .checked_add(offset as u64)
            .ok_or_else(|| malformed("object offset overflow"))?;
        let (byte_offset, shift, len) = if sv && !time_parameter {
            let shift = (total_offset % 32) as usize;
            let bytes = (shift + n.units)
                .div_ceil(32)
                .checked_mul(8)
                .ok_or_else(|| malformed("object storage overflow"))?;
            (total_offset / 32 * 8, shift, bytes)
        } else {
            (total_offset, 0, if time_parameter { 8 } else { n.units })
        };
        let start = object
            .handle
            .checked_add(byte_offset)
            .ok_or_else(|| malformed("object address overflow"))?;
        let signal = {
            let recipe = Recipe {
                start,
                len,
                shift,
                codec: codec.clone(),
                time_parameter,
            };
            if let Some(&id) = self.keys.get(&recipe) {
                Some(id)
            } else {
                let id = self.recipes.len();
                self.recipes.push(recipe.clone());
                self.keys.insert(recipe, id);
                Some(id)
            }
        };
        let enumeration = model::enumeration(self.types, n.ty, &codec)?;
        let range = match &n.shape {
            Shape::Array { ranges, .. }
                if ranges.len() == 1
                    && matches!(encoding, Encoding::Bits { width } if ranges[0].len()? == width as usize) =>
            {
                Some(BitRange::new(ranges[0].left, ranges[0].right))
            }
            _ => None,
        };
        self.variables.push(VariableData {
            name,
            reader_name: native.then_some(raw_name),
            name_was_escaped: escaped,
            parent,
            kind: kind(dc.kind, false),
            direction: match dc.mode {
                0 => Direction::InOut,
                1 => Direction::Input,
                2 => Direction::Output,
                3 => Direction::Buffer,
                4 => Direction::Linkage,
                _ => Direction::Unknown,
            },
            range,
            is_constant: matches!(dc.kind, 1 | 0x12 | 0x13),
            type_name: (!self.types[n.ty].name.is_empty()).then(|| self.types[n.ty].name.clone()),
            signedness: n.signedness,
            logic_domain: n.domain,
            enumeration,
            signal,
        });
        Ok(())
    }

    fn expand(
        &mut self,
        location: (String, Option<usize>),
        n: &Node,
        dc: &Decl,
        object: &Object,
        offset: usize,
        time_parameter: bool,
    ) -> Result<()> {
        let (name, parent) = location;
        let expanded_array = matches!(n.shape, Shape::Array { expand: true, .. });
        if n.codec != Codec::Unsupported && !expanded_array
            || matches!(n.shape, Shape::Leaf)
            || n.units == 0
            || !object.logged
        {
            return self.leaf((name, parent), n, dc, object, offset, time_parameter);
        }
        self.leaf((name.clone(), parent), n, dc, object, offset, false)?;
        let name = self::name(&name, self.types[dc.ty].verilog()).0;
        match &n.shape {
            Shape::Array {
                ranges,
                child,
                stride,
                ..
            } => {
                let mut indices = vec![(String::new(), 0_usize)];
                for range in ranges {
                    let mut next = Vec::new();
                    let count = range.len()?;
                    if indices
                        .len()
                        .checked_mul(count)
                        .is_none_or(|n| n > 1_000_000)
                    {
                        return Err(unsupported("expanded array count above 1000000"));
                    }
                    for (prefix, rank) in indices {
                        for i in 0..count {
                            let index = if range.direction == 1 {
                                range.left + i as i64
                            } else {
                                range.left - i as i64
                            };
                            next.push((format!("{prefix}[{index}]"), rank * count + i));
                        }
                    }
                    indices = next;
                }
                let count = indices.len();
                for (suffix, rank) in indices {
                    let at = if self.types[dc.ty].verilog() {
                        count - 1 - rank
                    } else {
                        rank
                    };
                    self.expand(
                        (format!("{name}{suffix}"), parent),
                        child,
                        dc,
                        object,
                        offset + at * stride,
                        false,
                    )?;
                }
            }
            Shape::Record(fields) => {
                for (field, at, child) in fields {
                    self.expand(
                        (format!("{name}.{field}"), parent),
                        child,
                        dc,
                        object,
                        offset + at,
                        false,
                    )?;
                }
            }
            Shape::Leaf => unreachable!(),
        }
        Ok(())
    }
}

pub(super) fn read(debug: &Debug, types: &[Type]) -> Result<(Hierarchy, Vec<Recipe>)> {
    let mut scopes = Vec::new();
    let mut ids = Vec::new();
    for s in &debug.scopes {
        let unit = &debug.units[s.unit];
        let parent = s.parent.and_then(|id| ids[id]);
        if s.parent.is_none() && unit.kind == 0x13 {
            ids.push(None);
            continue;
        }
        let (name, escaped) = name(&s.name, unit.kind <= 8);
        let id = scopes.len();
        scopes.push(ScopeData {
            name,
            name_was_escaped: escaped,
            parent,
            kind: kind(unit.kind, true),
            definition_name: unit.name.clone(),
            packing: None,
            is_hidden: false,
        });
        ids.push(Some(id));
    }
    let mut mapper = Mapper {
        types,
        variables: Vec::new(),
        recipes: Vec::new(),
        keys: HashMap::new(),
        derived: 0,
    };
    let recorded: HashSet<_> = debug
        .objects
        .iter()
        .filter(|o| o.logged)
        .map(|o| o.handle)
        .collect();
    let mut nodes: HashMap<usize, Node> = HashMap::new();
    for object in &debug.objects {
        let object = &Object {
            logged: object.logged || recorded.contains(&object.handle),
            ..*object
        };
        let dc = &debug.decls[object.decl];
        let n: &Node = match nodes.entry(object.decl) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(model::node(types, dc.ty, &dc.ranges)?)
            }
        };
        let time_parameter = dc.kind == 1
            && types[dc.ty].verilog()
            && types[dc.ty].name.is_empty()
            && dc.size == 64
            && debug.classes.get(dc.class) == Some(&4);
        let opaque = Node::unsupported(dc.ty);
        let n = if n.units != 0
            && n.units != dc.size as usize
            && !time_parameter
            && n.codec != Codec::Real
        {
            if object.logged {
                return Err(malformed(format!(
                    "declaration {:?} size disagrees with type",
                    dc.name
                )));
            }
            &opaque
        } else {
            n
        };
        mapper.expand(
            (dc.name.clone(), ids[object.scope]),
            n,
            dc,
            object,
            0,
            time_parameter,
        )?;
    }
    let encodings = mapper.recipes.iter().map(|r| r.codec.encoding()).collect();
    Ok((
        Hierarchy::new(scopes, mapper.variables, encodings),
        mapper.recipes,
    ))
}
