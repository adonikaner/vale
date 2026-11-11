//! New numbers: guids, `uniqueId`s, template and pool entries, client table
//! ids.
//!
//! Alone, an editor numbers a new record one above the largest it can see.
//! Two editors placing at the same moment see the same largest and choose the
//! same number. In a session each joined editor is handed blocks of numbers
//! by the host instead and numbers from its own blocks, so placing needs no
//! round trip, and an offline copy keeps numbering from the blocks it holds.
//!
//! A type of number is named by a string (`"creature.guid"`). The host does
//! not know what the database holds, so a request carries `floor`, the number
//! the editor would have used alone, and a block starts at the larger of that
//! and the end of the last block of that type.
//!
//! The host's editor numbers through the same [`Allocator`] it grants from
//! ([`Allocator::take`]), and keeps the allocator in its log folder, so a
//! number it gives alone between two sessions is never inside a block an
//! offline copy still holds. A type no block was ever granted of is numbered
//! from its floor, as an editor alone numbers.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

/// The host's side: where the next block of each type starts.
#[derive(Debug, Default)]
pub struct Allocator {
    next: BTreeMap<String, u64>,
    /// A grant was made since [`Self::take_changed`] was last called.
    changed: bool,
}

/// An allocator shared by the host and the host's editor, which numbers
/// from it directly.
pub type Shared = Arc<Mutex<Allocator>>;

/// The largest block one request is given.
pub const MAX_BLOCK: u64 = 1 << 16;

impl Allocator {
    /// The block `start..end` for a request, which moves the type's next
    /// start past it. A size over [`MAX_BLOCK`] is cut to it.
    pub fn grant(&mut self, kind: &str, floor: u64, size: u64) -> (u64, u64) {
        let next = self.next.entry(kind.to_string()).or_insert(0);
        let start = (*next).max(floor);
        let end = start.saturating_add(size.clamp(1, MAX_BLOCK));
        *next = end;
        self.changed = true;
        (start, end)
    }

    /// A number of the host's editor's own: the floor while no block of
    /// `kind` was ever granted, as an editor alone numbers, and otherwise
    /// one at or above the floor and past every block granted.
    pub fn take(&mut self, kind: &str, floor: u64) -> u64 {
        match self.next.contains_key(kind) {
            true => self.grant(kind, floor, 1).0,
            false => floor,
        }
    }

    /// Whether a grant was made since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The allocator as its file holds it: one line per type, the type and
    /// the next start separated by a tab.
    pub fn text(&self) -> String {
        self.next.iter().map(|(kind, next)| format!("{}\t{next}\n", clean(kind))).collect()
    }

    /// An allocator read back from [`Self::text`]. A line that does not read
    /// is skipped.
    pub fn parse(text: &str) -> Allocator {
        let next = text
            .lines()
            .filter_map(|line| {
                let (kind, next) = line.rsplit_once('\t')?;
                Some((kind.to_string(), next.trim().parse().ok()?))
            })
            .collect();
        Allocator { next, changed: false }
    }
}

/// An editor's side: the blocks it holds, by type, oldest first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Blocks {
    held: BTreeMap<String, VecDeque<(u64, u64)>>,
}

impl Blocks {
    /// Keep a granted block, after the blocks of its type already held.
    pub fn hold(&mut self, kind: &str, start: u64, end: u64) {
        if start < end {
            self.held.entry(kind.to_string()).or_default().push_back((start, end));
        }
    }

    /// The next number of `kind`, taken from the oldest block, or `None`
    /// when none is held. A block is never below a number in use: the host
    /// granted it from above the floor of the request, and every number
    /// above that floor given since came from the same allocator.
    pub fn take(&mut self, kind: &str) -> Option<u64> {
        let blocks = self.held.get_mut(kind)?;
        let (start, end) = blocks.front_mut()?;
        let number = *start;
        *start += 1;
        if *start >= *end {
            blocks.pop_front();
        }
        Some(number)
    }

    /// How many numbers of `kind` are left.
    pub fn left(&self, kind: &str) -> u64 {
        self.held.get(kind).map_or(0, |blocks| blocks.iter().map(|(start, end)| end - start).sum())
    }

    /// The blocks as their file holds them: one line per block, the type,
    /// the start and the end separated by tabs.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (kind, blocks) in &self.held {
            for (start, end) in blocks {
                out.push_str(&format!("{}\t{start}\t{end}\n", clean(kind)));
            }
        }
        out
    }

    /// Blocks read back from [`Self::text`]. A line that does not read is
    /// skipped.
    pub fn parse(text: &str) -> Blocks {
        let mut blocks = Blocks::default();
        for line in text.lines() {
            let mut fields = line.rsplitn(3, '\t');
            let (Some(end), Some(start), Some(kind)) = (fields.next(), fields.next(), fields.next()) else { continue };
            if let (Ok(start), Ok(end)) = (start.trim().parse(), end.trim().parse()) {
                blocks.hold(kind, start, end);
            }
        }
        blocks
    }
}

/// A type's name with no tab or line break, which the files separate by.
fn clean(kind: &str) -> String {
    kind.replace(['\t', '\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two editors asking from the same floor get blocks that do not
    /// overlap, and an editor numbers from its own blocks in order.
    #[test]
    fn two_requests_from_one_floor_get_separate_blocks() {
        let mut host = Allocator::default();
        let first = host.grant("creature.guid", 10_000_000, 100);
        let second = host.grant("creature.guid", 10_000_000, 100);
        assert_eq!(first, (10_000_000, 10_000_100));
        assert_eq!(second, (10_000_100, 10_000_200));
        assert_eq!(host.grant("pool_template.entry", 5, 10), (5, 15));

        let mut blocks = Blocks::default();
        blocks.hold("creature.guid", first.0, first.0 + 2);
        blocks.hold("creature.guid", second.0, second.1);
        assert_eq!(blocks.take("creature.guid"), Some(10_000_000));
        assert_eq!(blocks.take("creature.guid"), Some(10_000_001));
        assert_eq!(blocks.take("creature.guid"), Some(10_000_100));
        assert_eq!(blocks.left("creature.guid"), 99);
        assert_eq!(blocks.take("pool_template.entry"), None);
    }

    /// The host's own numbers are their floor until a block of the type is
    /// granted, and come past every block after that.
    #[test]
    fn the_hosts_own_numbers_and_the_blocks_do_not_overlap() {
        let mut host = Allocator::default();
        assert_eq!((host.take("uniqueId", 500), host.take("uniqueId", 500)), (500, 500));
        assert_eq!(host.grant("uniqueId", 500, 64), (500, 564));
        assert_eq!(host.take("uniqueId", 500), 564);
        assert_eq!(host.take("uniqueId", 900), 900);
    }

    /// Both files read back as they were written.
    #[test]
    fn the_files_read_back() {
        let mut host = Allocator::default();
        host.grant("Spell.dbc", 30_000, 64);
        host.grant("creature.guid", 7, 1);
        assert!(host.take_changed());
        assert!(!host.take_changed());
        let read = Allocator::parse(&host.text());
        assert_eq!(read.next, host.next);

        let mut blocks = Blocks::default();
        blocks.hold("Spell.dbc", 30_000, 30_064);
        blocks.hold("Spell.dbc", 30_100, 30_110);
        blocks.hold("uniqueId", 1, 2);
        assert_eq!(Blocks::parse(&blocks.text()), blocks);
    }
}
