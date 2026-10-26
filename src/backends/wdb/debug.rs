use super::{
    container::{Container, Span},
    malformed,
    types::Range,
    unsupported,
    wire::{Cursor, Source, at},
};
use crate::Result;

pub(super) struct Scope {
    pub name: String,
    pub parent: Option<usize>,
    pub unit: usize,
}

pub(super) struct Unit {
    pub name: Option<String>,
    pub kind: u32,
}

pub(super) struct Decl {
    pub name: String,
    pub size: u32,
    pub ty: usize,
    pub kind: u32,
    pub mode: u32,
    pub class: usize,
    pub ranges: Vec<Range>,
}

pub(super) struct Object {
    pub handle: u64,
    pub offset: u32,
    pub scope: usize,
    pub decl: usize,
    pub logged: bool,
}

pub(super) struct Debug {
    pub version: u8,
    pub precision: i32,
    pub scopes: Vec<Scope>,
    pub units: Vec<Unit>,
    pub decls: Vec<Decl>,
    pub objects: Vec<Object>,
    pub classes: Vec<u32>,
}

fn region<'a>(bytes: &'a [u8], offsets: &[usize], from: usize, to: usize) -> Result<&'a [u8]> {
    bytes
        .get(offsets[from]..offsets[to])
        .ok_or_else(|| malformed("DBG region span"))
}

fn records(bytes: &[u8], n: usize, words: usize) -> Result<()> {
    if n.checked_mul(words)
        .and_then(|n| n.checked_mul(4))
        .map(|n| n.div_ceil(8) * 8)
        != Some(bytes.len())
    {
        return Err(malformed("DBG record count/region size mismatch"));
    }
    Ok(())
}

fn name(pool: &[u8], offset: i32) -> Result<String> {
    let offset = usize::try_from(offset).map_err(|_| malformed("negative name pool offset"))?;
    at(pool, offset)?.string()
}

pub(super) fn read(source: &mut Source, container: &Container, num_types: usize) -> Result<Debug> {
    let Span { offset, len } = container.debug;
    let bytes = source.bytes(offset, len)?;
    let mut c = Cursor::new(&bytes);
    let version = match c.string()?.as_str() {
        "Xilinx ISim DBG 004" => 4,
        "Xilinx ISim DBG 005" => 5,
        "Xilinx ISim DBG 006" => 6,
        _ => return Err(unsupported("WDB DBG version")),
    };
    c.u32()?;
    let precision = c.i32()?;
    if !(-15..=0).contains(&precision) {
        return Err(unsupported("WDB timestamp precision"));
    }
    let mut offsets = [0; 18];
    for offset in &mut offsets {
        *offset = c.u32()? as usize;
    }
    let mut counts = [0; 4];
    for count in &mut counts {
        *count = c.u32()? as usize;
    }
    let mut words = [0; 17];
    for word in &mut words {
        *word = c.u32()? as usize;
    }
    if offsets.iter().any(|&p| p < c.at || p > bytes.len()) {
        return Err(malformed("DBG region offset"));
    }
    let instance_len = match version {
        4 => 40,
        5 => 48,
        _ => 56,
    };
    if counts[2].checked_mul(instance_len) != Some(bytes.len() - offsets[2]) {
        return Err(malformed("DBG instance count"));
    }
    let scope_pool = region(&bytes, &offsets, 5, 10)?;
    let decl_pool = region(&bytes, &offsets, 10, 11)?;
    let scope_bytes = region(&bytes, &offsets, 0, 1)?;
    let unit_bytes = region(&bytes, &offsets, 1, 3)?;
    let decl_bytes = region(&bytes, &offsets, 3, 4)?;
    records(scope_bytes, counts[0], 9)?;
    let unit_words = if version == 4 { 8 } else { 9 };
    records(unit_bytes, counts[1], unit_words)?;
    let decl_words = if version == 4 { 10 } else { 11 };
    records(decl_bytes, counts[3], decl_words)?;
    let mut units = Vec::new();
    for i in 0..counts[1] {
        let mut r = at(unit_bytes, i * unit_words * 4)?;
        let offset = r.i32()?;
        let name = if offset >= 0 {
            Some(name(scope_pool, offset)?)
        } else {
            None
        };
        r.i32()?;
        units.push(Unit {
            name,
            kind: r.u32()?,
        });
    }
    let mut scopes = Vec::new();
    for i in 0..counts[0] {
        let mut r = at(scope_bytes, i * 36)?;
        let name = name(scope_pool, r.i32()?)?;
        let parent = r.i32()?;
        r.take(24)?;
        let unit = r.u32()? as usize;
        let parent = match (i, parent) {
            (0, -1) => None,
            (_, p) if p >= 0 && (p as usize) < i => Some(p as usize),
            _ => return Err(malformed("DBG scope parent is not an earlier scope")),
        };
        if unit >= units.len() {
            return Err(malformed("DBG scope unit"));
        }
        scopes.push(Scope { name, parent, unit });
    }
    let mut ranges = Vec::new();
    let mut r = Cursor::new(region(&bytes, &offsets, 4, 5)?);
    while r.at < r.data.len() {
        let left = r.u64()? as i64;
        let right = r.u64()? as i64;
        let direction = r.i32()?;
        r.u32()?;
        if !matches!(direction, -2 | -1 | 1) {
            return Err(unsupported("DBG range direction"));
        }
        ranges.push(Range {
            left,
            right,
            direction,
        });
    }
    let class_bytes = region(&bytes, &offsets, 17, 2)?;
    records(class_bytes, words[13], 3)?;
    let mut classes = Vec::new();
    for i in 0..words[13] {
        classes.push(at(class_bytes, 12 * i)?.u32()?);
    }
    let mut decls = Vec::new();
    for i in 0..counts[3] {
        let mut r = at(decl_bytes, i * decl_words * 4)?;
        let name = name(decl_pool, r.i32()?)?;
        let class = r.u32()? as usize;
        r.take(8)?;
        let size = r.u32()?;
        let ty = r.u32()? as usize;
        let n = r.u32()? as usize;
        let first = r.u32()? as usize;
        let kind = r.u32()?;
        let mode = r.u32()?;
        if ty >= num_types || class >= classes.len() && !(class == 0 && classes.is_empty()) {
            return Err(malformed("DBG declaration type/value class"));
        }
        let ranges = if n == 0 {
            Vec::new()
        } else {
            let end = first
                .checked_add(n)
                .ok_or_else(|| malformed("range count overflow"))?;
            ranges
                .get(first..end)
                .ok_or_else(|| malformed("DBG declaration ranges"))?
                .to_vec()
        };
        decls.push(Decl {
            name,
            size,
            ty,
            kind,
            mode,
            class,
            ranges,
        });
    }
    let mut objects = Vec::new();
    let mut r = Cursor::new(&bytes[offsets[2]..]);
    for _ in 0..counts[2] {
        let bytes = r.take(instance_len)?;
        let mut c = Cursor::new(bytes);
        let handle = c.u64()?;
        c.u64()?;
        let scope = c.u32()? as usize;
        let offset = c.u32()?;
        c.take(8)?;
        let decl =
            usize::try_from(c.u64()?).map_err(|_| malformed("DBG declaration index overflow"))?;
        if scope >= scopes.len() || decl >= decls.len() {
            return Err(malformed("DBG object scope/declaration"));
        }
        objects.push(Object {
            handle,
            offset,
            scope,
            decl,
            logged: false,
        });
    }
    for range in &container.logged {
        let objects = objects
            .get_mut(range.clone())
            .ok_or_else(|| malformed("logged range exceeds object count"))?;
        for object in objects {
            object.logged = true;
        }
    }
    Ok(Debug {
        version,
        precision,
        scopes,
        units,
        decls,
        objects,
        classes,
    })
}
