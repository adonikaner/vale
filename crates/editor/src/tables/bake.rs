//! An NPC's baked skin, made again from its `CreatureDisplayInfoExtra` row.
//!
//! The game ships the body texture of an NPC in a character model as a file,
//! the row's `BakeName`, baked offline from the row's seven appearance ids and
//! its ten item columns. The renderer reads the item columns for geometry only
//! (a boot's geoset, a pauldron's model), so a changed face, hair colour or
//! garment colour is not seen until the skin is baked again.
//!
//! [`rebake`] composes the skin the way the client composes a player's:
//! `vale_assets::look::dress::worn_skin_recipe` for the layers and
//! `vale_assets::look::character::paint_skin` to paint them. It writes the
//! result into the project in the shipped bakes' own form (BLP2, DXT1 with no
//! alpha, 256x256, with mips) and points the row at it, as one undo entry.
//!
//! The file is named by the row and by its contents, `x<id>_<hash>.blp`, and a
//! rebake never writes over a file. Two rows can name one shipped bake, and an
//! undo of a rebake names the earlier file again, which must still be there.

use crate::session::EditSession;
use vale_assets::look::character::Appearance;
use vale_assets::tables::item::Equipped;
use vale_client::assets::GameAssets;

/// The table a bake is a column of.
pub(crate) const EXTRA: &str = "CreatureDisplayInfoExtra";

/// `CreatureDisplayInfoExtra`'s columns, as `vale_assets::tables::schema`
/// describes them: the seven appearance ids from 1, the ten item display ids
/// from 8, and the bake's file name.
const APPEARANCE: usize = 1;
const ITEMS: usize = 8;
pub(crate) const BAKE_NAME: usize = 18;

/// `CreatureDisplayInfo`'s `ExtendedDisplayInfoID`: the `Extra` row a display
/// in a character model names.
const EXTENDED: usize = 3;

/// The folder a `BakeName` is a file of.
pub(crate) const BAKED_FOLDER: &str = "Textures\\BakedNpcTextures\\";

/// The `Extra` row the `CreatureDisplayInfo` record names, or `None` for a
/// display that names none: a creature with a skin of its own.
pub(crate) fn extra_of(session: &EditSession, record: usize) -> Option<u32> {
    session
        .table("CreatureDisplayInfo")?
        .u32_at(record, EXTENDED)
        .filter(|&id| id != 0)
}

/// What an `Extra` record is: the appearance, and what it wears in each of
/// the ten slots that hold an item.
pub(crate) fn wearing(table: &vale_edit::dbc::DbcFile, record: usize) -> Option<(Appearance, Vec<Equipped>)> {
    let id = |field: usize| table.u32_at(record, field).map(|value| value as u8);
    let appearance = Appearance {
        race: id(APPEARANCE)?,
        gender: id(APPEARANCE + 1)?,
        skin: id(APPEARANCE + 2)?,
        face: id(APPEARANCE + 3)?,
        hair_style: id(APPEARANCE + 4)?,
        hair_colour: id(APPEARANCE + 5)?,
        facial_hair: id(APPEARANCE + 6)?,
    };
    let worn = vale_assets::tables::dbc::DisplayTables::EQUIPPED_SLOTS
        .iter()
        .enumerate()
        .filter_map(|(column, &slot)| {
            let display_id = table.u32_at(record, ITEMS + column)?;
            (display_id != 0).then_some(Equipped { display_id, slot })
        })
        .collect();
    Some((appearance, worn))
}

/// The file name a bake of row `id` is saved under: the row, and a hash of the
/// file, so a different skin is a different file and the same skin baked twice
/// is one.
pub(crate) fn bake_name(id: u32, blp: &[u8]) -> String {
    // FNV-1a, 64 bits: stable across builds, unlike the standard hasher.
    let hash = blp.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("x{id}_{hash:016x}.blp")
}

/// Whether the `Extra` record `record` of `table` wears a skin that was not
/// baked from it: its appearance or worn items differ from the archives' row
/// of the same id (`original`, the archives' table and that row's record), or
/// the archives have no such row, and its `BakeName` is not a bake of this row
/// that the project holds.
///
/// A row rebaked here and edited again is not caught: its `BakeName` is a
/// bake of the row, made from what the row was before the edit.
pub(crate) fn needs_rebake(
    table: &vale_edit::dbc::DbcFile,
    record: usize,
    original: Option<(&vale_edit::dbc::DbcFile, usize)>,
    project: &vale_edit::project::Project,
) -> bool {
    let Some(id) = table.u32_at(record, 0) else { return false };
    let bake = table.string_at(record, BAKE_NAME).unwrap_or_default();
    let own_bake = bake.to_ascii_lowercase().starts_with(&format!("x{id}_"))
        && project.has(&format!("{BAKED_FOLDER}{bake}"));
    if own_bake {
        return false;
    }
    let Some((original, at)) = original else {
        return true;
    };
    (APPEARANCE..BAKE_NAME).any(|field| table.u32_at(record, field) != original.u32_at(at, field))
}

/// Bake the skin of the `Extra` row `id` again and point its `BakeName` at the
/// new file. Answers what was done, for the status line, or why nothing was.
pub(crate) fn rebake(session: &mut EditSession, assets: &GameAssets, id: u32, now: f64) -> Result<String, String> {
    if !session.open_table(assets, EXTRA) {
        return Err(format!("{EXTRA}.dbc could not be opened"));
    }
    let table = session.table(EXTRA).ok_or_else(|| format!("{EXTRA}.dbc is not open"))?;
    let record = table.row_of(id).ok_or_else(|| format!("{EXTRA} has no row {id}"))?;
    let (appearance, worn) = wearing(table, record).ok_or_else(|| format!("{EXTRA} row {id} is too short"))?;
    let old = table.string_at(record, BAKE_NAME).unwrap_or_default();
    let (layers, rgba) = compose(assets, &appearance, &worn)?;
    let side = vale_assets::look::character::COMPOSITE_SIDE;
    let blp = vale_assets::world::blp::encode(&rgba, side, side, vale_assets::world::blp::Encoding::Dxt1, true)
        .map_err(|e| e.to_string())?;
    let name = bake_name(id, &blp);
    if name.eq_ignore_ascii_case(&old) {
        return Ok(format!("{EXTRA} {id}: the skin is already baked from this row"));
    }
    let size = blp.len();
    if !session.save_bytes(&format!("{BAKED_FOLDER}{name}"), blp) {
        return Err(session.status.clone());
    }
    crate::tables::fields::set_text(session, EXTRA, record, BAKE_NAME, &name, "Rebake skin", now);
    Ok(format!("{EXTRA} {id}: baked {name} from {layers} layers ({size} bytes) and set it as BakeName"))
}

/// The skin an appearance wearing `worn` is baked as: how many layers the
/// recipe has, and the 256x256 RGBA texture.
///
/// The item rows come from the client's display tables, which hold the
/// project's rows once they are published. `CharSections` and every layer are
/// read through the overlay, so a project's own textures are painted.
fn compose(assets: &GameAssets, appearance: &Appearance, worn: &[Equipped]) -> Result<(usize, Vec<u8>), String> {
    let tables = assets.display_tables().map_err(|e| e.to_string())?;
    let (layers, rgba) = assets
        .with_archive(|chain| {
            let sections = chain
                .read(&vale_assets::tables::dbc::dbc_path("CharSections"))
                .ok()
                .and_then(|raw| vale_assets::look::character::CharSections::parse(&raw).ok());
            let Some(sections) = sections else {
                return Ok((0, None));
            };
            let recipe = vale_assets::look::dress::worn_skin_recipe(
                &sections,
                tables.items(),
                appearance,
                worn,
                |path| chain.exists(path),
            );
            // The base skin is the one layer over the whole atlas. Without it
            // the garments are painted over nothing, which is worse than the
            // bake the row has; some shipped rows name a skin tone the
            // shipped `CharSections` lacks (an orc male's 9).
            let based = recipe
                .layers
                .iter()
                .any(|layer| layer.region == vale_assets::look::character::regions::BASE);
            let rgba = based
                .then(|| vale_assets::look::character::paint_skin(&recipe, |path| chain.read(path).ok()))
                .flatten();
            Ok((recipe.layers.len(), rgba))
        })
        .map_err(|e| e.to_string())?;
    let rgba = rgba.ok_or_else(|| {
        format!(
            "nothing was baked: CharSections has no base skin for race {}, gender {}, skin {}",
            appearance.race, appearance.gender, appearance.skin
        )
    })?;
    Ok((layers, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row's appearance is its seven ids in order, and an item column that
    /// holds 0 wears nothing in its slot.
    #[test]
    fn a_row_wears_its_appearance_and_its_filled_slots() {
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, 19, 19 * 4, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        let mut table = vale_edit::dbc::DbcFile::parse(&bytes).unwrap();
        let blank = table.blank_record(5);
        let record = table.push_record(&blank).unwrap();
        for (field, value) in [(1, 3), (2, 1), (3, 4), (5, 6), (9, 1234), (11, 777)] {
            table.set_u32(record, field, value);
        }
        let (appearance, worn) = wearing(&table, record).unwrap();
        assert_eq!((appearance.race, appearance.gender, appearance.skin), (3, 1, 4));
        assert_eq!(appearance.hair_style, 6);
        use vale_assets::tables::item::Slot;
        let slots: Vec<(u32, Slot)> = worn.iter().map(|item| (item.display_id, item.slot)).collect();
        assert_eq!(slots, vec![(1234, Slot::Shoulders), (777, Slot::Chest)]);
    }

    /// A bake is named by its row and its bytes, so different bytes give a
    /// different name.
    #[test]
    fn a_bake_is_named_by_its_row_and_its_contents() {
        let a = bake_name(12, b"one");
        assert!(a.starts_with("x12_") && a.ends_with(".blp"));
        assert_eq!(a, bake_name(12, b"one"));
        assert_ne!(a, bake_name(12, b"two"));
        assert_ne!(a, bake_name(13, b"one"));
    }
}
