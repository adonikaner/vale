//! A guild's emblem on a tabard: the five numbers, the six textures they
//! name, and how the numbers travel with a character's equipment.
//!
//! ```text
//! Textures\GuildEmblems\Background_<bg>_TU_U.blp      torso upper, 128x64
//! Textures\GuildEmblems\Background_<bg>_TL_U.blp      torso lower, 128x32
//! Textures\GuildEmblems\Border_<style>_<colour>_TU_U.blp
//! Textures\GuildEmblems\Border_<style>_<colour>_TL_U.blp
//! Textures\GuildEmblems\Emblem_<style>_<colour>_TU_U.blp
//! Textures\GuildEmblems\Emblem_<style>_<colour>_TL_U.blp
//! ```
//!
//! Each number is written with at least two digits. The colours are baked
//! into the files: a colour number selects a file and there is no colour
//! table. The archives hold every combination inside [`COUNTS`].
//!
//! ## Which tabard takes the emblem
//!
//! A tabard whose `ItemDisplayInfo.dbc` row has bit 0 of its flags field set
//! is a guild tabard. On a wearer whose guild has an emblem, the 1.12.1
//! client paints the background, the border and the emblem over the chest,
//! in that order, in place of the tabard's own two textures. A guild tabard
//! on a wearer with no emblem, and every other tabard, paints its own
//! textures.
//!
//! ## The emblem travels as an equipment entry
//!
//! A character's look is its appearance and a list of `(display id,
//! inventory type)` pairs, and that list is what the renderer keys a
//! composed skin by. The emblem is carried as one more pair whose inventory
//! type is [`WORN`] or [`PREVIEW`] and whose first number is the five
//! numbers packed by [`pack`]. Neither value is an inventory type the game
//! has. [`crate::look::dress`] takes the pair out before it reads the list
//! as items.
//!
//! [`PREVIEW`] is the tabard designer's entry. The designer draws the emblem
//! on a character who wears no tabard, so a preview paints its layers
//! whatever is worn and turns the tabard's geoset on.

use crate::look::character::{regions, SkinLayer};

/// How many values each of the five numbers has: emblem style, emblem
/// colour, border style, border colour, background.
pub const COUNTS: [u32; 5] = [170, 17, 6, 17, 51];

/// The inventory type of the pair that carries a wearer's guild emblem.
pub const WORN: u32 = 0x4755_0001;

/// The inventory type of the pair that carries the tabard designer's design.
pub const PREVIEW: u32 = 0x4755_0002;

/// The bit of `ItemDisplayInfo.dbc`'s flags field that marks a guild tabard.
pub const DISPLAY_FLAG_GUILD_TABARD: u32 = 1;

/// The geoset the designer's preview turns on: group 12, second variant.
/// It is the geoset a guild tabard's own `ItemDisplayInfo.dbc` row selects,
/// whose first geoset number is 1. The group's first variant, 1201, is the
/// chest with no tabard over it.
pub const TABARD_GEOSET: u16 = 1202;

/// The five numbers as one, or `None` when any is outside its count. A guild
/// that has saved no emblem has -1 in every field and packs to nothing.
pub fn pack(emblem: [i32; 5]) -> Option<u32> {
    let mut packed = 0u32;
    for (value, count) in emblem.iter().zip(COUNTS) {
        let value = u32::try_from(*value).ok().filter(|v| *v < count)?;
        packed = packed * count + value;
    }
    Some(packed)
}

/// The inverse of [`pack`].
pub fn unpack(mut packed: u32) -> [i32; 5] {
    let mut emblem = [0i32; 5];
    for (value, count) in emblem.iter_mut().zip(COUNTS).rev() {
        *value = (packed % count) as i32;
        packed /= count;
    }
    emblem
}

/// The equipment pair for an emblem, or `None` for one that is not drawable.
pub fn entry(emblem: [i32; 5], preview: bool) -> Option<(u32, u32)> {
    Some((pack(emblem)?, if preview { PREVIEW } else { WORN }))
}

/// Whether an equipment pair carries an emblem and is not an item.
pub fn is_entry(pair: &(u32, u32)) -> bool {
    pair.1 == WORN || pair.1 == PREVIEW
}

/// The emblem an equipment list carries, and whether it is the designer's
/// preview. A preview wins over a worn emblem.
pub fn find(equipment: &[(u32, u32)]) -> Option<([i32; 5], bool)> {
    let of = |kind: u32| equipment.iter().find(|pair| pair.1 == kind).map(|pair| unpack(pair.0));
    of(PREVIEW)
        .map(|emblem| (emblem, true))
        .or_else(|| of(WORN).map(|emblem| (emblem, false)))
}

/// The six layers of an emblem in paint order: the background, the border
/// and the emblem, each on the upper and the lower torso.
pub fn layers(emblem: [i32; 5]) -> Vec<SkinLayer> {
    let [style, colour, border, border_colour, background] = emblem;
    let mut out = Vec::with_capacity(6);
    for name in [
        format!("Background_{background:02}"),
        format!("Border_{border:02}_{border_colour:02}"),
        format!("Emblem_{style:02}_{colour:02}"),
    ] {
        for (suffix, region) in [("TU", regions::TORSO_UPPER), ("TL", regions::TORSO_LOWER)] {
            out.push(SkinLayer {
                path: format!("Textures\\GuildEmblems\\{name}_{suffix}_U.blp"),
                region,
            });
        }
    }
    out
}

/// The path of the emblem alone, without an extension, as the tabard
/// designer's two `Get…EmblemFileName` methods answer it. `upper` chooses
/// the upper torso's file.
pub fn emblem_file(emblem: [i32; 5], upper: bool) -> String {
    format!(
        "Textures\\GuildEmblems\\Emblem_{:02}_{:02}_{}_U",
        emblem[0],
        emblem[1],
        if upper { "TU" } else { "TL" }
    )
}

/// The path of the background, without an extension, as the designer's two
/// `Get…BackgroundFileName` methods answer it.
pub fn background_file(emblem: [i32; 5], upper: bool) -> String {
    format!(
        "Textures\\GuildEmblems\\Background_{:02}_{}_U",
        emblem[4],
        if upper { "TU" } else { "TL" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every design inside the counts packs to a distinct number and comes
    /// back unchanged, and the largest fits in 32 bits.
    #[test]
    fn a_design_round_trips_through_one_number() {
        for emblem in [[0, 0, 0, 0, 0], [169, 16, 5, 16, 50], [3, 4, 5, 6, 7], [100, 0, 0, 0, 0]] {
            assert_eq!(unpack(pack(emblem).expect("in range")), emblem);
        }
        let combinations: u64 = COUNTS.iter().map(|c| u64::from(*c)).product();
        assert!(combinations < u64::from(u32::MAX));
        assert_ne!(pack([1, 0, 0, 0, 0]), pack([0, 1, 0, 0, 0]));
    }

    /// A guild with no emblem has -1 in every field, and a number at its
    /// count is outside it.
    #[test]
    fn a_design_outside_the_counts_is_not_packed() {
        assert_eq!(pack([-1; 5]), None);
        assert_eq!(pack([170, 0, 0, 0, 0]), None);
        assert_eq!(pack([0, 0, 6, 0, 0]), None);
        assert_eq!(entry([-1; 5], false), None);
    }

    /// The pair is found among items by its inventory type, and the
    /// designer's preview is preferred to the worn emblem.
    #[test]
    fn the_emblem_is_found_among_the_items() {
        let worn = entry([3, 4, 5, 6, 7], false).expect("in range");
        let preview = entry([8, 9, 1, 2, 3], true).expect("in range");
        assert!(is_entry(&worn) && is_entry(&preview));
        assert!(!is_entry(&(20_621, 19)));
        assert_eq!(find(&[(20_621, 19), worn]), Some(([3, 4, 5, 6, 7], false)));
        assert_eq!(find(&[worn, preview]), Some(([8, 9, 1, 2, 3], true)));
        assert_eq!(find(&[(20_621, 19)]), None);
    }

    /// The layers are background, border, emblem, each upper then lower, and
    /// a number past 99 is written with three digits.
    #[test]
    fn the_layers_are_background_border_emblem() {
        let paths: Vec<String> = layers([120, 4, 5, 6, 7]).into_iter().map(|l| l.path).collect();
        assert_eq!(
            paths,
            [
                r"Textures\GuildEmblems\Background_07_TU_U.blp",
                r"Textures\GuildEmblems\Background_07_TL_U.blp",
                r"Textures\GuildEmblems\Border_05_06_TU_U.blp",
                r"Textures\GuildEmblems\Border_05_06_TL_U.blp",
                r"Textures\GuildEmblems\Emblem_120_04_TU_U.blp",
                r"Textures\GuildEmblems\Emblem_120_04_TL_U.blp",
            ]
        );
        assert_eq!(layers([0; 5])[1].region, regions::TORSO_LOWER);
        assert_eq!(emblem_file([3, 4, 0, 0, 0], true), r"Textures\GuildEmblems\Emblem_03_04_TU_U");
        assert_eq!(background_file([0, 0, 0, 0, 9], false), r"Textures\GuildEmblems\Background_09_TL_U");
    }
}
