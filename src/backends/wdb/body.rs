use super::{
    container::{ARENA_SPAN, PAGE_LEN, Span},
    malformed,
    wire::{Cursor, Source},
};
use crate::Result;
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    ops::{ControlFlow, Range},
};

struct Record {
    time: u64,
    key: u32,
    data: Range<usize>,
}

struct Page {
    raw: Vec<u8>,
    records: Vec<Record>,
}

impl Page {
    fn read(source: &mut Source, span: Span) -> Result<Self> {
        let compressed = source.bytes(span.offset, span.len)?;
        let mut raw = vec![0; PAGE_LEN];
        let n = miniz_oxide::inflate::decompress_slice_iter_to_slice(
            &mut raw,
            [compressed.as_slice()].into_iter(),
            true,
            false,
        )
        .map_err(|e| malformed(format!("value page inflate: {e:?}")))?;
        if n != PAGE_LEN {
            return Err(malformed("value page has wrong inflated length"));
        }
        let mut c = Cursor::new(&raw);
        let first = c.u64()?;
        let last = first
            .checked_add(c.u64()?)
            .ok_or_else(|| malformed("page time overflow"))?;
        let count = c.count(16)?;
        let mut records = Vec::new();
        let mut previous = first;
        for _ in 0..count {
            let time = c.u64()?;
            let key = c.u32()?;
            let len = c.u32()? as usize;
            if time < previous
                || time > last
                || len == 0
                || u64::from(key) + len as u64 > ARENA_SPAN
            {
                return Err(malformed("value record time/address/length"));
            }
            let start = c.at;
            c.take(len)?;
            records.push(Record {
                time,
                key,
                data: start..c.at,
            });
            previous = time;
        }
        if records.first().is_some_and(|r| r.time != first)
            || records.last().is_some_and(|r| r.time != last)
            || raw[c.at..].iter().any(|&b| b != 0)
        {
            return Err(malformed("value page time bounds or padding"));
        }
        Ok(Self { raw, records })
    }
}

struct Arena<'a> {
    spans: &'a [Span],
    page: Option<Page>,
    record: usize,
    previous: Option<u64>,
}

impl Arena<'_> {
    fn seek_before(&mut self, source: &mut Source, end: u64) -> Result<bool> {
        // Seek assumes native page order. Check each decoded page and the
        // probed upper boundary, but not unvisited adjacent pages. A late
        // seek can retain the final page without checking its predecessor.
        // Sequential traversal checks every crossed page boundary.
        let mut lo = 0;
        let mut hi = self.spans.len();
        let mut candidate = None;
        let mut upper = None;
        let mut last = true;
        while lo < hi {
            // The common late point needs only the final page.
            let mid = if last { hi - 1 } else { lo + (hi - lo) / 2 };
            last = false;
            let page = Page::read(source, self.spans[mid])?;
            let Some(first) = page.records.first() else {
                return Ok(false);
            };
            if first.time <= end {
                lo = mid + 1;
                candidate = Some(page);
            } else {
                hi = mid;
                upper = Some(first.time);
            }
        }
        if candidate
            .as_ref()
            .and_then(|page| page.records.last())
            .is_some_and(|candidate_last| {
                upper.is_some_and(|next_page_first| candidate_last.time > next_page_first)
            })
        {
            return Err(malformed("arena pages are not chronological"));
        }
        self.spans = &self.spans[..lo.saturating_sub(1)];
        self.record = candidate.as_ref().map_or(0, |page| page.records.len());
        self.page = candidate;
        Ok(true)
    }

    fn advance(&mut self, source: &mut Source) -> Result<Option<u64>> {
        loop {
            if let Some(record) = self.page.as_ref().and_then(|p| p.records.get(self.record)) {
                if self.previous.is_some_and(|t| record.time < t) {
                    return Err(malformed("arena pages are not chronological"));
                }
                self.previous = Some(record.time);
                return Ok(Some(record.time));
            }
            let Some((span, rest)) = self.spans.split_first() else {
                self.page = None;
                return Ok(None);
            };
            self.page = Some(Page::read(source, *span)?);
            self.spans = rest;
            self.record = 0;
        }
    }

    fn retreat(&mut self, source: &mut Source) -> Result<Option<u64>> {
        loop {
            if self.record > 0 {
                let record = &self.page.as_ref().unwrap().records[self.record - 1];
                if self.previous.is_some_and(|t| record.time > t) {
                    return Err(malformed("arena pages are not chronological"));
                }
                self.previous = Some(record.time);
                return Ok(Some(record.time));
            }
            let Some((span, rest)) = self.spans.split_last() else {
                self.page = None;
                return Ok(None);
            };
            let page = Page::read(source, *span)?;
            self.record = page.records.len();
            self.page = Some(page);
            self.spans = rest;
        }
    }
}

pub(super) enum Event<'a> {
    Write { address: u64, data: &'a [u8] },
    Tick(u64),
}

// Merge selected arenas, keeping one inflated page per arena and no histories.
pub(super) fn walk<B>(
    source: &mut Source,
    spans: &[Vec<Span>],
    selected: &[usize],
    end: u64,
    mut visit: impl FnMut(Event<'_>) -> Result<ControlFlow<B>>,
) -> Result<ControlFlow<B>> {
    let mut cursors = Vec::new();
    let mut heap = BinaryHeap::new();
    for &arena in selected {
        let mut cursor = Arena {
            spans: spans
                .get(arena)
                .ok_or_else(|| malformed("selected arena outside table"))?,
            page: None,
            record: 0,
            previous: None,
        };
        if let Some(time) = cursor.advance(source)? {
            heap.push(Reverse((time, arena, cursors.len())));
        }
        cursors.push(cursor);
    }
    let mut tick = None;
    while let Some(Reverse((time, arena, slot))) = heap.pop() {
        if tick.is_some_and(|t| t != time) {
            if let ControlFlow::Break(value) = visit(Event::Tick(tick.unwrap()))? {
                return Ok(ControlFlow::Break(value));
            }
            tick = None;
        }
        if time > end {
            break;
        }
        tick = Some(time);
        let cursor = &mut cursors[slot];
        let page = cursor.page.as_ref().unwrap();
        let record = &page.records[cursor.record];
        let address = (arena as u64)
            .checked_mul(ARENA_SPAN)
            .and_then(|base| base.checked_add(u64::from(record.key)))
            .ok_or_else(|| malformed("value address overflow"))?;
        if let ControlFlow::Break(value) = visit(Event::Write {
            address,
            data: &page.raw[record.data.clone()],
        })? {
            return Ok(ControlFlow::Break(value));
        }
        cursor.record += 1;
        if let Some(next) = cursor.advance(source)? {
            heap.push(Reverse((next, arena, slot)));
        }
    }
    if let Some(time) = tick {
        return visit(Event::Tick(time));
    }
    Ok(ControlFlow::Continue(()))
}
// Merge backwards with the same bounded page cursors as the forward walk.
pub(super) fn backwards<B>(
    source: &mut Source,
    spans: &[Vec<Span>],
    selected: &[usize],
    end: u64,
    mut visit: impl FnMut(Event<'_>) -> Result<ControlFlow<B>>,
) -> Result<Option<ControlFlow<B>>> {
    let mut cursors = Vec::new();
    let mut heap = BinaryHeap::new();
    for &arena in selected {
        let mut cursor = Arena {
            spans: spans
                .get(arena)
                .ok_or_else(|| malformed("point arena outside table"))?,
            page: None,
            record: 0,
            previous: None,
        };
        if !cursor.seek_before(source, end)? {
            return Ok(None);
        }
        if let Some(time) = cursor.retreat(source)? {
            heap.push((time, arena, cursors.len()));
        }
        cursors.push(cursor);
    }
    let mut tick = None;
    while let Some((time, arena, slot)) = heap.pop() {
        if tick.is_some_and(|t| t != time) {
            if let ControlFlow::Break(value) = visit(Event::Tick(tick.unwrap()))? {
                return Ok(Some(ControlFlow::Break(value)));
            }
            tick = None;
        }
        let cursor = &mut cursors[slot];
        let page = cursor.page.as_ref().unwrap();
        let record = &page.records[cursor.record - 1];
        if time <= end {
            tick = Some(time);
            let address = (arena as u64)
                .checked_mul(ARENA_SPAN)
                .and_then(|base| base.checked_add(u64::from(record.key)))
                .ok_or_else(|| malformed("point address overflow"))?;
            if let ControlFlow::Break(value) = visit(Event::Write {
                address,
                data: &page.raw[record.data.clone()],
            })? {
                return Ok(Some(ControlFlow::Break(value)));
            }
        }
        cursor.record -= 1;
        if let Some(next) = cursor.retreat(source)? {
            heap.push((next, arena, slot));
        }
    }
    if let Some(time) = tick {
        return visit(Event::Tick(time)).map(Some);
    }
    Ok(Some(ControlFlow::Continue(())))
}
