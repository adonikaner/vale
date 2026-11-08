//! New numbers: guids, `uniqueId`s, template and pool entries.
//!
//! Alone, an editor numbers a new spawn one above the largest it can see. Two
//! editors placing at the same moment see the same largest and choose the
//! same number. In a session each editor is handed blocks of numbers by the
//! host instead and numbers from its own block, so placing needs no round
//! trip.
//!
//! A kind is named by a string (`"creature guid"`). The host does not know
//! what the database holds, so a request carries `floor`, the number the
//! editor would have used alone, and a block starts at the larger of that and
//! the end of the last block of that kind.

use std::collections::HashMap;

/// The host's side: where the next block of each kind starts.
#[derive(Debug, Default)]
pub struct Allocator {
    next: HashMap<String, u64>,
}

/// The largest block one request is given.
pub const MAX_BLOCK: u64 = 1 << 16;

impl Allocator {
    /// The block `start..end` for a request, which moves the kind's next start
    /// past it. A size over [`MAX_BLOCK`] is cut to it.
    pub fn grant(&mut self, kind: &str, floor: u64, size: u64) -> (u64, u64) {
        let next = self.next.entry(kind.to_string()).or_insert(0);
        let start = (*next).max(floor);
        let end = start.saturating_add(size.clamp(1, MAX_BLOCK));
        *next = end;
        (start, end)
    }
}

/// An editor's side: the blocks it holds, by kind.
#[derive(Debug, Default)]
pub struct Blocks {
    held: HashMap<String, (u64, u64)>,
}

impl Blocks {
    /// Keep a granted block. A block replaces what is left of the last one,
    /// since the host grants a kind's blocks in increasing order.
    pub fn hold(&mut self, kind: &str, start: u64, end: u64) {
        self.held.insert(kind.to_string(), (start, end));
    }

    /// The next number of `kind` not below `floor`, taken from the block, or
    /// `None` when the block is empty or ends below `floor`.
    pub fn take(&mut self, kind: &str, floor: u64) -> Option<u64> {
        let (start, end) = self.held.get_mut(kind)?;
        let number = (*start).max(floor);
        if number >= *end {
            return None;
        }
        *start = number + 1;
        Some(number)
    }

    /// How many numbers of `kind` are left.
    pub fn left(&self, kind: &str) -> u64 {
        self.held.get(kind).map_or(0, |(start, end)| end.saturating_sub(*start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two editors asking from the same floor get blocks that do not
    /// overlap, and an editor numbers from its own block.
    #[test]
    fn two_requests_from_one_floor_get_separate_blocks() {
        let mut host = Allocator::default();
        let first = host.grant("creature guid", 10_000_000, 100);
        let second = host.grant("creature guid", 10_000_000, 100);
        assert_eq!(first, (10_000_000, 10_000_100));
        assert_eq!(second, (10_000_100, 10_000_200));
        assert_eq!(host.grant("pool entry", 5, 10), (5, 15));

        let mut blocks = Blocks::default();
        blocks.hold("creature guid", first.0, first.1);
        assert_eq!(blocks.take("creature guid", 0), Some(10_000_000));
        assert_eq!(blocks.take("creature guid", 10_000_050), Some(10_000_050));
        assert_eq!(blocks.left("creature guid"), 49);
        assert_eq!(blocks.take("creature guid", 10_000_100), None);
        assert_eq!(blocks.take("pool entry", 0), None);
    }
}
