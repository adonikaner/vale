//! The explored areas' art: the `WorldMapOverlay` rows of a zone or a single
//! map. Each is a picture drawn over the map's picture at an offset once any
//! of its four areas is explored, cut into files as
//! `vale_edit::worldmap::overlay_pieces` says, with a label rectangle the
//! client names the area under the pointer by.
//!
//! The panel draws every overlay of the chosen map over its picture, as a
//! character who has explored everything sees it. One is selected at a time:
//! it is moved by dragging it or by its numbers, given its areas and its
//! label rectangle, given new art, or removed. New art, for a row or for a
//! new overlay, is [`Art`]: shown in place until Write art saves its files
//! (and, for a new overlay, adds its row). Its files are DXT3 with mipmaps,
//! as the shipped ones are, because the art has soft edges.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{rows, Entry, OVERLAYS};
use crate::session::EditSession;
use vale_assets::tables::worldmap::overlay_fields as of;
use vale_assets::world::blp::{self, Encoding};
use vale_client::assets::GameAssets;
use vale_edit::worldmap as pieces;

/// One `WorldMapOverlay` row, as the panel edits it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OverlayRow {
    pub(crate) record: usize,
    pub(crate) id: u32,
    /// The four `AreaTable` ids, any one of which explored shows it; 0 is none.
    pub(crate) areas: [u32; 4],
    /// The start of its files' names.
    pub(crate) texture: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) offset: (u32, u32),
    /// The label rectangle: top, left, bottom, right, in the picture's pixels.
    pub(crate) hit: [u32; 4],
}

/// What an overlay's joined art was read for: its texture name, its size,
/// and the [`EditSession::files_revision`] it was read at.
type Joined = (String, u32, u32, u64);

/// What new art is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// An overlay row, by id.
    Row(u32),
    /// A new overlay of the chosen map.
    New,
}

/// New art, shown in place until it is written.
pub(crate) struct Art {
    pub(crate) target: Target,
    pub(crate) path: PathBuf,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
    /// Where a new overlay's art is drawn; a row's art is drawn at the row's
    /// offset.
    pub(crate) offset: (u32, u32),
}

/// The explored areas' state.
#[derive(Default)]
pub(crate) struct Overlays {
    /// The selected overlay's row id.
    pub(crate) selected: Option<u32>,
    pub(crate) art: Option<Art>,
    /// Each overlay's art joined from its files, by row id.
    joined: HashMap<u32, (Joined, Vec<u8>)>,
    stamps: u64,
}

impl Overlays {
    /// Drop what belongs to the map that was chosen.
    pub(crate) fn leave(&mut self) {
        self.selected = None;
        self.art = None;
    }

    /// The overlays of the `WorldMapArea` row `map_area`, in file order.
    pub(crate) fn rows(session: &EditSession, map_area: u32) -> Vec<OverlayRow> {
        let Some(table) = session.table(OVERLAYS) else { return Vec::new() };
        (0..table.record_count())
            .filter(|record| table.u32_at(*record, of::MAP_AREA_ID) == Some(map_area))
            .filter_map(|record| {
                let word = |field| table.u32_at(record, field).unwrap_or(0);
                let mut areas = [0u32; 4];
                for (slot, field) in areas.iter_mut().zip(of::AREA_IDS) {
                    *slot = word(field);
                }
                Some(OverlayRow {
                    record,
                    id: table.u32_at(record, 0)?,
                    areas,
                    texture: table.string_at(record, of::TEXTURE).unwrap_or_default(),
                    width: word(of::TEXTURE_WIDTH),
                    height: word(of::TEXTURE_HEIGHT),
                    offset: (word(of::OFFSET_X), word(of::OFFSET_Y)),
                    hit: [word(of::HIT_TOP), word(of::HIT_LEFT), word(of::HIT_BOTTOM), word(of::HIT_RIGHT)],
                })
            })
            .collect()
    }

    /// An overlay's art joined from its files, read again when its name, its
    /// size or the project's files change.
    pub(crate) fn art_of(&mut self, row: &OverlayRow, directory: &str, session: &EditSession, assets: &GameAssets) -> &[u8] {
        let key = (row.texture.clone(), row.width, row.height, session.files_revision);
        let fresh = self.joined.get(&row.id).is_some_and(|(held, _)| *held == key);
        if !fresh {
            let rgba = pieces::join_overlay(row.width, row.height, |n| {
                let path = pieces::overlay_piece_path(directory, &row.texture, n);
                let bytes = assets.with_archive(|chain| Ok(chain.read(&path).ok())).ok().flatten()?;
                let decoded = blp::decode(&bytes).ok()?;
                Some((decoded.width, decoded.height, decoded.rgba))
            });
            self.joined.insert(row.id, (key, rgba));
        }
        &self.joined[&row.id].1
    }

    /// `base` with every overlay of `rows` drawn over it, new art in place of
    /// its row's.
    pub(crate) fn composite(
        &mut self,
        base: &[u8],
        rows: &[OverlayRow],
        directory: &str,
        session: &EditSession,
        assets: &GameAssets,
    ) -> Vec<u8> {
        let mut out = base.to_vec();
        for row in rows {
            if let Some(art) = self.art.as_ref().filter(|art| art.target == Target::Row(row.id)) {
                pieces::blend(&mut out, &art.rgba, art.width, art.height, row.offset.0, row.offset.1);
                continue;
            }
            let rgba = self.art_of(row, directory, session, assets).to_vec();
            pieces::blend(&mut out, &rgba, row.width, row.height, row.offset.0, row.offset.1);
        }
        if let Some(art) = self.art.as_ref().filter(|art| art.target == Target::New) {
            pieces::blend(&mut out, &art.rgba, art.width, art.height, art.offset.0, art.offset.1);
        }
        out
    }

    /// A stamp for the composite: changed whenever the art held changes.
    pub(crate) fn stamp(&self) -> u64 {
        self.stamps
    }

    /// Read a picture file as new art for `target`. A picture larger than the
    /// map's picture is scaled down to fit it, keeping its shape.
    pub(crate) fn read_art(&mut self, target: Target, path: &Path) -> Result<(), String> {
        let (width, height, rgba) = crate::import::assets::read_picture(path)?;
        let scale = (pieces::WIDTH as f32 / width.max(1) as f32).min(pieces::HEIGHT as f32 / height.max(1) as f32).min(1.0);
        let (w, h) = (((width as f32 * scale) as u32).max(1), ((height as f32 * scale) as u32).max(1));
        let rgba = match (w, h) == (width, height) {
            true => rgba,
            false => pieces::fit_to(&rgba, width, height, pieces::Fit::Stretch, w, h, true),
        };
        let offset = ((pieces::WIDTH - w) / 2, (pieces::HEIGHT - h) / 2);
        self.stamps += 1;
        self.art = Some(Art { target, path: path.to_path_buf(), width: w, height: h, rgba, offset });
        if let Target::Row(id) = target {
            self.selected = Some(id);
        }
        Ok(())
    }

    /// Move the new overlay's art to `offset`.
    pub(crate) fn place_new(&mut self, offset: (u32, u32)) {
        if let Some(art) = self.art.as_mut().filter(|art| art.target == Target::New) {
            let offset = (offset.0.min(pieces::WIDTH - 1), offset.1.min(pieces::HEIGHT - 1));
            if art.offset != offset {
                art.offset = offset;
                self.stamps += 1;
            }
        }
    }

    /// Write the new art's files, and for a new overlay add its row, as one
    /// undo entry for the row. Answers what happened.
    pub(crate) fn write(&mut self, entry: &Entry, session: &mut EditSession) -> String {
        let Some(row_id) = entry.area.as_ref().map(|area| area.id) else {
            return "the whole world has no explored areas".to_string();
        };
        let Some(art) = self.art.take() else { return "there is no new art to write".to_string() };
        let rows = Self::rows(session, row_id);
        let texture = match art.target {
            Target::Row(id) => match rows.iter().find(|row| row.id == id) {
                Some(row) => row.texture.clone(),
                None => return "the overlay is no longer in the table".to_string(),
            },
            Target::New => texture_name(&art.path, &rows),
        };
        let cut = pieces::overlay_pieces(&art.rgba, art.width, art.height);
        let count = cut.len();
        for (index, (w, h, rgba)) in cut.into_iter().enumerate() {
            let path = pieces::overlay_piece_path(&entry.directory, &texture, index as u32 + 1);
            let bytes = match blp::encode(&rgba, w, h, Encoding::Dxt3, true) {
                Ok(bytes) => bytes,
                Err(why) => return format!("{path} could not be encoded: {why}"),
            };
            if !session.save_bytes(&path, bytes) {
                return format!("{index} of {count} files written; {}", session.status);
            }
        }
        match art.target {
            Target::Row(id) => {
                let Some(row) = rows.iter().find(|row| row.id == id) else { return String::new() };
                if (row.width, row.height) != (art.width, art.height) {
                    rows::set_words(
                        session,
                        OVERLAYS,
                        row.record,
                        &[(of::TEXTURE_WIDTH, art.width), (of::TEXTURE_HEIGHT, art.height)],
                        "Overlay art size",
                        None,
                    );
                }
                format!("wrote {count} files of {texture}")
            }
            Target::New => {
                let [top, left, bottom, right] = pieces::default_hit(art.width, art.height, art.offset.0, art.offset.1);
                let area = entry.area.as_ref().map_or(0, |area| area.area);
                session.history.begin("Add overlay");
                let record = crate::tables::rows::push_row(
                    session,
                    OVERLAYS,
                    &[
                        (of::MAP_AREA_ID, row_id),
                        (of::AREA_IDS.start, area),
                        (of::TEXTURE_WIDTH, art.width),
                        (of::TEXTURE_HEIGHT, art.height),
                        (of::OFFSET_X, art.offset.0),
                        (of::OFFSET_Y, art.offset.1),
                        (of::HIT_TOP, top),
                        (of::HIT_LEFT, left),
                        (of::HIT_BOTTOM, bottom),
                        (of::HIT_RIGHT, right),
                    ],
                    &[(of::TEXTURE, texture.clone())],
                );
                session.history.end();
                self.selected = record.and_then(|record| session.table(OVERLAYS)?.u32_at(record, 0));
                format!(
                    "added overlay {texture} and wrote its {count} files; it shows when the zone itself is \
                     explored until its areas are set"
                )
            }
        }
    }
}

/// A new overlay's texture name: the picture's name in capitals, letters and
/// digits only as the shipped names are, with a number after it when the map
/// already has an overlay of that name.
fn texture_name(path: &Path, rows: &[OverlayRow]) -> String {
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("OVERLAY");
    let mut base: String = stem.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase();
    if base.is_empty() || base.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        // A name ending in a digit would run into the piece numbers.
        base.push('X');
    }
    let taken = |name: &str| rows.iter().any(|row| row.texture.eq_ignore_ascii_case(name));
    if !taken(&base) {
        return base;
    }
    (b'A'..=b'Z').map(|letter| format!("{base}{}", char::from(letter))).find(|name| !taken(name)).unwrap_or(base)
}

/// Move an overlay, as one undo entry for one drag.
pub(crate) fn set_offset(session: &mut EditSession, row: &OverlayRow, offset: (u32, u32), now: f64) {
    let (dx, dy) = (offset.0 as i64 - row.offset.0 as i64, offset.1 as i64 - row.offset.1 as i64);
    let shift = |value: u32, by: i64| (value as i64 + by).max(0) as u32;
    // The label rectangle moves with the art.
    rows::set_words(
        session,
        OVERLAYS,
        row.record,
        &[
            (of::OFFSET_X, offset.0),
            (of::OFFSET_Y, offset.1),
            (of::HIT_TOP, shift(row.hit[0], dy)),
            (of::HIT_LEFT, shift(row.hit[1], dx)),
            (of::HIT_BOTTOM, shift(row.hit[2], dy)),
            (of::HIT_RIGHT, shift(row.hit[3], dx)),
        ],
        "Move overlay",
        Some((&format!("overlay {}", row.id), now)),
    );
}

/// Set one of an overlay's four areas.
pub(crate) fn set_area(session: &mut EditSession, row: &OverlayRow, slot: usize, area: u32) {
    let field = of::AREA_IDS.start + slot;
    rows::set_words(session, OVERLAYS, row.record, &[(field, area)], "Overlay area", None);
}

/// Set an overlay's label rectangle: top, left, bottom, right.
pub(crate) fn set_hit(session: &mut EditSession, row: &OverlayRow, hit: [u32; 4], now: f64) {
    rows::set_words(
        session,
        OVERLAYS,
        row.record,
        &[(of::HIT_TOP, hit[0]), (of::HIT_LEFT, hit[1]), (of::HIT_BOTTOM, hit[2]), (of::HIT_RIGHT, hit[3])],
        "Overlay label",
        Some((&format!("overlay label {}", row.id), now)),
    );
}

/// Remove an overlay's row. Its files stay in the project until removed in
/// the Assets workspace.
pub(crate) fn remove(session: &mut EditSession, row: &OverlayRow) -> bool {
    crate::tables::rows::delete_row(session, OVERLAYS, row.record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(texture: &str) -> OverlayRow {
        OverlayRow {
            record: 0,
            id: 1,
            areas: [0; 4],
            texture: texture.to_string(),
            width: 1,
            height: 1,
            offset: (0, 0),
            hit: [0; 4],
        }
    }

    /// A new overlay is named after its picture in capitals, never ends in a
    /// digit, and takes a letter when the name is taken.
    #[test]
    fn a_new_overlay_is_named_after_its_picture() {
        assert_eq!(texture_name(Path::new("C:/art/Gold Shire.png"), &[]), "GOLDSHIRE");
        assert_eq!(texture_name(Path::new("tower2.png"), &[]), "TOWER2X");
        assert_eq!(texture_name(Path::new("goldshire.png"), &[row("GOLDSHIRE")]), "GOLDSHIREA");
    }
}
