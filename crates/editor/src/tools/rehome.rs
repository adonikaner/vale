//! One row per placement, in the tile its origin is in.
//!
//! ## The claim rule in `render::terrain`
//!
//! `render::terrain`'s claim rule is one line, and it is a rule for reading:
//!
//! ```text
//! let claimed = |p| tile_for_position(p.position[0], p.position[1]) == coord;
//! ```
//!
//! A model touching two tiles is listed in both tiles' `MDDF`/`MODF` with one
//! `unique_id`, so exactly one of them has to claim it or every tree along every
//! seam is drawn twice. The tile containing the origin is the one that does.
//!
//! The rule works for reading. Writing can break it in two ways.
//!
//! * Dragging a placement past a tile border can leave it undrawn. The tile
//!   whose file still holds the record no longer claims it, and the tile that
//!   would claim it has no record to claim. It does not vanish as it crosses
//!   the border — the entity is still there with its transform written — it
//!   vanishes when something next reads the tile. An undo does not bring it
//!   back either: undo of a plain move marks nothing stale, because the
//!   entity is already on screen and gets its transform written directly.
//!   Only a playtest, which reads everything, puts it back on screen.
//! * A large placement is listed in many tiles at once. Stormwind's `MODF`
//!   row is in every tile its box touches. Moving it leaves a stale copy in
//!   each of those tiles, waiting to be found — by the next run of this
//!   tool, by the reference client, by anything else that reads the tile.
//!   Written naively, one drag produced fifteen entries on the undo stack and
//!   as many copies of the city.
//!
//! ## Settling enforces the invariant instead of moving one record
//!
//! [`settle_doodad`] and [`settle_building`] do not move a single record.
//! They make the files agree with one statement: every row with a
//! `unique_id` states the same position, and one of them is in the tile that
//! position is in.
//!
//! When that does not hold, every row is taken out of every open tile and one
//! is put back into the tile the origin is now over. The operation is
//! idempotent — running it twice changes nothing the second time — so it
//! cannot accumulate stale rows, and it repairs a file that has already
//! accumulated them rather than only declining to add to it.
//!
//! ## Why the invariant is agreement, not "one row"
//!
//! Blizzard's data is not one row per placement, and an editor must not
//! quietly rewrite what it was given. A placement that straddles a seam is
//! listed in every tile its box touches, all stating the same position, and
//! that is correct data which this client reads correctly. Collapsing it
//! would be an edit nobody asked for, on a file nobody touched, the moment a
//! tree on a border is selected.
//!
//! What is not correct is two rows that disagree about where the thing is.
//! That cannot come out of the game's own files and can only come out of a
//! half-finished move, so it is the condition worth acting on — and it is
//! the discriminator, because a stale copy is stale precisely in its
//! position.
//!
//! It is also what the output needs. A stale row left behind is a row the
//! reference client draws the building from, at its old position.
//!
//! ## Cost when nothing needs to move, which is nearly always the case
//!
//! A scan of four bytes per placement per open tile — `doodads_with_id` reads
//! the id and nothing else — and then a comparison. Nine tiles of a thousand
//! placements is nine thousand `u32` reads, and no allocation and no history
//! entry unless something actually has to move.
//!
//! ## What settling does not do
//!
//! It does not write a tile this session has not parsed. The 3x3 area around
//! the camera is what is open, so this only comes up for a drag that ends
//! more than a tile away from where the camera is looking; the operation
//! reports that rather than writing a file that has not been read. A row in
//! a tile that is not open is also a row this cannot clean up, which is the
//! one way a duplicate can survive.

use crate::session::EditSession;
use vale_edit::ops::{Edit, Placements};

/// Which tile a placement's origin is over.
fn home_of(position: [f32; 3]) -> (u32, u32) {
    let world = vale_assets::world::adt::placement_to_world(position);
    vale_assets::tile_for_position(world[0], world[1])
}

/// One row carrying an id: where it is, and what it says.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Row {
    coord: (u32, u32),
    index: usize,
    position: [f32; 3],
}

/// Every open tile's rows for one id.
fn rows(session: &EditSession, unique_id: u32, buildings: bool) -> Vec<Row> {
    let mut found = Vec::new();
    for (coord, tile) in &session.tiles {
        let at = match buildings {
            true => tile.buildings_with_id(unique_id),
            false => tile.doodads_with_id(unique_id),
        };
        for index in at {
            let position = match buildings {
                true => tile.building_at(index).map(|row| row.position),
                false => tile.doodad_at(index).map(|row| row.position),
            };
            if let Some(position) = position {
                found.push(Row {
                    coord: *coord,
                    index,
                    position,
                });
            }
        }
    }
    // Sorted so the work is the same whatever order the map iterates in, which is
    // what makes a history entry reproducible.
    found.sort_unstable_by_key(|row| (row.coord, row.index));
    found
}

/// Whether the files already say what they should, so nothing has to be done.
///
/// This is asked every frame, and answering "no" writes to files, so it must
/// stay cheap. See the module comment for why the test is agreement rather
/// than uniqueness: a placement legitimately listed in six tiles that all
/// state the same position is correct data, and rewriting it because a
/// person clicked on it would be an edit nobody asked for.
fn settled(rows: &[Row], home: (u32, u32)) -> bool {
    !rows.is_empty()
        && rows.iter().all(|row| row.position == rows[0].position)
        && rows.iter().any(|row| row.coord == home)
}

/// Make the open tiles agree that this `MDDF` placement is in one place.
///
/// Answers the `(tile, index)` it ended up at, or `None` when nothing needed
/// doing or the tile it belongs in is not open.
pub fn settle_doodad(
    session: &mut EditSession,
    unique_id: u32,
    radius: f32,
) -> Option<((u32, u32), usize)> {
    settle(session, unique_id, false, radius)
}

/// …and the same for a `MODF` one. It needs no radius: a `MODF` row carries its
/// own box, so `add_building` works out which chunks to reference from the
/// record itself.
pub fn settle_building(session: &mut EditSession, unique_id: u32) -> Option<((u32, u32), usize)> {
    settle(session, unique_id, true, 0.0)
}

fn settle(
    session: &mut EditSession,
    unique_id: u32,
    buildings: bool,
    radius: f32,
) -> Option<((u32, u32), usize)> {
    let rows = rows(session, unique_id, buildings);
    // The row the claim rule would draw, or the newest-looking one when none
    // of them is claimed, which is the state a half-finished move leaves. A
    // stale copy is stale in its position, so the row whose own position puts
    // it in its own tile is the one that was moved.
    let live = *rows
        .iter()
        .find(|row| home_of(row.position) == row.coord)
        .or_else(|| rows.first())?;
    let home = home_of(live.position);
    if settled(&rows, home) {
        return None;
    }

    let (coord, index) = (live.coord, live.index);
    let tile = session.tiles.get(&coord)?;
    let path = match buildings {
        true => {
            let row = tile.building_at(index)?;
            let names = tile.building_names();
            names.get(row.name_id as usize).cloned().unwrap_or_default()
        }
        false => {
            let row = tile.doodad_at(index)?;
            let names = tile.model_names();
            names.get(row.name_id as usize).cloned().unwrap_or_default()
        }
    };
    if !session.tiles.contains_key(&home) {
        session.status = format!(
            "tile {},{} is not open; the placement was not moved into it",
            home.0, home.1
        );
        return None;
    }

    // The record is taken before anything is removed, and the history entry
    // is not opened until every step is known to be possible. An earlier
    // version did the work as it went and could return partway through with
    // a change left open, which folded the next edit into it.
    let moving = match buildings {
        true => Moving::Building(session.tiles.get(&coord)?.building_at(index)?),
        false => Moving::Doodad(session.tiles.get(&coord)?.doodad_at(index)?),
    };

    // Into the entry of the move that took the origin across the border, and
    // not an entry of its own. A group of fifty moved over a border is fifty
    // settles, and as entries of their own they were fifty presses of undo
    // before the move itself was reached. `amend` declines on an empty stack
    // and on a redo branch; only then is an entry opened here.
    let label = match buildings {
        true => format!("Move WMO to tile {},{}", home.0, home.1),
        false => format!("Move doodad to tile {},{}", home.0, home.1),
    };
    if !session.history.amend() {
        session.history.begin(label);
    }

    // Out of every open tile, highest index first so the ones below stay valid.
    let mut touched: Vec<(u32, u32)> = Vec::new();
    for coord in rows.iter().map(|row| row.coord).rev() {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let at = match buildings {
            true => tile.buildings_with_id(unique_id),
            false => tile.doodads_with_id(unique_id),
        };
        if at.is_empty() {
            continue;
        }
        let before = Placements::capture(tile);
        for index in at.into_iter().rev() {
            match buildings {
                true => {
                    tile.remove_building(index);
                }
                false => {
                    tile.remove_doodad(index);
                }
            }
        }
        let edit = Edit::Placements {
            before: Box::new(before),
            after: Box::new(Placements::capture(tile)),
        };
        session.history.record(&key, [edit]);
        if !touched.contains(&coord) {
            touched.push(coord);
        }
    }

    // …and one back into the tile it belongs in, under that tile's own
    // numbering: a `name_id` is an index into the file it is in and means
    // nothing outside it.
    let key = session.key(home);
    let tile = session.tiles.get_mut(&home)?;
    let before = Placements::capture(tile);
    let landed = match moving {
        Moving::Building(mut row) => {
            row.name_id = tile.name_building(&path);
            tile.add_building(row)
        }
        Moving::Doodad(mut row) => {
            row.name_id = tile.name_model(&path);
            tile.add_doodad(row, radius)
        }
    };
    let edit = Edit::Placements {
        before: Box::new(before),
        after: Box::new(Placements::capture(tile)),
    };
    session.history.record(&key, [edit]);
    session.history.end();
    if !touched.contains(&home) {
        touched.push(home);
    }

    for coord in &touched {
        session.publish(*coord);
        session.stale.insert(*coord);
    }
    session.status = match rows.len() {
        1 => format!("moved to tile {},{}", home.0, home.1),
        n => format!("{n} rows collapsed into one, in tile {},{}", home.0, home.1),
    };
    Some((home, landed))
}

/// The row being moved, read out before anything is removed.
enum Moving {
    Doodad(vale_edit::adt::place::Doodad),
    Building(vale_edit::adt::place::Building),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A placement is in the wrong tile the moment its origin crosses. This is
    /// the rule `render::terrain` claims by, asked of a position rather than
    /// of a file.
    #[test]
    fn a_placement_belongs_to_the_tile_its_origin_is_in() {
        let origin = vale_assets::world::adt::MAP_ORIGIN;
        let side = vale_assets::world::adt::TILE_SIZE;
        let middle = [
            origin - 32.0 * side - side * 0.5,
            origin - 32.0 * side - side * 0.5,
            0.0,
        ];
        let here = vale_assets::tile_for_position(middle[0], middle[1]);
        let record = vale_assets::world::adt::placement_from_world(middle);
        assert_eq!(home_of(record), here, "still in its own tile");

        // A nudge inside the tile does not move it…
        let near = vale_assets::world::adt::placement_from_world([
            middle[0] + side * 0.4,
            middle[1],
            0.0,
        ]);
        assert_eq!(home_of(near), here);

        // …and a drag past the border does.
        let over = [middle[0] + side, middle[1], 0.0];
        let across = vale_assets::world::adt::placement_from_world(over);
        assert_ne!(home_of(across), here);
        assert_eq!(
            home_of(across),
            vale_assets::tile_for_position(over[0], over[1])
        );
    }

    /// A group moved over a tile border is one entry on the history: the drag
    /// and every member's move into the tile its origin is now in. One undo
    /// puts every row back where it was, and a redo puts them all across.
    ///
    /// Each member's settle opened an entry of its own before, so a group of
    /// twenty dragged over a border took twenty-one presses of undo.
    #[test]
    fn a_group_moved_over_a_border_is_one_entry() {
        use crate::tools::doodads::{write_record, Selected};
        use vale_edit::adt::place::Doodad;

        let install = std::env::temp_dir().join(format!("vale-rehome-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);

        let origin = vale_assets::world::adt::MAP_ORIGIN;
        let side = vale_assets::world::adt::TILE_SIZE;
        let middle = [
            origin - 32.0 * side - side * 0.5,
            origin - 32.0 * side - side * 0.5,
            0.0,
        ];
        let here = vale_assets::tile_for_position(middle[0], middle[1]);
        let there = vale_assets::tile_for_position(middle[0] + side, middle[1]);
        for coord in [here, there] {
            session.tiles.insert(
                coord,
                vale_edit::adt::blank::blank_tile(coord.0, coord.1, "a.blp", 0.0, 0),
            );
        }
        let tile = session.tiles.get_mut(&here).unwrap();
        let name_id = tile.name_model("tree.m2");
        let mut group = Vec::new();
        for n in 0..20u32 {
            let record = Doodad {
                name_id,
                unique_id: 1000 + n,
                position: vale_assets::world::adt::placement_from_world([
                    middle[0] + n as f32,
                    middle[1],
                    0.0,
                ]),
                rotation: [0.0; 3],
                scale: 1024,
                flags: 0,
            };
            let index = tile.add_doodad(record, 5.0);
            group.push(Selected {
                tile: here,
                index,
                unique_id: record.unique_id,
                path: "tree.m2".to_string(),
                record,
            });
        }
        let start = |session: &EditSession| {
            [here, there].map(|coord| vale_edit::ops::Placements::capture(&session.tiles[&coord]))
        };
        let before = start(&session);

        // The drag: one entry around every member's write.
        session.history.begin("Move 20 doodads");
        for member in &mut group {
            let world = vale_assets::world::adt::placement_to_world(member.record.position);
            member.record.position = vale_assets::world::adt::placement_from_world([
                world[0] + side,
                world[1],
                world[2],
            ]);
            write_record(&mut session, member);
        }
        session.history.end();
        // …and the settle that follows the release, member by member.
        for member in &group {
            assert!(settle_doodad(&mut session, member.unique_id, 5.0).is_some());
        }
        let after = start(&session);

        assert_eq!(session.history.depth_done(), 1, "the drag and every settle are one entry");
        assert_eq!(session.tiles[&here].doodad_list().len(), 0);
        assert_eq!(session.tiles[&there].doodad_list().len(), 20);

        let change = session.history.undo().unwrap();
        for coord in [there, here] {
            let key = session.key(coord);
            change.revert(&key, session.tiles.get_mut(&coord).unwrap());
        }
        assert!(start(&session) == before, "one undo puts every row back");
        for coord in [here, there] {
            let key = session.key(coord);
            change.apply(&key, session.tiles.get_mut(&coord).unwrap());
        }
        assert!(start(&session) == after, "one redo moves them all again");
        let _ = std::fs::remove_dir_all(&install);
    }

    fn row(coord: (u32, u32), index: usize, position: [f32; 3]) -> Row {
        Row {
            coord,
            index,
            position,
        }
    }

    /// Rows that agree are settled however many there are; rows that disagree
    /// are not.
    ///
    /// The first half is what keeps this from rewriting the game's own files: a
    /// placement straddling a seam is listed in every tile its box touches, all
    /// stating one position, and that is correct data. Collapsing it because
    /// somebody clicked on a tree would be an edit nobody asked for.
    ///
    /// The second half is the fault this exists for. Two rows that disagree
    /// about where a thing is cannot come out of the game's own files, only
    /// out of a half-finished move, and left alone they accumulate into
    /// duplicate copies of a placement, such as the fifteen copies of
    /// Stormwind described in the module comment.
    #[test]
    fn rows_that_agree_are_settled_and_rows_that_disagree_are_not() {
        let home = (31, 50);
        let here = [1.0, 2.0, 3.0];
        let elsewhere = [900.0, 2.0, 3.0];

        assert!(settled(&[row(home, 0, here)], home));
        assert!(
            settled(
                &[
                    row(home, 0, here),
                    row((30, 50), 4, here),
                    row((31, 49), 2, here)
                ],
                home
            ),
            "a straddling placement listed in three tiles is correct data"
        );

        assert!(!settled(&[], home), "no row at all is not settled");
        assert!(
            !settled(&[row((30, 50), 4, here)], home),
            "one row, and not in the tile its own position puts it in"
        );
        assert!(
            !settled(&[row(home, 0, here), row((30, 50), 4, elsewhere)], home),
            "two rows that disagree about where it is"
        );
    }
}
