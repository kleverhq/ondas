use std::{cmp::Reverse, collections::BinaryHeap, ops::ControlFlow};

use super::{body, types::Types, wire::Wire};
use crate::{Result, Time};

const INDEX_BYTES: usize = 64 * 1024 * 1024;
// Avoid repeated small allocations while opening short histories.
const INITIAL_POSTINGS: usize = 8;

#[derive(Clone, Copy)]
struct Posting {
    time: Time,
    offset: u64,
}

/// Positions of validated source payloads, shared by overlapping whole recipes.
pub(super) struct Index {
    histories: Vec<Vec<Posting>>,
    allocated: usize,
    limit: usize,
}

impl Index {
    pub(super) fn new(scalars: usize) -> Option<Self> {
        Self::with_limit(scalars, INDEX_BYTES)
    }

    fn with_limit(scalars: usize, limit: usize) -> Option<Self> {
        if scalars.checked_mul(size_of::<Vec<Posting>>())? > limit {
            return None;
        }
        let mut histories = Vec::new();
        histories.try_reserve_exact(scalars).ok()?;
        let allocated = histories
            .capacity()
            .checked_mul(size_of::<Vec<Posting>>())?;
        if allocated > limit {
            return None;
        }
        histories.resize_with(scalars, Vec::new);
        Some(Self {
            histories,
            allocated,
            limit,
        })
    }

    /// False discards the optional index; body validation must still finish.
    pub(super) fn push(&mut self, rank: usize, time: Time, offset: u64) -> bool {
        let history = &mut self.histories[rank];
        if let Some(last) = history.last_mut()
            && last.time == time
        {
            // Persistent observations use only the final value of this tick.
            last.offset = offset;
            return true;
        }
        if history.len() == history.capacity() {
            let additional = history.capacity().max(INITIAL_POSTINGS);
            let Some(bytes) = additional.checked_mul(size_of::<Posting>()) else {
                return false;
            };
            if bytes > self.limit - self.allocated {
                return false;
            }
            if history.try_reserve_exact(additional).is_err() {
                return false;
            }
            let allocated = history.capacity() * size_of::<Posting>();
            let previous = history.len() * size_of::<Posting>();
            if allocated - previous > self.limit - self.allocated {
                return false;
            }
            self.allocated += allocated - previous;
        }
        history.push(Posting { time, offset });
        true
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
        let mut heap = BinaryHeap::with_capacity(ids.len());
        for &id in ids {
            let rank = active
                .binary_search_by_key(&id, |&(id, _)| id)
                .map_err(|_| wire.malformed("inactive private source ID"))?;
            if let Some(first) = self.histories[rank].first() {
                heap.push(Reverse((first.offset, rank, 0usize)));
            }
        }
        let mut current = None;
        while let Some(Reverse((offset, rank, position))) = heap.pop() {
            let posting = self.histories[rank][position];
            if posting.time > end {
                break;
            }
            if let Some(previous) = current
                && previous != posting.time
            {
                if let ControlFlow::Break(value) = visitor(body::Event::ListEnd(previous)) {
                    return Ok(ControlFlow::Break(value));
                }
                if let ControlFlow::Break(value) = visitor(body::Event::Tick(previous)) {
                    return Ok(ControlFlow::Break(value));
                }
            }
            current = Some(posting.time);
            wire.seek_value(offset)?;
            let (id, ty) = active[rank];
            let value = body::scalar(wire, types, ty)?;
            if let ControlFlow::Break(value) = visitor(body::Event::Scalar {
                id,
                rank,
                time: posting.time,
                offset,
                value,
            }) {
                return Ok(ControlFlow::Break(value));
            }
            let next = position + 1;
            if let Some(posting) = self.histories[rank].get(next) {
                heap.push(Reverse((posting.offset, rank, next)));
            }
        }
        if let Some(time) = current {
            if let ControlFlow::Break(value) = visitor(body::Event::ListEnd(time)) {
                return Ok(ControlFlow::Break(value));
            }
            if let ControlFlow::Break(value) = visitor(body::Event::Tick(time)) {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_keep_final_same_tick_payload_and_account_capacity() {
        let mut index = Index::with_limit(2, 256).unwrap();
        assert!(index.push(0, Time::from_ticks(3), 10));
        let bytes = index.allocated;
        assert!(index.push(0, Time::from_ticks(3), 20));
        assert_eq!(index.histories[0].len(), 1);
        assert_eq!(index.histories[0][0].offset, 20);
        assert_eq!(index.allocated, bytes);
        for tick in 4..11 {
            assert!(index.push(0, Time::from_ticks(tick), tick * 10));
        }
        assert_eq!(
            index.allocated,
            index.histories.capacity() * size_of::<Vec<Posting>>()
                + index
                    .histories
                    .iter()
                    .map(|h| h.capacity() * size_of::<Posting>())
                    .sum::<usize>()
        );
        assert!(!index.push(0, Time::from_ticks(11), 110));
        assert!(index.allocated <= index.limit);
    }
}
