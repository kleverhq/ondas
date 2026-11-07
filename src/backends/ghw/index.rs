use std::{cmp::Reverse, collections::BinaryHeap, ops::ControlFlow};

use super::{
    body,
    types::{Storage, Types},
    wire::Wire,
};
use crate::{Result, Time};

// A backend budget, not a platform or resident-set limit. The representation is
// bounded by the validated source: frames, one frame-number stream per group and
// two fields per (group, frame) pair. An input that does not fit keeps the
// sequential walker.
const INDEX_BYTES: usize = 512 * 1024 * 1024;
// Consecutive active ranks covered by one group. A group records validated
// source positions only, never values or query results.
pub(super) const RANKS_PER_GROUP: usize = 64;
// Minimum growth steps keep tiny budgets usable; growth itself is geometric.
const FRAME_CHUNK: usize = 64;
const STREAM_START: usize = 16;
// A group that already has a frame recorded in the current frame.
const NO_GROUP: usize = usize::MAX;
// Retained per-group arrays: two byte streams of `Vec<u8>` elements plus one
// `u64` and three `u32` counters. Used for the constructor preflight.
const GROUPS_PER_ELEMENT_BYTES: usize =
    2 * size_of::<Vec<u8>>() + size_of::<u64>() + 3 * size_of::<u32>();
// A group without a recorded pair yet.
const NO_FRAME: u32 = u32::MAX;

/// One validated source list.
#[derive(Clone, Copy)]
struct Frame {
    time: Time,
    /// True for cycle lists, where every record carries a rank delta.
    cyclic: bool,
}

/// Source positions of validated payloads, shared by whole recipes.
///
/// A frame is one validated source list; it keeps the tick and the list kind.
/// Rank groups of `RANKS_PER_GROUP` consecutive active ranks keep, for every
/// frame where the group reports records, the frame number and the position of
/// its first record: the payload offset and the rank inside the group. Streams
/// are kept per group and stay in ascending frame order, so a query walks them
/// forward, seeks straight to the selected runs and decodes them again with the
/// checked scalar decoder. A representation that does not fit is dropped and
/// the sequential walker answers instead.
pub(super) struct Index {
    frames: Vec<Frame>,
    /// Frame numbers per group: absolute first, then `(delta, repeat)` runs.
    ids: Vec<Vec<u8>>,
    /// `(offset delta, rank in group)` per (group, frame) pair.
    offs: Vec<Vec<u8>>,
    limit: usize,
    allocated: usize,
    // Build state, released by `finish` so it is not retained.
    open: bool,
    current: u32,
    last_group: usize,
    last_frame: Vec<u32>,
    last_offset: Vec<u64>,
    pending_delta: Vec<u32>,
    pending_run: Vec<u32>,
    // Query work counters, kept separate from the retained size.
    #[cfg(test)]
    frames_scanned: usize,
    #[cfg(test)]
    payloads_decoded: usize,
    #[cfg(test)]
    payloads_skipped: usize,
}

/// Query work of the last indexed read.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Work {
    pub(super) frames_scanned: usize,
    pub(super) payloads_decoded: usize,
    pub(super) payloads_skipped: usize,
}

impl Index {
    pub(super) fn new(active: usize) -> Option<Self> {
        Self::with_limit(active, INDEX_BYTES)
    }

    fn with_limit(active: usize, limit: usize) -> Option<Self> {
        let count = active.div_ceil(RANKS_PER_GROUP);
        // Aggregate preflight before any reserve: an oversized active table is
        // refused without requesting optional capacity. The post-reserve check
        // below keeps the accounting exact if the allocator rounds up.
        if count.checked_mul(GROUPS_PER_ELEMENT_BYTES)? > limit {
            return None;
        }
        let mut ids = Vec::new();
        ids.try_reserve_exact(count).ok()?;
        let mut offs = Vec::new();
        offs.try_reserve_exact(count).ok()?;
        let mut last_frame = Vec::new();
        last_frame.try_reserve_exact(count).ok()?;
        let mut last_offset = Vec::new();
        last_offset.try_reserve_exact(count).ok()?;
        let mut pending_delta = Vec::new();
        pending_delta.try_reserve_exact(count).ok()?;
        let mut pending_run = Vec::new();
        pending_run.try_reserve_exact(count).ok()?;
        let allocated = (ids.capacity() + offs.capacity()) * size_of::<Vec<u8>>()
            + (last_frame.capacity() + pending_delta.capacity() + pending_run.capacity())
                * size_of::<u32>()
            + last_offset.capacity() * size_of::<u64>();
        if allocated > limit {
            return None;
        }
        ids.resize_with(count, Vec::new);
        offs.resize_with(count, Vec::new);
        last_frame.resize(count, NO_FRAME);
        last_offset.resize(count, 0);
        pending_delta.resize(count, 0);
        pending_run.resize(count, 0);
        Some(Self {
            frames: Vec::new(),
            ids,
            offs,
            limit,
            allocated,
            open: false,
            current: 0,
            last_group: NO_GROUP,
            last_frame,
            last_offset,
            pending_delta,
            pending_run,
            #[cfg(test)]
            frames_scanned: 0,
            #[cfg(test)]
            payloads_decoded: 0,
            #[cfg(test)]
            payloads_skipped: 0,
        })
    }

    /// False discards the optional index; body validation must still finish.
    pub(super) fn push(&mut self, rank: usize, time: Time, offset: u64, cyclic: bool) -> bool {
        if self.last_frame.is_empty() {
            return false;
        }
        if !self.open {
            if !self.grow_frames() {
                return false;
            }
            let Ok(current) = u32::try_from(self.frames.len()) else {
                return false;
            };
            self.frames.push(Frame { time, cyclic });
            self.current = current;
            self.open = true;
            self.last_group = NO_GROUP;
        }
        let group = rank / RANKS_PER_GROUP;
        if group == self.last_group {
            return true;
        }
        self.last_group = group;
        self.record(group, rank, offset)
    }

    /// Close the current frame so the next record starts a new one.
    pub(super) fn list_end(&mut self) {
        self.open = false;
        self.last_group = NO_GROUP;
    }

    /// Flush pending runs and release build-only arrays. False means the
    /// representation could not be completed and the whole index must be
    /// discarded: an incomplete index is never kept.
    pub(super) fn finish(&mut self) -> bool {
        if self.last_frame.is_empty() {
            return true;
        }
        for group in 0..self.ids.len() {
            if !self.flush(group) {
                return false;
            }
        }
        self.last_frame = Vec::new();
        self.last_offset = Vec::new();
        self.pending_delta = Vec::new();
        self.pending_run = Vec::new();
        self.allocated = self.retained();
        true
    }

    /// Bytes of the retained representation.
    fn retained(&self) -> usize {
        self.frames.capacity() * size_of::<Frame>()
            + (self.ids.capacity() + self.offs.capacity()) * size_of::<Vec<u8>>()
            + self.ids.iter().map(Vec::capacity).sum::<usize>()
            + self.offs.iter().map(Vec::capacity).sum::<usize>()
    }

    /// Retained bytes of the completed representation.
    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.retained()
    }

    #[cfg(test)]
    pub(super) fn work(&self) -> Work {
        Work {
            frames_scanned: self.frames_scanned,
            payloads_decoded: self.payloads_decoded,
            payloads_skipped: self.payloads_skipped,
        }
    }

    #[cfg(test)]
    fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    fn grow_frames(&mut self) -> bool {
        if self.frames.len() < self.frames.capacity() {
            return true;
        }
        let remaining = self.limit - self.allocated;
        // Geometric growth keeps the build amortized linear; the reserve is
        // capped by the remaining budget, so slack stays bounded.
        let chunk = if self.frames.is_empty() {
            1
        } else {
            self.frames.capacity().div_ceil(2).max(FRAME_CHUNK)
        };
        let chunk = chunk.min(remaining / size_of::<Frame>());
        if chunk == 0 {
            return false;
        }
        let before = self.frames.capacity();
        if self.frames.try_reserve_exact(chunk).is_err() {
            return false;
        }
        self.allocated += (self.frames.capacity() - before) * size_of::<Frame>();
        self.allocated <= self.limit
    }

    fn grow_stream(
        stream: &mut Vec<u8>,
        needed: usize,
        limit: usize,
        allocated: &mut usize,
    ) -> bool {
        if stream.len() + needed <= stream.capacity() {
            return true;
        }
        let remaining = limit - *allocated;
        if needed > remaining {
            return false;
        }
        // Small streams start small: one chunk per group would cost more than
        // the positions it stores.
        let chunk = needed
            .max(stream.capacity().div_ceil(2))
            .max(STREAM_START)
            .min(remaining);
        let before = stream.capacity();
        if stream.try_reserve_exact(chunk).is_err() {
            return false;
        }
        *allocated += stream.capacity() - before;
        *allocated <= limit
    }

    /// Record the first pair of one group in the current frame.
    fn record(&mut self, group: usize, rank: usize, offset: u64) -> bool {
        let frame = self.current;
        let within = (rank % RANKS_PER_GROUP) as u64;
        let mut bytes = [0u8; 20];
        let mut used = 0;
        if self.last_frame[group] == NO_FRAME {
            // The first frame number and offset of a group are absolute.
            used += encode(u64::from(frame), &mut bytes[used..]);
        } else {
            let delta = frame - self.last_frame[group];
            if delta == self.pending_delta[group] {
                self.pending_run[group] += 1;
            } else {
                if !self.flush(group) {
                    return false;
                }
                self.pending_delta[group] = delta;
                self.pending_run[group] = 1;
            }
        }
        let mut position = [0u8; 20];
        let offset_used = encode(offset - self.last_offset[group], &mut position);
        let position_used = offset_used + encode(within, &mut position[offset_used..]);
        if !Self::grow_stream(
            &mut self.offs[group],
            position_used,
            self.limit,
            &mut self.allocated,
        ) || !Self::grow_stream(&mut self.ids[group], used, self.limit, &mut self.allocated)
        {
            return false;
        }
        self.offs[group].extend_from_slice(&position[..position_used]);
        self.ids[group].extend_from_slice(&bytes[..used]);
        self.last_frame[group] = frame;
        self.last_offset[group] = offset;
        true
    }

    /// Write the pending run of one group as a delta and repeat count.
    fn flush(&mut self, group: usize) -> bool {
        let run = self.pending_run[group];
        if run == 0 {
            return true;
        }
        let mut bytes = [0u8; 10];
        let used = encode(u64::from(self.pending_delta[group]), &mut bytes);
        let used = used + encode(u64::from(run), &mut bytes[used..]);
        if !Self::grow_stream(&mut self.ids[group], used, self.limit, &mut self.allocated) {
            return false;
        }
        self.ids[group].extend_from_slice(&bytes[..used]);
        self.pending_run[group] = 0;
        true
    }

    /// Read the selected recipes from validated source positions.
    ///
    /// `None` means the query scratch could not be reserved and the caller
    /// must use the sequential walker instead.
    pub(super) fn read<B>(
        &mut self,
        wire: &mut Wire,
        types: &Types,
        active: &[(u32, usize)],
        ids: &[u32],
        end: Time,
        mut visitor: impl FnMut(body::Event) -> ControlFlow<B>,
    ) -> Result<Option<ControlFlow<B>>> {
        #[cfg(test)]
        {
            self.frames_scanned = 0;
            self.payloads_decoded = 0;
            self.payloads_skipped = 0;
        }
        let mut ranks = Vec::new();
        if ranks.try_reserve_exact(ids.len()).is_err() {
            return Ok(None);
        }
        for &id in ids {
            let rank = active
                .binary_search_by_key(&id, |&(id, _)| id)
                .map_err(|_| wire.malformed("inactive private source ID"))?;
            ranks.push(rank);
        }
        ranks.sort_unstable();
        ranks.dedup();
        let mut cursors = Vec::new();
        if cursors.try_reserve_exact(ranks.len()).is_err() {
            return Ok(None);
        }
        // One cursor per group, carrying the group's own last selected rank so
        // a scan never decodes trailing payloads after it.
        let mut position = 0;
        while position < ranks.len() {
            let group = ranks[position] / RANKS_PER_GROUP;
            let mut last = position;
            while last + 1 < ranks.len() && ranks[last + 1] / RANKS_PER_GROUP == group {
                last += 1;
            }
            position = last + 1;
            if self.ids[group].is_empty() {
                continue;
            }
            cursors.push(Cursor {
                group,
                last: ranks[last],
                ids: 0,
                offs: 0,
                started: false,
                frame: 0,
                delta: 0,
                remaining: 0,
                offset: 0,
                first: 0,
            });
        }
        let mut heap = BinaryHeap::new();
        if heap.try_reserve(cursors.len()).is_err() {
            return Ok(None);
        }
        for (slot, cursor) in cursors.iter_mut().enumerate() {
            if self.advance(cursor) {
                heap.push(Reverse((cursor.frame, slot)));
            }
        }
        let mut pending = Vec::new();
        if pending.try_reserve_exact(cursors.len()).is_err() {
            return Ok(None);
        }
        let mut current = None;
        while let Some(Reverse((frame, _))) = heap.peek().copied() {
            let stored = self.frames[frame as usize];
            if stored.time > end {
                break;
            }
            if let Some(previous) = current
                && previous != stored.time
                && let ControlFlow::Break(value) = tick(&mut visitor, previous)
            {
                return Ok(Some(ControlFlow::Break(value)));
            }
            current = Some(stored.time);
            pending.clear();
            while let Some(Reverse((next, slot))) = heap.peek().copied() {
                if next != frame {
                    break;
                }
                heap.pop();
                pending.push(slot);
            }
            for &slot in &pending {
                let cursor = &cursors[slot];
                let sources = Sources {
                    active,
                    types,
                    ranks: &ranks,
                };
                if let ControlFlow::Break(value) =
                    self.scan(wire, sources, stored, cursor, &mut visitor)?
                {
                    return Ok(Some(ControlFlow::Break(value)));
                }
            }
            for &slot in &pending {
                if self.advance(&mut cursors[slot]) {
                    heap.push(Reverse((cursors[slot].frame, slot)));
                }
            }
        }
        if let Some(time) = current
            && let ControlFlow::Break(value) = tick(&mut visitor, time)
        {
            return Ok(Some(ControlFlow::Break(value)));
        }
        Ok(Some(ControlFlow::Continue(())))
    }

    /// Decode the next frame number and position of one group.
    fn advance(&self, cursor: &mut Cursor) -> bool {
        let ids = &self.ids[cursor.group];
        let mut position = cursor.ids as usize;
        if !cursor.started {
            let Some(first) = decode(ids, &mut position) else {
                return false;
            };
            let Ok(first) = u32::try_from(first) else {
                return false;
            };
            cursor.frame = first;
            cursor.started = true;
        } else if cursor.remaining > 0 {
            cursor.remaining -= 1;
            cursor.frame += cursor.delta;
        } else {
            if position >= ids.len() {
                return false;
            }
            let (Some(delta), Some(run)) = (decode(ids, &mut position), decode(ids, &mut position))
            else {
                return false;
            };
            let (Ok(delta), Ok(run)) = (u32::try_from(delta), u32::try_from(run)) else {
                return false;
            };
            if run == 0 {
                return false;
            }
            cursor.delta = delta;
            cursor.remaining = run - 1;
            cursor.frame += delta;
        }
        cursor.ids = position as u32;
        let offs = &self.offs[cursor.group];
        let mut position = cursor.offs as usize;
        if position >= offs.len() {
            return false;
        }
        let (Some(delta), Some(first)) = (decode(offs, &mut position), decode(offs, &mut position))
        else {
            return false;
        };
        cursor.offset += delta;
        cursor.first = first as usize;
        cursor.offs = position as u32;
        true
    }

    /// Decode the records of one group in one frame, reporting selected ranks.
    fn scan<B>(
        &mut self,
        wire: &mut Wire,
        sources: Sources<'_>,
        frame: Frame,
        cursor: &Cursor,
        visitor: &mut impl FnMut(body::Event) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        let last = cursor.last;
        let base = cursor.group * RANKS_PER_GROUP;
        let group_end = (base + RANKS_PER_GROUP).min(sources.active.len());
        let mut rank = base + cursor.first;
        if rank >= group_end || rank > last {
            return Ok(ControlFlow::Continue(()));
        }
        #[cfg(test)]
        {
            self.frames_scanned += 1;
        }
        wire.seek_value(cursor.offset)?;
        loop {
            let (id, ty) = sources.active[rank];
            if sources.ranks.binary_search(&rank).is_ok() {
                let offset = wire.offset;
                let value = body::scalar(wire, sources.types, ty)?;
                #[cfg(test)]
                {
                    self.payloads_decoded += 1;
                }
                if let ControlFlow::Break(value) = visitor(body::Event::Scalar {
                    id,
                    rank,
                    time: frame.time,
                    offset,
                    value,
                    cyclic: frame.cyclic,
                }) {
                    return Ok(ControlFlow::Break(value));
                }
            } else {
                skip(wire, sources.types, ty)?;
                #[cfg(test)]
                {
                    self.payloads_skipped += 1;
                }
            }
            if !frame.cyclic {
                rank += 1;
            } else {
                let delta = wire.uleb()? as usize;
                if delta == 0 {
                    break;
                }
                rank = rank
                    .checked_add(delta)
                    .ok_or_else(|| wire.malformed("cycle scalar rank overflow"))?;
            }
            if rank >= group_end || rank >= sources.active.len() || rank > last {
                break;
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// Read-only inputs shared by every frame of one query.
#[derive(Clone, Copy)]
struct Sources<'a> {
    active: &'a [(u32, usize)],
    types: &'a Types,
    ranks: &'a [usize],
}

/// Decoding state of one group's position streams.
struct Cursor {
    group: usize,
    /// Largest selected rank of this group.
    last: usize,
    ids: u32,
    offs: u32,
    started: bool,
    frame: u32,
    delta: u32,
    remaining: u32,
    offset: u64,
    first: usize,
}

/// Skip one validated payload without building its value.
fn skip(wire: &mut Wire, types: &Types, ty: usize) -> Result<()> {
    let (storage, _, _) = types
        .scalar(ty)
        .ok_or_else(|| wire.malformed("non-scalar body type"))?;
    match storage {
        Storage::B2 | Storage::E8 => {
            wire.byte()?;
        }
        Storage::F64 => {
            wire.fixed::<8>()?;
        }
        Storage::I32 | Storage::I64 | Storage::P32 | Storage::P64 => loop {
            if wire.byte()? & 0x80 == 0 {
                break;
            }
        },
    }
    Ok(())
}

fn tick<B>(visitor: &mut impl FnMut(body::Event) -> ControlFlow<B>, time: Time) -> ControlFlow<B> {
    if let ControlFlow::Break(value) = visitor(body::Event::ListEnd(time)) {
        return ControlFlow::Break(value);
    }
    visitor(body::Event::Tick(time))
}

fn encode(mut value: u64, out: &mut [u8]) -> usize {
    let mut used = 0;
    while value >= 0x80 {
        out[used] = (value as u8) | 0x80;
        value >>= 7;
        used += 1;
    }
    out[used] = value as u8;
    used + 1
}

fn decode(bytes: &[u8], position: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for group in 0..10 {
        let byte = *bytes.get(*position)?;
        *position += 1;
        value |= u64::from(byte & 0x7f) << (7 * group);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_frame(index: &mut Index, time: u64, ranks: &[usize]) {
        for &rank in ranks {
            assert!(index.push(
                rank,
                Time::from_ticks(time),
                100 * time + rank as u64,
                false
            ));
        }
        index.list_end();
    }

    /// Collect one group's `(frame, offset, first rank)` triples in source order.
    fn pairs(index: &Index, group: usize) -> Vec<(u32, u64, usize)> {
        let mut cursor = Cursor {
            group,
            last: usize::MAX,
            ids: 0,
            offs: 0,
            started: false,
            frame: 0,
            delta: 0,
            remaining: 0,
            offset: 0,
            first: 0,
        };
        let mut pairs = Vec::new();
        while index.advance(&mut cursor) {
            pairs.push((cursor.frame, cursor.offset, cursor.first));
        }
        pairs
    }

    #[test]
    fn one_pair_per_group_and_frame_carries_its_own_position() {
        let mut index = Index::with_limit(2 * RANKS_PER_GROUP, 1 << 24).unwrap();
        push_frame(&mut index, 0, &[3, 9, 70]);
        push_frame(&mut index, 5, &[3, 70, 71]);
        push_frame(&mut index, 9, &[70]);
        assert!(index.finish());
        // Ranks 3 and 9 share group 0 and frame 0: only the run start is kept.
        assert_eq!(pairs(&index, 0), vec![(0, 3, 3), (1, 503, 3)]);
        assert_eq!(pairs(&index, 1), vec![(0, 70, 6), (1, 570, 6), (2, 970, 6)]);
        assert!(index.retained_bytes() <= index.limit);
    }

    #[test]
    fn repeated_stride_stays_lossless_and_compact() {
        let mut index = Index::with_limit(RANKS_PER_GROUP, 1 << 24).unwrap();
        for frame in 0..100u64 {
            push_frame(&mut index, frame, &[0, 1]);
        }
        assert!(index.finish());
        let expected = (0..100)
            .map(|frame| (frame as u32, 0, 0))
            .collect::<Vec<_>>();
        let actual = pairs(&index, 0)
            .into_iter()
            .map(|(frame, _, first)| (frame, 0, first))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        // Frame numbers use runs; positions stay one pair each.
        assert!(index.ids[0].len() < 8, "{}", index.ids[0].len());
        assert!(index.offs[0].len() >= 100, "{}", index.offs[0].len());
        assert!(index.offs[0].len() <= 200, "{}", index.offs[0].len());
    }

    #[test]
    fn oversized_active_table_is_refused_before_any_reserve() {
        // 600 M ranks is 9 375 000 groups and 637.5 MB of per-group arrays,
        // above the 512 MiB budget, so construction must refuse quickly.
        assert!(Index::new(600_000_000).is_none());
        assert!(Index::with_limit(600_000_000, 1024).is_none());
        // The preflight is exact: one byte below the aggregate is refused too.
        let groups = 600_000_000usize.div_ceil(RANKS_PER_GROUP);
        let aggregate = groups.checked_mul(GROUPS_PER_ELEMENT_BYTES).unwrap();
        assert!(Index::with_limit(600_000_000, aggregate - 1).is_none());
    }

    #[test]
    fn failed_finish_never_yields_an_incomplete_index() {
        let mut index = Index::with_limit(RANKS_PER_GROUP, 1 << 24).unwrap();
        // Alternating strides flush runs during the build; the last run stays
        // pending until `finish`.
        for frame in 0..16u64 {
            push_frame(&mut index, frame * 2 + frame % 2, &[0]);
        }
        // The pending run is only written by `finish`: fill the stream capacity
        // exactly and refuse any growth, so completion cannot succeed.
        let group = 0;
        index.ids[group].shrink_to_fit();
        index.set_limit(index.allocated);
        assert!(!index.finish());
        // The caller must discard the index; nothing is silently completed.
        assert!(!index.ids[group].is_empty());
    }

    #[test]
    fn budget_counts_every_retained_capacity_and_rejects_growth() {
        let limit = 1 << 20;
        let mut index = Index::with_limit(RANKS_PER_GROUP, limit).unwrap();
        for frame in 0..64u64 {
            push_frame(&mut index, frame, &[0, 1]);
        }
        assert!(index.finish());
        let independent = index.frames.capacity() * size_of::<Frame>()
            + (index.ids.capacity() + index.offs.capacity()) * size_of::<Vec<u8>>()
            + index.ids.iter().map(Vec::capacity).sum::<usize>()
            + index.offs.iter().map(Vec::capacity).sum::<usize>();
        assert_eq!(independent, index.retained());
        assert!(index.retained() <= limit);

        // A representation that cannot fit is rejected without overshoot.
        let mut index = Index::with_limit(RANKS_PER_GROUP, 64 * 1024).unwrap();
        let mut frame = 0u64;
        while index.push(0, Time::from_ticks(frame), frame, false) {
            index.list_end();
            frame += 1;
            assert!(frame < 100_000);
        }
        assert!(frame > 0);
        // A refused growth is bounded by the last reserve step, not unbounded.
        assert!(index.retained() <= 64 * 1024 + 2 * index.frames.capacity() * size_of::<Frame>());
    }
}
