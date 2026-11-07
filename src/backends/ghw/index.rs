use std::{cmp::Reverse, collections::BinaryHeap, ops::ControlFlow};

use super::{body, types::Types, wire::Wire};
use crate::{Result, Time};

const INDEX_BYTES: usize = 64 * 1024 * 1024;
// Consecutive active ranks covered by one chain. A chain records validated
// source positions only, never values or query results.
pub(super) const RANKS_PER_CHAIN: usize = 64;
// Chunked storage avoids growth copies and unused tail capacity.
const CHUNK_BITS: u32 = 20;
const CHUNK_BYTES: usize = 1 << CHUNK_BITS;
// A chain without a recorded segment yet.
const NO_SEGMENT: u32 = u32::MAX;

/// The newest validated source segment of one chain.
#[derive(Clone, Copy)]
struct Chain {
    last: u32,
    time: Time,
    offset: u64,
}

/// Decoding state of one chain inside a query.
struct Cursor {
    chain: usize,
    base: usize,
    len: usize,
    consumed: usize,
    time: Time,
    offset: u64,
    first: usize,
    cyclic: bool,
}

/// Append-only byte arena addressed by one contiguous virtual offset.
struct Arena {
    chunks: Vec<Vec<u8>>,
    len: u32,
    allocated: usize,
}

impl Arena {
    fn new() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
            allocated: 0,
        }
    }

    fn push(&mut self, bytes: &[u8], limit: usize) -> Option<u32> {
        let start = self.len;
        let mut written = 0;
        while written < bytes.len() {
            let used = (self.len as usize) & (CHUNK_BYTES - 1);
            if self.chunks.is_empty() || used == 0 {
                let mut chunk = Vec::new();
                chunk.try_reserve_exact(CHUNK_BYTES).ok()?;
                if self.allocated + chunk.capacity() > limit {
                    return None;
                }
                self.allocated += chunk.capacity();
                let previous = self.chunks.capacity();
                self.chunks.try_reserve(1).ok()?;
                self.allocated += self.chunks.capacity() - previous;
                self.chunks.push(chunk);
            }
            let chunk = self.chunks.last_mut()?;
            let take = (bytes.len() - written).min(CHUNK_BYTES - used);
            chunk.extend_from_slice(&bytes[written..written + take]);
            written += take;
            self.len += take as u32;
        }
        Some(start)
    }

    fn byte(&self, address: &mut u32) -> Option<u8> {
        if *address >= self.len {
            return None;
        }
        let chunk = (*address >> CHUNK_BITS) as usize;
        let byte = *self
            .chunks
            .get(chunk)?
            .get((*address as usize) & (CHUNK_BYTES - 1))?;
        *address += 1;
        Some(byte)
    }
}

/// Positions of validated source payloads, shared by overlapping whole recipes.
///
/// Each active rank belongs to one chain of `RANKS_PER_CHAIN` consecutive
/// ranks. A source list reports ranks in ascending order, so the records of
/// one chain in one list form one contiguous byte segment. The index stores
/// only the start of that segment: tick, payload offset and first rank. A
/// query decodes the segment again with the checked scalar decoder.
pub(super) struct Index {
    chains: Vec<Chain>,
    arena: Arena,
    limit: usize,
    projected: u32,
    open: bool,
    chain: usize,
    time: Time,
    offset: u64,
}

impl Index {
    pub(super) fn new(active: usize) -> Option<Self> {
        Self::with_limit(active, INDEX_BYTES)
    }

    fn with_limit(active: usize, limit: usize) -> Option<Self> {
        let count = active.div_ceil(RANKS_PER_CHAIN);
        let mut chains = Vec::new();
        chains.try_reserve_exact(count).ok()?;
        let allocated = chains.capacity().checked_mul(size_of::<Chain>())?;
        if allocated > limit {
            return None;
        }
        chains.resize(
            count,
            Chain {
                last: NO_SEGMENT,
                time: Time::ZERO,
                offset: 0,
            },
        );
        Some(Self {
            chains,
            arena: Arena {
                allocated,
                ..Arena::new()
            },
            limit,
            projected: 0,
            open: false,
            chain: 0,
            time: Time::ZERO,
            offset: 0,
        })
    }

    /// False discards the optional index; body validation must still finish.
    pub(super) fn push(&mut self, rank: usize, time: Time, offset: u64, cyclic: bool) -> bool {
        let chain = rank / RANKS_PER_CHAIN;
        if self.open && chain == self.chain && time == self.time {
            // Continuing records of one list stay in the current segment.
            return true;
        }
        let tail = self.chains[chain];
        // A first segment links to nothing and keeps absolute fields, so all
        // four fields of a header decode with one accumulator.
        let (link, delta_time, delta_offset) = if tail.last == NO_SEGMENT {
            (0, time.ticks(), offset)
        } else {
            (
                u64::from(self.arena.len - tail.last),
                time.ticks() - tail.time.ticks(),
                offset - tail.offset,
            )
        };
        let within = ((rank % RANKS_PER_CHAIN) as u64) << 1 | u64::from(cyclic);
        let mut bytes = [0u8; 40];
        let mut used = 0;
        for field in [link, delta_time, delta_offset, within] {
            used += encode(field, &mut bytes[used..]);
        }
        let Some(start) = self.arena.push(&bytes[..used], self.limit) else {
            return false;
        };
        self.chains[chain] = Chain {
            last: start,
            time,
            offset,
        };
        self.open = true;
        self.chain = chain;
        self.time = time;
        self.offset = offset;
        true
    }

    /// True when the representation already seen cannot fit once the whole
    /// body is validated. Checked once per storage chunk, so a source that
    /// cannot be indexed stops paying for the attempt.
    pub(super) fn projected_overrun(&mut self, source: u64, total: u64) -> bool {
        let chunks = self.arena.len >> CHUNK_BITS;
        if chunks == 0 || self.arena.len & (CHUNK_BYTES as u32 - 1) != 0 {
            return false;
        }
        if chunks <= self.projected {
            return false;
        }
        self.projected = chunks;
        u128::from(self.arena.len) * u128::from(total)
            > u128::from(self.limit as u64) * u128::from(source)
    }

    /// Close the current segment so the next record starts a new one.
    pub(super) fn list_end(&mut self) {
        self.open = false;
    }

    pub(super) fn read<B>(
        &self,
        wire: &mut Wire,
        types: &Types,
        active: &[(u32, usize)],
        ids: &[u32],
        end: Time,
        mut visitor: impl FnMut(body::Event) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        let mut chains = Vec::with_capacity(ids.len());
        for &id in ids {
            let rank = active
                .binary_search_by_key(&id, |&(id, _)| id)
                .map_err(|_| wire.malformed("inactive private source ID"))?;
            chains.push(rank / RANKS_PER_CHAIN);
        }
        chains.sort_unstable();
        chains.dedup();
        // One flat list of segment starts. Chains share the arena in source
        // order, so backward links recover each chain's segments oldest-last.
        let mut offsets = Vec::new();
        let mut cursors: Vec<Cursor> = Vec::with_capacity(chains.len());
        for chain in chains {
            let base = offsets.len();
            let mut position = self.chains[chain].last;
            while position != NO_SEGMENT {
                offsets.push(position);
                let mut address = position;
                let Some(link) = decode(&self.arena, &mut address) else {
                    return Err(wire.malformed("truncated source segment link"));
                };
                if link == 0 {
                    break;
                }
                let Ok(link) = u32::try_from(link) else {
                    return Err(wire.malformed("source segment link exceeds input"));
                };
                let Some(previous) = position.checked_sub(link) else {
                    return Err(wire.malformed("source segment link underflow"));
                };
                position = previous;
            }
            cursors.push(Cursor {
                chain,
                base,
                len: offsets.len() - base,
                consumed: 0,
                time: Time::ZERO,
                offset: 0,
                first: 0,
                cyclic: false,
            });
        }
        let mut heap = BinaryHeap::with_capacity(cursors.len());
        for (slot, cursor) in cursors.iter_mut().enumerate() {
            if self.advance(&offsets, cursor) {
                heap.push(Reverse((cursor.time, slot)));
            }
        }
        let mut current = None;
        while let Some(Reverse((time, slot))) = heap.pop() {
            if time > end {
                break;
            }
            if let Some(previous) = current
                && previous != time
                && let ControlFlow::Break(value) = tick(&mut visitor, previous)
            {
                return Ok(ControlFlow::Break(value));
            }
            current = Some(time);
            let cursor = &mut cursors[slot];
            if let ControlFlow::Break(value) =
                self.replay(wire, types, active, cursor, &mut visitor)?
            {
                return Ok(ControlFlow::Break(value));
            }
            if self.advance(&offsets, cursor) {
                heap.push(Reverse((cursor.time, slot)));
            }
        }
        if let Some(time) = current
            && let ControlFlow::Break(value) = tick(&mut visitor, time)
        {
            return Ok(ControlFlow::Break(value));
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Decode the next segment header of one chain, oldest first.
    fn advance(&self, offsets: &[u32], cursor: &mut Cursor) -> bool {
        if cursor.consumed >= cursor.len {
            return false;
        }
        let position = cursor.base + cursor.len - 1 - cursor.consumed;
        let mut address = offsets[position];
        let (Some(_link), Some(time), Some(offset), Some(first)) = (
            decode(&self.arena, &mut address),
            decode(&self.arena, &mut address),
            decode(&self.arena, &mut address),
            decode(&self.arena, &mut address),
        ) else {
            return false;
        };
        cursor.consumed += 1;
        cursor.time = Time::from_ticks(cursor.time.ticks() + time);
        cursor.offset += offset;
        cursor.first = (first >> 1) as usize;
        cursor.cyclic = first & 1 != 0;
        true
    }

    /// Decode the records of one segment. A rank leaving the chain or a list
    /// terminator ends the segment, so no stored end offset is needed.
    fn replay<B>(
        &self,
        wire: &mut Wire,
        types: &Types,
        active: &[(u32, usize)],
        cursor: &Cursor,
        visitor: &mut impl FnMut(body::Event) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        let base = cursor.chain * RANKS_PER_CHAIN;
        let chain_end = (base + RANKS_PER_CHAIN).min(active.len());
        let mut rank = base + cursor.first;
        wire.seek_value(cursor.offset)?;
        loop {
            let (id, ty) = active[rank];
            let offset = wire.offset;
            let value = body::scalar(wire, types, ty)?;
            if let ControlFlow::Break(value) = visitor(body::Event::Scalar {
                id,
                rank,
                time: cursor.time,
                offset,
                value,
                cyclic: cursor.cyclic,
            }) {
                return Ok(ControlFlow::Break(value));
            }
            if !cursor.cyclic {
                rank += 1;
                if rank >= chain_end {
                    return Ok(ControlFlow::Continue(()));
                }
                continue;
            }
            let delta = wire.uleb()? as usize;
            if delta == 0 {
                return Ok(ControlFlow::Continue(()));
            }
            rank = rank
                .checked_add(delta)
                .ok_or_else(|| wire.malformed("cycle scalar rank overflow"))?;
            if rank >= chain_end {
                return Ok(ControlFlow::Continue(()));
            }
        }
    }
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

fn decode(arena: &Arena, address: &mut u32) -> Option<u64> {
    let mut value = 0u64;
    for group in 0..10 {
        let byte = arena.byte(address)?;
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

    /// Collect one chain's segments in source order, as a query does.
    fn segments(index: &Index, chain: usize) -> Vec<(Time, u64, usize, bool)> {
        let mut offsets = Vec::new();
        let mut position = index.chains[chain].last;
        while position != NO_SEGMENT {
            offsets.push(position);
            let mut address = position;
            let link = decode(&index.arena, &mut address).unwrap();
            if link == 0 {
                break;
            }
            position -= u32::try_from(link).unwrap();
        }
        let mut cursor = Cursor {
            chain,
            base: 0,
            len: offsets.len(),
            consumed: 0,
            time: Time::ZERO,
            offset: 0,
            first: 0,
            cyclic: false,
        };
        let mut segments = Vec::new();
        while index.advance(&offsets, &mut cursor) {
            segments.push((cursor.time, cursor.offset, cursor.first, cursor.cyclic));
        }
        segments
    }

    #[test]
    fn interleaved_chains_keep_their_own_segments_in_source_order() {
        let mut index = Index::with_limit(2 * RANKS_PER_CHAIN, 1 << 24).unwrap();
        assert!(index.push(3, Time::from_ticks(7), 10, false));
        assert!(index.push(9, Time::from_ticks(7), 40, false));
        // Another chain starts its own segment, then the first chain returns.
        assert!(index.push(70, Time::from_ticks(7), 50, true));
        index.list_end();
        assert!(index.push(3, Time::from_ticks(9), 80, false));
        // Chain times and offsets stay relative to their own chain, and the
        // shared arena keeps source order for both chains.
        let first = segments(&index, 0);
        let second = segments(&index, 1);
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!((first[1].0, first[1].1), (Time::from_ticks(9), 80));
        assert_eq!((second[0].0, second[0].1), (Time::from_ticks(7), 50));
        assert_eq!((second[0].2, second[0].3), (6, true));
        // Continuing records of the same chain and list add no metadata.
        let len = index.arena.len;
        assert!(index.push(5, Time::from_ticks(9), 90, false));
        assert_eq!(index.arena.len, len);
        assert!(index.arena.allocated <= index.limit);
    }

    #[test]
    fn overflow_discards_the_optional_index() {
        let mut index = Index::with_limit(RANKS_PER_CHAIN, CHUNK_BYTES + 4096).unwrap();
        let mut tick = 0;
        while index.push(0, Time::from_ticks(tick), tick * 10, false) {
            index.list_end();
            tick += 1;
        }
        assert!(tick > 0);
        assert!(index.arena.allocated <= index.limit);
    }
}
