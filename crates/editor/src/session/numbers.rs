//! The numbers this editor gives new records: client table ids, guids,
//! template entries, `uniqueId`s. The rule is `vale_edit::numbers`'s; the
//! blocks are `vale_collab::ids`'s.
//!
//! ```text
//! alone or hosting   the allocator kept in the project's log folder: the
//!                    floor, or past every block handed out of that type
//! joined or a copy   the blocks the host handed this editor, kept in the
//!                    copy's log folder; more are asked for when they run low
//! ```
//!
//! The allocator is used alone as well as hosting, so a number given alone
//! between two sessions is never inside a block an offline copy still
//! holds. A project that has never been hosted has an empty allocator, and
//! every number is its floor, as an editor without a session numbers.
//!
//! A joined editor that has no block of a type asks the host and refuses the
//! operation with a status line; the block arrives within a moment, and the
//! same operation then succeeds. The types used most are asked for as soon
//! as the copy is open (`crate::collab`). An operation that makes many
//! records at once, such as a paste, reserves its count first
//! ([`Numbers::reserve`]), so it is refused whole rather than cut short when
//! the blocks run out partway.

use std::collections::{BTreeMap, BTreeSet};
use vale_collab::ids::{Allocator, Blocks, Shared};
use vale_edit::numbers::UNIQUE_ID;

/// Where this editor's numbers come from. See the module comment.
#[derive(Debug)]
pub(crate) enum Source {
    Allocator(Shared),
    /// A joined editor's copy, connected to the host or not.
    Blocks { connected: bool },
}

/// This editor's numbers. See the module comment.
#[derive(Debug)]
pub(crate) struct Numbers {
    pub(crate) source: Source,
    pub(crate) blocks: Blocks,
    /// Types to ask the host for, with the floor to ask from and the size.
    wanted: BTreeMap<String, (u64, u64)>,
    /// Types asked for and not granted yet.
    asked: BTreeSet<String>,
    /// The blocks changed since they were last written.
    pub(crate) blocks_changed: bool,
    /// Why the last number asked for could not be given, for the status
    /// line, which `crate::collab` sets from it each frame.
    pub(crate) refused: Option<String>,
}

impl Default for Numbers {
    fn default() -> Self {
        Numbers {
            source: Source::Allocator(Shared::new(std::sync::Mutex::new(Allocator::default()))),
            blocks: Blocks::default(),
            wanted: BTreeMap::new(),
            asked: BTreeSet::new(),
            blocks_changed: false,
            refused: None,
        }
    }
}

/// How many numbers of a type one request asks for. A map id has three
/// digits in the server's tile files and a trigger id is a smallint, so
/// those are asked for a few at a time.
pub(crate) fn block_size(kind: &str) -> u64 {
    match kind {
        "Map.dbc" => 2,
        "AreaTrigger.dbc" | "TaxiNodes.dbc" => 4,
        UNIQUE_ID => 256,
        kind if kind.ends_with(".guid") => 256,
        _ => 32,
    }
}

impl Numbers {
    /// Number from an allocator: alone or hosting.
    pub(crate) fn use_allocator(&mut self, allocator: Allocator) {
        self.source = Source::Allocator(Shared::new(std::sync::Mutex::new(allocator)));
        self.blocks = Blocks::default();
        self.wanted.clear();
        self.asked.clear();
        self.blocks_changed = false;
    }

    /// Number from blocks: a joined editor's copy, with the blocks it kept.
    pub(crate) fn use_blocks(&mut self, blocks: Blocks, connected: bool) {
        self.source = Source::Blocks { connected };
        self.blocks = blocks;
        self.wanted.clear();
        self.asked.clear();
        self.blocks_changed = false;
    }

    /// Whether the copy is connected to its host, which asks for blocks. A
    /// connection made again forgets the requests the last one did not
    /// answer.
    pub(crate) fn set_connected(&mut self, now: bool) {
        if let Source::Blocks { connected } = &mut self.source {
            if *connected != now {
                *connected = now;
                self.asked.clear();
                self.wanted.clear();
            }
        }
    }

    /// The allocator, alone or hosting.
    pub(crate) fn allocator(&self) -> Option<&Shared> {
        match &self.source {
            Source::Allocator(shared) => Some(shared),
            Source::Blocks { .. } => None,
        }
    }

    /// Ask the host for a block of `kind` when none is held or asked for.
    pub(crate) fn want(&mut self, kind: &str, floor: u64) {
        if matches!(self.source, Source::Blocks { connected: true })
            && self.blocks.left(kind) == 0
            && !self.asked.contains(kind)
        {
            self.wanted.entry(kind.to_string()).or_insert((floor, block_size(kind)));
        }
    }

    /// Whether `count` numbers of `kind` can be taken now. When they cannot,
    /// a block large enough for them is asked for and the reason is kept for
    /// the status line, as a refused [`vale_edit::numbers::Numbers::take`]
    /// keeps it.
    pub(crate) fn reserve(&mut self, kind: &str, count: u64, floor: u64) -> bool {
        let connected = match self.source {
            Source::Allocator(_) => return true,
            Source::Blocks { connected } => connected,
        };
        let left = self.blocks.left(kind);
        if left >= count {
            return true;
        }
        if connected && !self.asked.contains(kind) {
            let size = (count - left).max(block_size(kind));
            let wanted = self.wanted.entry(kind.to_string()).or_insert((floor, size));
            wanted.1 = wanted.1.max(size);
        }
        self.refused = Some(match connected {
            true => format!("asked the host for {count} new {kind} numbers; try again in a moment"),
            false => format!("this copy has {left} {kind} numbers left offline and needs {count}; Reconnect to the host for more"),
        });
        false
    }

    /// Whether a block of `kind` is held or asked for, or not needed.
    pub(crate) fn has(&self, kind: &str) -> bool {
        match self.source {
            Source::Allocator(_) => true,
            Source::Blocks { .. } => self.blocks.left(kind) > 0 || self.asked.contains(kind) || self.wanted.contains_key(kind),
        }
    }

    /// The requests to send, each with its floor and size. Each is asked
    /// once until it is granted.
    pub(crate) fn take_wanted(&mut self) -> Vec<(String, u64, u64)> {
        let wanted = std::mem::take(&mut self.wanted);
        wanted
            .into_iter()
            .map(|(kind, (floor, size))| {
                self.asked.insert(kind.clone());
                (kind, floor, size)
            })
            .collect()
    }

    /// A number for a record whose key is its own conflict group, a server
    /// row: two editors giving one key is refused by the host rather than
    /// written twice. With no block held the number is the floor, and a block
    /// is asked for, so the next one comes from a block.
    pub(crate) fn soft(&mut self, kind: &str, floor: u64) -> u64 {
        match vale_edit::numbers::Numbers::take(self, kind, floor) {
            Ok(number) => number,
            Err(_) => {
                self.refused = None;
                floor
            }
        }
    }

    /// [`Self::soft`] for a 32-bit entry. A number past `u32::MAX` is the
    /// floor.
    pub(crate) fn soft_u32(&mut self, kind: &str, floor: u32) -> u32 {
        u32::try_from(self.soft(kind, u64::from(floor))).unwrap_or(floor)
    }

    /// Keep a block the host granted.
    pub(crate) fn hold(&mut self, kind: &str, start: u64, end: u64) {
        self.blocks.hold(kind, start, end);
        self.asked.remove(kind);
        self.blocks_changed = true;
    }
}

impl vale_edit::numbers::Numbers for Numbers {
    fn take(&mut self, kind: &str, floor: u64) -> Result<u64, String> {
        let connected = match &self.source {
            Source::Allocator(shared) => return Ok(shared.lock().expect("numbers").take(kind, floor)),
            Source::Blocks { connected } => *connected,
        };
        let number = self.blocks.take(kind);
        if number.is_some() {
            self.blocks_changed = true;
        }
        // Asked for before the last one runs out, so the next operation
        // does not wait.
        if connected && self.blocks.left(kind) < block_size(kind) / 2 && !self.asked.contains(kind) {
            self.wanted.entry(kind.to_string()).or_insert((floor, block_size(kind)));
        }
        let number = number.ok_or_else(|| match connected {
            true => format!("asked the host for new {kind} numbers; try again in a moment"),
            false => format!("this copy has no {kind} numbers left offline; Reconnect to the host for more"),
        });
        if let Err(why) = &number {
            self.refused = Some(why.clone());
        }
        number
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_edit::numbers::Numbers as _;

    /// Alone, a number is its floor until a block of its type is handed
    /// out.
    #[test]
    fn alone_a_number_is_its_floor() {
        let mut numbers = Numbers::default();
        assert_eq!(numbers.take("Spell.dbc", 100), Ok(100));
        assert_eq!(numbers.take("Spell.dbc", 100), Ok(100));
        numbers.allocator().unwrap().lock().unwrap().grant("Spell.dbc", 100, 32);
        assert_eq!(numbers.take("Spell.dbc", 100), Ok(132));
    }

    /// Joined, a type with no block is asked for once and refused until it
    /// arrives, and then numbered from the block.
    #[test]
    fn joined_a_number_comes_from_a_block() {
        let mut numbers = Numbers::default();
        numbers.use_blocks(Blocks::default(), true);
        assert!(numbers.take("creature.guid", 5).is_err());
        assert!(numbers.take("creature.guid", 5).is_err());
        let asked = numbers.take_wanted();
        assert_eq!(asked, vec![("creature.guid".to_string(), 5, 256)]);
        assert!(numbers.take_wanted().is_empty());
        numbers.hold("creature.guid", 1000, 1256);
        assert_eq!(numbers.take("creature.guid", 5), Ok(1000));
        assert!(numbers.blocks_changed);
    }

    /// Offline, the blocks kept are used and nothing is asked for.
    #[test]
    fn offline_the_kept_blocks_are_used() {
        let mut blocks = Blocks::default();
        blocks.hold("uniqueId", 7, 8);
        let mut numbers = Numbers::default();
        numbers.use_blocks(blocks, false);
        assert_eq!(numbers.take("uniqueId", 0), Ok(7));
        assert!(numbers.take("uniqueId", 0).is_err());
        assert!(numbers.take_wanted().is_empty());
    }

    /// A paste of more placements than the blocks hold is refused whole and
    /// asks for a block large enough for it, which then lets it through.
    #[test]
    fn a_reservation_larger_than_the_blocks_asks_for_enough() {
        let mut blocks = Blocks::default();
        blocks.hold("uniqueId", 100, 110);
        let mut numbers = Numbers::default();
        numbers.use_blocks(blocks, true);
        assert!(numbers.reserve("uniqueId", 10, 0));
        assert!(!numbers.reserve("uniqueId", 600, 0));
        assert!(numbers.refused.is_some());
        assert_eq!(numbers.take_wanted(), vec![("uniqueId".to_string(), 0, 590)]);
        numbers.hold("uniqueId", 5000, 5590);
        assert!(numbers.reserve("uniqueId", 600, 0));
        assert_eq!(numbers.blocks.left("uniqueId"), 600);
    }
}
