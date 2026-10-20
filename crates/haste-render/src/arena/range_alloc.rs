//! The arenas' first-fit range allocator (vertices, indices and quad records).

/// First-fit range allocator with coalescing frees.
#[derive(Debug, Clone)]
pub(crate) struct RangeAlloc {
    /// Free ranges (start, len), sorted by start, non-adjacent.
    pub(super) free: Vec<(u32, u32)>,
    /// Total capacity in entries.
    capacity: u32,
}

impl RangeAlloc {
    pub(crate) fn new(len: u32) -> RangeAlloc {
        RangeAlloc {
            free: vec![(0, len)],
            capacity: len,
        }
    }

    pub(crate) fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let i = self.free.iter().position(|&(_, l)| l >= len)?;
        let (start, l) = self.free[i];
        if l == len {
            self.free.remove(i);
        } else {
            self.free[i] = (start + len, l - len);
        }
        Some(start)
    }

    pub(crate) fn free(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        let i = self.free.partition_point(|&(s, _)| s < start);
        self.free.insert(i, (start, len));
        // Merge with the next, then with the previous.
        if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }

    /// Entries currently allocated (capacity minus the free list).
    pub(crate) fn used(&self) -> u32 {
        let free: u32 = self.free.iter().map(|&(_, l)| l).sum();
        self.capacity - free
    }

    /// Total capacity in entries.
    pub(crate) fn total(&self) -> u32 {
        self.capacity
    }
}
