use super::{
    malformed, unsupported,
    wire::{Cursor, Source, at},
};
use crate::Result;
use std::collections::HashSet;

pub(super) const ARENA_SPAN: u64 = 2048;
pub(super) const PAGE_LEN: usize = 10240;

#[derive(Clone, Copy, Debug)]
pub(super) struct Span {
    pub offset: u64,
    pub len: u64,
}

pub(super) struct Container {
    pub rtti: Span,
    pub debug: Span,
    pub end: u64,
    pub logged: Vec<std::ops::RangeInclusive<usize>>,
    pub arenas: Vec<Vec<Span>>,
}

impl Container {
    pub fn read(source: &mut Source) -> Result<Self> {
        let header = source.bytes(0, 0xc8)?;
        if Cursor::new(&header[..24]).string()? != "Xilinx WAVE DATABASE 01" {
            return Err(malformed("WDB file signature"));
        }
        if Cursor::new(&header[24..48]).string()? != "Xilinx Simulator" {
            return Err(unsupported("non-simulation WDB producer"));
        }
        let mut dirs = std::collections::HashMap::new();
        let mut pointers = at(&header, 0x48)?;
        for _ in 0..3 {
            let offset = pointers.u64()?;
            let bytes = source.bytes(offset, 48)?;
            let name = Cursor::new(&bytes[..24]).string()?;
            if !matches!(name.as_str(), "WDB.Event" | "Xilinx RTTI" | "Xilinx DBG") {
                return Err(unsupported(format!("directory entry {name:?}")));
            }
            let mut c = at(&bytes, 24)?;
            c.expect(1)?;
            c.expect(0)?;
            let span = Span {
                offset: c.u64()?,
                len: c.u64()?,
            };
            source.check(span.offset, span.len)?;
            if dirs.insert(name, span).is_some() {
                return Err(malformed("duplicate directory entry"));
            }
        }
        let event = dirs["WDB.Event"];
        let rtti = dirs["Xilinx RTTI"];
        let debug = dirs["Xilinx DBG"];
        if event.len != 0x48 || event.offset < 0xc8 {
            return Err(malformed("WDB.Event trailer span"));
        }
        let bytes = source.bytes(event.offset, event.len)?;
        let end = at(&bytes, 0)?.u64()?;
        let n = u64::from(at(&bytes, 12)?.u32()?);
        let handles = at(&bytes, 24)?.u64()?;
        if n != handles.div_ceil(ARENA_SPAN) || n > (event.offset - 0xc8) / 8 {
            return Err(malformed("arena table size disagrees with handle space"));
        }
        let count = at(&bytes, 48)?.u64()?;
        let marker = at(&bytes, 56)?.u64()?;
        if (count == 0) != (marker == 0) {
            return Err(malformed("logged marker/count disagreement"));
        }
        let marker_len = count
            .checked_mul(16)
            .ok_or_else(|| malformed("logged count overflow"))?;
        let markers = source.bytes(marker, marker_len)?;
        let mut c = Cursor::new(&markers);
        let mut logged = Vec::new();
        let mut previous = None;
        for _ in 0..count {
            let lo = c.u64()?;
            let hi = c.u64()?;
            if lo > hi || previous.is_some_and(|last| lo <= last) {
                return Err(malformed("unordered logged object ranges"));
            }
            let lo = usize::try_from(lo).map_err(|_| malformed("object index overflow"))?;
            let hi_usize = usize::try_from(hi).map_err(|_| malformed("object index overflow"))?;
            logged.push(lo..=hi_usize);
            previous = Some(hi);
        }
        if at(&bytes, 64)?.u32()? != PAGE_LEN as u32 {
            return Err(unsupported("WDB value page size"));
        }
        let table = source.bytes(event.offset - n * 8, n * 8)?;
        let mut table = Cursor::new(&table);
        let mut arenas = Vec::new();
        let mut seen = HashSet::new();
        let directory_end = |span: Span| {
            span.offset
                .checked_add(span.len)
                .and_then(|end| end.checked_add(48))
                .ok_or_else(|| malformed("section directory offset overflow"))
        };
        let base = directory_end(debug)?.max(directory_end(event)?);
        for _ in 0..n {
            let mut offset = table.u64()?;
            let mut pages = Vec::new();
            while offset != 0 {
                if offset < base || !seen.insert(offset) {
                    return Err(malformed("invalid or repeated arena record pointer"));
                }
                let bytes = source.bytes(offset, 0x4c0)?;
                let next = at(&bytes, 0)?.u64()?;
                let count = at(&bytes, 0x4b8)?.u32()? as usize;
                if count > 100 || next != 0 && (count != 100 || next <= offset) {
                    return Err(malformed("arena continuation/count"));
                }
                for i in 0..count {
                    let span = Span {
                        offset: at(&bytes, 8 + 8 * i)?.u64()?,
                        len: u64::from(at(&bytes, 0x328 + 4 * i)?.u32()?),
                    };
                    source.check(span.offset, span.len)?;
                    pages.push(span);
                }
                if bytes[8 + 8 * count..0x328].iter().any(|&b| b != 0)
                    || bytes[0x328 + 4 * count..0x4b8].iter().any(|&b| b != 0)
                {
                    return Err(malformed("nonzero unused page directory slots"));
                }
                offset = next;
            }
            arenas.push(pages);
        }
        let mut offsets: Vec<_> = seen.into_iter().collect();
        offsets.sort_unstable();
        if offsets.windows(2).any(|w| w[1] - w[0] < 0x4c0) {
            return Err(malformed("overlapping arena records"));
        }
        Ok(Self {
            rtti,
            debug,
            end,
            logged,
            arenas,
        })
    }
}
