//! A bounded index of validated VCD record positions.
//!
//! The index keeps only source positions of records that opening already
//! validated. It stores no values, no source bytes and no query state. A
//! representation that does not fit the private budgets is dropped and the
//! chronological paths answer instead.

use std::mem::size_of;

pub(super) const ENTRY_BYTES: usize = size_of::<Entry>();
pub(super) const LIST_BYTES: usize = size_of::<Vec<Entry>>();
const TICK_BYTES: usize = size_of::<(u32, u64)>();
const BLOCK_BYTES: usize = size_of::<(u64, u64)>();
const RECORD_BYTES: usize = size_of::<u32>() + size_of::<u64>();
const CAPABILITY_BYTES: usize = size_of::<u64>();

/// The validated time and source offset of one record token.
#[derive(Clone, Copy)]
pub(super) struct Entry {
    pub(super) time: u64,
    pub(super) offset: u64,
}

/// Transient records of one opening walk, in source order.
///
/// A record keeps its signal and offset; the tick table turns consecutive
/// records with equal time into one `(first record, time)` pair. A record costs
/// 12 bytes, a recorded tick or dump block costs 16 bytes, and one raw-token
/// high-water mark per declared signal costs 8 bytes. The budget covers the
/// actual capacities, not the current lengths, so an input with a new tick on
/// every record or with many blocks cannot overrun it.
#[derive(Default)]
pub(super) struct Builder {
    signals: Vec<u32>,
    offsets: Vec<u64>,
    ticks: Vec<(u32, u64)>,
    blocks: Vec<(u64, u64)>,
    /// Largest raw payload/identifier token length seen per declared signal.
    max_payload: Vec<u64>,
    max_identifier: u64,
    time: u64,
    allocated: usize,
    limit: usize,
    overflow: bool,
}

impl Builder {
    pub(super) fn new(signals: usize, limit: usize) -> Self {
        let mut builder = Self {
            limit,
            ..Self::default()
        };
        let bytes = signals.saturating_mul(CAPABILITY_BYTES);
        let mut max_payload = Vec::new();
        if builder.exceeds(bytes) || max_payload.try_reserve_exact(signals).is_err() {
            builder.overflow = true;
            return builder;
        }
        max_payload.resize(signals, 0);
        builder.max_payload = max_payload;
        builder.allocated = bytes;
        builder
    }

    pub(super) fn record(&mut self, signal: usize, time: u64, offset: u64, token: (u64, u64)) {
        if self.overflow {
            return;
        }
        let Ok(signal) = u32::try_from(signal) else {
            self.abandon();
            return;
        };
        let Some(max_payload) = self.max_payload.get_mut(signal as usize) else {
            self.abandon();
            return;
        };
        *max_payload = (*max_payload).max(token.0);
        self.max_identifier = self.max_identifier.max(token.1);
        if self.time != time || self.signals.is_empty() {
            let Ok(next) = u32::try_from(self.offsets.len()) else {
                self.abandon();
                return;
            };
            if !self.grow_ticks() {
                return;
            }
            self.ticks.push((next, time));
            self.time = time;
        }
        if !self.grow_records() {
            return;
        }
        self.signals.push(signal);
        self.offsets.push(offset);
    }

    /// Records one dump block, whose records are snapshots rather than occurrences.
    pub(super) fn block(&mut self, start: u64, end: u64) {
        if self.overflow {
            return;
        }
        if self.blocks.len() == self.blocks.capacity() {
            let capacity = self.blocks.capacity().max(16).saturating_mul(2);
            let bytes = (capacity - self.blocks.capacity()).saturating_mul(BLOCK_BYTES);
            if self.exceeds(bytes)
                || self
                    .blocks
                    .try_reserve_exact(capacity - self.blocks.len())
                    .is_err()
            {
                self.abandon();
                return;
            }
            self.allocated += bytes;
        }
        self.blocks.push((start, end));
    }

    fn grow_records(&mut self) -> bool {
        if self.signals.len() < self.signals.capacity() {
            return true;
        }
        let capacity = self.signals.capacity().max(1024).saturating_mul(2);
        let extra = capacity - self.signals.len();
        let bytes = extra.saturating_mul(RECORD_BYTES);
        if self.exceeds(bytes)
            || self.signals.try_reserve_exact(extra).is_err()
            || self.offsets.try_reserve_exact(extra).is_err()
        {
            self.abandon();
            return false;
        }
        self.allocated += bytes;
        true
    }

    fn grow_ticks(&mut self) -> bool {
        if self.ticks.len() < self.ticks.capacity() {
            return true;
        }
        let capacity = self.ticks.capacity().max(64).saturating_mul(2);
        let extra = capacity - self.ticks.len();
        let bytes = extra.saturating_mul(TICK_BYTES);
        if self.exceeds(bytes) || self.ticks.try_reserve_exact(extra).is_err() {
            self.abandon();
            return false;
        }
        self.allocated += bytes;
        true
    }

    fn exceeds(&self, bytes: usize) -> bool {
        self.allocated.saturating_add(bytes) > self.limit
    }

    fn abandon(&mut self) {
        self.overflow = true;
        self.signals = Vec::new();
        self.offsets = Vec::new();
        self.ticks = Vec::new();
        self.blocks = Vec::new();
        self.max_payload = Vec::new();
        self.allocated = 0;
    }

    fn into_parts(self) -> Option<Part> {
        if self.overflow {
            return None;
        }
        Some(Part {
            signals: self.signals,
            offsets: self.offsets,
            ticks: self.ticks,
            blocks: self.blocks,
            max_payload: self.max_payload,
            max_identifier: self.max_identifier,
        })
    }
}

struct Part {
    signals: Vec<u32>,
    offsets: Vec<u64>,
    ticks: Vec<(u32, u64)>,
    blocks: Vec<(u64, u64)>,
    max_payload: Vec<u64>,
    max_identifier: u64,
}

impl Part {
    fn held(&self) -> usize {
        self.signals.capacity() * size_of::<u32>()
            + self.offsets.capacity() * size_of::<u64>()
            + self.ticks.capacity() * TICK_BYTES
            + self.blocks.capacity() * BLOCK_BYTES
            + self.max_payload.capacity() * CAPABILITY_BYTES
    }
}

/// A complete, immutable position index for one opened source.
pub(super) struct Index {
    signals: Vec<Vec<Entry>>,
    blocks: Vec<(u64, u64)>,
    max_payload: Vec<u64>,
    max_identifier: u64,
}

impl Index {
    /// Merges per-chunk records in source order.
    ///
    /// The representation is dropped when any chunk overflowed its share, when
    /// a count or the retained size does not fit, when the peak of the merge
    /// would exceed the combined budget, or when an allocation fails. Every
    /// retaining allocation is fallible.
    pub(super) fn build(
        parts: impl IntoIterator<Item = Builder>,
        signals: usize,
        limit: usize,
        peak: usize,
    ) -> Option<Self> {
        let parts = parts
            .into_iter()
            .map(Builder::into_parts)
            .collect::<Option<Vec<_>>>()?;
        // Check the bounds that do not need records before touching one counter
        // per declaration. An impossible list table or peak refuses here.
        let list_floor = signals.checked_mul(LIST_BYTES)?;
        let counts_bytes = signals.checked_mul(size_of::<u32>())?;
        let capability_bytes = signals.checked_mul(CAPABILITY_BYTES)?;
        let mut held = 0_usize;
        for part in &parts {
            held = held.checked_add(part.held())?;
        }
        if list_floor.checked_add(capability_bytes)? > limit
            || held
                .checked_add(counts_bytes)?
                .checked_add(capability_bytes)?
                > peak
        {
            return None;
        }
        let mut counts = Vec::new();
        counts.try_reserve_exact(signals).ok()?;
        counts.resize(signals, 0_u32);
        let mut max_payload = Vec::new();
        max_payload.try_reserve_exact(signals).ok()?;
        max_payload.resize(signals, 0_u64);
        let mut max_identifier = 0_u64;
        let mut blocks_len = 0_usize;
        for part in &parts {
            blocks_len = blocks_len.checked_add(part.blocks.len())?;
            max_identifier = max_identifier.max(part.max_identifier);
            for (signal, &payload) in part.max_payload.iter().enumerate() {
                let slot = max_payload.get_mut(signal)?;
                *slot = (*slot).max(payload);
            }
            for &signal in &part.signals {
                let count = counts.get_mut(signal as usize)?;
                *count = count.checked_add(1)?;
            }
        }
        let entries = counts
            .iter()
            .try_fold(0_usize, |total, &count| total.checked_add(count as usize))?;
        let retained = entries
            .checked_mul(ENTRY_BYTES)?
            .checked_add(list_floor)?
            .checked_add(blocks_len.checked_mul(BLOCK_BYTES)?)?
            .checked_add(capability_bytes)?;
        if retained > limit || held.checked_add(retained)?.checked_add(counts_bytes)? > peak {
            return None;
        }
        let mut lists = Vec::new();
        lists.try_reserve_exact(signals).ok()?;
        for &count in &counts {
            let mut list = Vec::new();
            list.try_reserve_exact(count as usize).ok()?;
            lists.push(list);
        }
        let mut blocks = Vec::new();
        blocks.try_reserve_exact(blocks_len).ok()?;
        for part in parts {
            let mut tick = 0_usize;
            for (record, (&signal, &offset)) in part.signals.iter().zip(&part.offsets).enumerate() {
                while tick + 1 < part.ticks.len() && part.ticks[tick + 1].0 as usize <= record {
                    tick += 1;
                }
                lists[signal as usize].push(Entry {
                    time: part.ticks[tick].1,
                    offset,
                });
            }
            blocks.extend(part.blocks);
        }
        Some(Self {
            signals: lists,
            blocks,
            max_payload,
            max_identifier,
        })
    }

    pub(super) fn entries(&self, signal: usize) -> &[Entry] {
        &self.signals[signal]
    }

    /// Largest raw record token seen for this signal during opening validation.
    ///
    /// This is the payload token plus the identifier and a separator; leading
    /// whitespace is skipped without retention and is not part of the bound.
    pub(super) fn max_token(&self, signal: usize) -> u64 {
        self.max_payload[signal]
            .saturating_add(self.max_identifier)
            .saturating_add(2)
    }

    /// Returns the index of the last recorded entry at or before `time`.
    pub(super) fn last_at_or_before(&self, signal: usize, time: u64) -> Option<usize> {
        self.entries(signal)
            .partition_point(|entry| entry.time <= time)
            .checked_sub(1)
    }

    /// Returns the index of the first recorded entry after `time`.
    pub(super) fn first_after(&self, signal: usize, time: u64) -> usize {
        self.entries(signal)
            .partition_point(|entry| entry.time <= time)
    }

    /// Returns whether a source offset lies inside a dump block.
    pub(super) fn in_block(&self, offset: u64) -> bool {
        let after = self.blocks.partition_point(|&(start, _)| start <= offset);
        after
            .checked_sub(1)
            .is_some_and(|index| offset < self.blocks[index].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(signals: usize, records: &[(u32, u64, u64)], blocks: &[(u64, u64)]) -> Builder {
        let mut builder = Builder::new(signals, usize::MAX / 2);
        for &(signal, time, offset) in records {
            builder.record(signal as usize, time, offset, (8, 2));
        }
        for &(start, end) in blocks {
            builder.block(start, end);
        }
        builder
    }

    #[test]
    fn merge_keeps_source_order_per_signal_and_tick_context() {
        let first = part(2, &[(1, 2, 10), (0, 2, 11), (1, 5, 30)], &[(4, 8)]);
        let second = part(2, &[(1, 5, 31), (0, 9, 50)], &[]);
        let index = Index::build([first, second], 2, usize::MAX, usize::MAX).unwrap();
        let one = index.entries(1);
        assert_eq!(
            one.iter()
                .map(|entry| (entry.time, entry.offset))
                .collect::<Vec<_>>(),
            [(2, 10), (5, 30), (5, 31)]
        );
        assert_eq!(index.last_at_or_before(1, 4), Some(0));
        assert_eq!(index.last_at_or_before(1, 5), Some(2));
        assert_eq!(index.first_after(1, 5), 3);
        assert!(index.in_block(5));
        assert!(!index.in_block(8));
        assert_eq!(index.max_token(1), 8 + 2 + 2);
    }

    #[test]
    fn record_budget_counts_actual_capacities() {
        let mut builder = Builder::new(1, 4 * RECORD_BYTES);
        for offset in 0..1024 {
            builder.record(0, 0, offset, (8, 2));
        }
        assert!(Index::build([builder], 1, usize::MAX, usize::MAX).is_none());
        let exact = part(1, &[(0, 1, 4)], &[]);
        assert!(Index::build([exact], 1, 0, usize::MAX).is_none());
        // Retained entries fit, while the transient minimum capacities only
        // count against the peak.
        assert!(Index::build([part(1, &[(0, 1, 4)], &[])], 1, 1024, usize::MAX).is_some());
    }

    #[test]
    fn tick_and_block_budgets_are_bounded() {
        // The same 1024 records in one tick fit the records and one tick entry.
        let mut same_tick = Builder::new(1, 32 * 1024);
        for offset in 0..1024 {
            same_tick.record(0, 7, offset, (8, 2));
        }
        assert!(!same_tick.overflow);
        assert_eq!(same_tick.signals.len(), 1024);
        assert_eq!(same_tick.ticks.len(), 1);
        assert!(Index::build([same_tick], 1, usize::MAX, usize::MAX).is_some());
        // One tick per record keeps the same record capacities but must exhaust
        // the tick budget, which an implementation that ignored ticks would not.
        let mut ticks = Builder::new(1, 32 * 1024);
        for tick in 0..1024 {
            ticks.record(0, tick, tick, (8, 2));
        }
        assert!(ticks.overflow);
        assert!(Index::build([ticks], 1, usize::MAX, usize::MAX).is_none());
        // An empty block on every tick is also retained state.
        let mut blocks = Builder::new(1, 8 * BLOCK_BYTES);
        for tick in 0..1024 {
            blocks.block(tick, tick + 1);
        }
        assert!(Index::build([blocks], 1, usize::MAX, usize::MAX).is_none());
        // Retained state stays available inside a budget that covers it.
        let mut small = Builder::new(1, 64 * 1024);
        for tick in 0..4 {
            small.record(0, tick, tick, (8, 2));
            small.block(tick, tick + 1);
        }
        assert!(Index::build([small], 1, 4096, usize::MAX).is_some());
    }

    #[test]
    fn impossible_lower_bounds_refuse_before_counting() {
        // The per-signal list table alone exceeds the retained limit.
        let builder = part(1024, &[], &[]);
        assert!(
            Index::build(
                [builder],
                1024,
                1024 * (LIST_BYTES + CAPABILITY_BYTES) - 1,
                usize::MAX
            )
            .is_none()
        );
        // The transient parts plus the counters exceed the merge peak.
        let builder = part(1024, &[(0, 1, 4)], &[]);
        let held = builder.into_parts().unwrap().held();
        assert!(
            Index::build(
                [part(1024, &[(0, 1, 4)], &[])],
                1024,
                usize::MAX,
                held + 1024 * (4 + CAPABILITY_BYTES) - 1
            )
            .is_none()
        );
    }

    #[test]
    fn build_peak_covers_parts_counts_and_retained_lists() {
        let builder = part(3, &[(0, 1, 4), (1, 1, 5), (0, 2, 6)], &[]);
        let index = Index::build([builder], 3, usize::MAX, usize::MAX).unwrap();
        assert_eq!(index.entries(0).len(), 2);
        assert_eq!(index.entries(1).len(), 1);
        assert!(index.entries(2).is_empty());
        let builder = part(3, &[(0, 1, 4), (1, 1, 5), (0, 2, 6)], &[]);
        assert!(Index::build([builder], 3, usize::MAX, 1).is_none());
    }
}
