//! A hard-capped map from verified or network identity to limiter state.
//!
//! An entry whose cells are rested and whose concurrency slots nobody holds or
//! awaits carries no information, so forgetting it is lossless. When the map
//! is full of entries that do carry information, a new identity gets no entry
//! and the caller decides what overflow means at its level.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use tokio::sync::Semaphore;

use super::bucket::Cell;

/// Per-class limiter state for one identity.
#[derive(Debug)]
pub(crate) struct Entry<const N: usize> {
    pub(crate) cells: [Cell; N],
    slots: [Option<Arc<Semaphore>>; N],
}

impl<const N: usize> Default for Entry<N> {
    fn default() -> Self {
        Self {
            cells: [Cell::default(); N],
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl<const N: usize> Entry<N> {
    /// The class's concurrency slots, created with `capacity` on first use.
    pub(crate) fn slots(&mut self, class: usize, capacity: usize) -> Arc<Semaphore> {
        Arc::clone(self.slots[class].get_or_insert_with(|| Arc::new(Semaphore::new(capacity))))
    }

    /// Permits and waiters each hold a clone, so a lone reference means
    /// nothing is admitted or queued at this identity.
    fn is_idle(&self, now: u64) -> bool {
        self.cells.iter().all(|cell| cell.is_rested(now))
            && self
                .slots
                .iter()
                .flatten()
                .all(|slots| Arc::strong_count(slots) == 1)
    }
}

pub(crate) struct Table<K, const N: usize> {
    entries: HashMap<K, Entry<N>>,
    capacity: usize,
    sweep_interval: u64,
    next_sweep: u64,
}

impl<K: Copy + Eq + Hash, const N: usize> Table<K, N> {
    /// `sweep_interval` bounds how often a full table may be scanned, so a
    /// flood of new identities cannot turn every lookup into a full scan.
    pub(crate) fn new(capacity: usize, sweep_interval: u64) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity.min(1_024)),
            capacity,
            sweep_interval,
            next_sweep: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The identity's entry, created if there is room, or `None` when the
    /// table is full of entries that still carry state.
    pub(crate) fn entry(&mut self, key: K, now: u64) -> Option<&mut Entry<N>> {
        if !self.entries.contains_key(&key) && !self.make_room(now) {
            return None;
        }
        Some(self.entries.entry(key).or_default())
    }

    fn make_room(&mut self, now: u64) -> bool {
        if self.entries.len() < self.capacity {
            return true;
        }
        if now < self.next_sweep {
            return false;
        }
        self.next_sweep = now.saturating_add(self.sweep_interval);
        self.entries.retain(|_, entry| !entry.is_idle(now));
        self.entries.len() < self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::super::bucket::Rate;
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    fn busy(table: &mut Table<u32, 1>, key: u32, now: u64) -> bool {
        let rate = Rate::new(60, 1).expect("rate");
        match table.entry(key, now) {
            Some(entry) => {
                entry.cells[0].charge(rate, now);
                true
            }
            None => false,
        }
    }

    #[test]
    fn a_full_table_of_live_entries_refuses_new_identities_without_growing() {
        let mut table: Table<u32, 1> = Table::new(4, SECOND);
        for key in 0..4 {
            assert!(busy(&mut table, key, 0));
        }
        for key in 4..10_000 {
            assert!(!busy(&mut table, key, 0), "identity {key} must overflow");
        }
        assert_eq!(table.len(), 4);
        assert!(busy(&mut table, 2, 0), "a known identity keeps its entry");
    }

    #[test]
    fn rested_entries_are_forgotten_to_make_room() {
        let mut table: Table<u32, 1> = Table::new(4, SECOND);
        for key in 0..4 {
            assert!(busy(&mut table, key, 0));
        }
        assert!(busy(&mut table, 99, 2 * SECOND), "rested entries are swept");
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_held_or_awaited_slot_pins_an_entry_even_when_its_cells_are_rested() {
        let mut table: Table<u32, 1> = Table::new(1, SECOND);
        let slots = table.entry(7, 0).expect("room").slots(0, 1);
        let permit = Arc::clone(&slots).try_acquire_owned().expect("permit");
        drop(slots);
        assert!(table.entry(8, 10 * SECOND).is_none(), "a permit pins");
        let waiter = table.entry(7, 10 * SECOND).expect("pinned").slots(0, 1);
        drop(permit);
        assert!(
            table.entry(8, 20 * SECOND).is_none(),
            "a queued waiter pins"
        );
        drop(waiter);
        assert!(table.entry(8, 30 * SECOND).is_some());
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_full_table_is_scanned_at_most_once_per_interval() {
        let mut table: Table<u32, 1> = Table::new(2, 10 * SECOND);
        assert!(busy(&mut table, 1, 0));
        assert!(busy(&mut table, 2, 0));
        assert!(
            !busy(&mut table, 3, 0),
            "the first full lookup scans and fails"
        );
        assert!(
            !busy(&mut table, 3, 5 * SECOND),
            "rested entries wait for the next permitted scan"
        );
        assert!(busy(&mut table, 3, 10 * SECOND));
    }
}
