//! A tile's heights and texture blends as picture files, written out and read
//! back in.
//!
//! What the pictures are and how a pixel becomes a height is
//! `vale_edit::ops::image`. This file is the half that touches the disk: the
//! folder, the file names, the PNG encoding, and the edit an import becomes.
//!
//! ## The files
//!
//! ```text
//! Edit\<project>\images\
//!   Azeroth_32_48.height.png   257 x 257, 16-bit greyscale
//!   Azeroth_32_48.height.txt   the heights black and white stand for
//!   Azeroth_32_48.blend.png    1024 x 1024, 8-bit RGB
//! ```
//!
//! The folder is beside the project's `project\` folder and not inside it:
//! these are not files the game reads, and a publish must not pack them.
//!
//! The range is a text file beside the picture, `low high` in yards, because
//! a paint program or a terrain generator writes the picture back without any
//! text it carried. An import with no range file is refused, since a height
//! map with no range is a shape with no size. Editing the two numbers before
//! an import rescales the whole tile.
//!
//! ## An import is one undo entry
//!
//! Every tile imported in one press is one entry. With the switch on, what
//! stands on a tile is carried by as much as the ground under it moved, as a
//! height stroke carries it (`super::terrain::carry`). A tile is imported from
//! whichever of its two pictures is in the folder, so a height map can be
//! brought in alone.
//!
//! Both operations act on tiles the session has open, which are the ones near
//! the camera. The map window counts them on the buttons.

use crate::session::EditSession;
use std::path::{Path, PathBuf};
use vale_edit::ops::image::{self, HeightMap, BLEND_SIDE, HEIGHT_SIDE};

/// The folder the pictures are written to and read from.
pub fn folder(session: &EditSession) -> PathBuf {
    session.project.root.join("images")
}

/// The three files of one tile, without the folder: the height map, its
/// range, and the blend map.
pub fn names(map: &str, at: (u32, u32)) -> [String; 3] {
    let stem = format!("{map}_{}_{}", at.0, at.1);
    [
        format!("{stem}.height.png"),
        format!("{stem}.height.txt"),
        format!("{stem}.blend.png"),
    ]
}

fn said(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// Encode one picture. `sixteen` writes one 16-bit grey channel from
/// big-endian pairs; otherwise three 8-bit channels.
fn write_png(path: &Path, side: usize, sixteen: bool, bytes: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| said(path, e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), side as u32, side as u32);
    match sixteen {
        true => {
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Sixteen);
        }
        false => {
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
        }
    }
    let mut writer = encoder.write_header().map_err(|e| said(path, e))?;
    writer.write_image_data(bytes).map_err(|e| said(path, e))
}

/// Decode one picture into its size, colour type, depth and bytes.
fn read_png(path: &Path) -> Result<(usize, usize, png::ColorType, png::BitDepth, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| said(path, e))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .map_err(|e| said(path, e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| said(path, "too large to read"))?;
    let mut bytes = vec![0u8; size];
    let frame = reader.next_frame(&mut bytes).map_err(|e| said(path, e))?;
    bytes.truncate(frame.buffer_size());
    Ok((
        frame.width as usize,
        frame.height as usize,
        frame.color_type,
        frame.bit_depth,
        bytes,
    ))
}

/// Write a height map and its range file.
pub fn write_heights(dir: &Path, names: &[String; 3], map: &HeightMap) -> Result<(), String> {
    let bytes: Vec<u8> = map.pixels.iter().flat_map(|pixel| pixel.to_be_bytes()).collect();
    write_png(&dir.join(&names[0]), HEIGHT_SIDE, true, &bytes)?;
    let range = dir.join(&names[1]);
    std::fs::write(&range, format!("{} {}\n", map.low, map.high)).map_err(|e| said(&range, e))
}

/// Read a height map and its range file. A picture that is not 257 pixels
/// square is refused, and so is one with no range beside it. An 8-bit
/// greyscale picture is accepted and widened, since some programs save one
/// whatever they were given; it has 256 levels where the file had 65,536.
pub fn read_heights(dir: &Path, names: &[String; 3]) -> Result<HeightMap, String> {
    let picture = dir.join(&names[0]);
    let (width, height, colour, depth, bytes) = read_png(&picture)?;
    if width != HEIGHT_SIDE || height != HEIGHT_SIDE {
        return Err(said(
            &picture,
            format!("{width} x {height}, and a height map is {HEIGHT_SIDE} x {HEIGHT_SIDE}"),
        ));
    }
    let pixels: Vec<u16> = match (colour, depth) {
        (png::ColorType::Grayscale, png::BitDepth::Sixteen) => bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect(),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => {
            bytes.iter().map(|&grey| u16::from(grey) * 257).collect()
        }
        other => {
            return Err(said(
                &picture,
                format!("{other:?}, and a height map is 16-bit greyscale"),
            ))
        }
    };
    let range = dir.join(&names[1]);
    let text = std::fs::read_to_string(&range)
        .map_err(|_| said(&range, "missing: it holds the two heights black and white stand for"))?;
    let mut numbers = text.split_whitespace().map(str::parse::<f32>);
    let (Some(Ok(low)), Some(Ok(high))) = (numbers.next(), numbers.next()) else {
        return Err(said(&range, "not two numbers"));
    };
    if !(high > low) {
        return Err(said(&range, "the second height must be above the first"));
    }
    Ok(HeightMap { low, high, pixels })
}

/// Write a blend map.
pub fn write_blend(dir: &Path, names: &[String; 3], rgb: &[u8]) -> Result<(), String> {
    write_png(&dir.join(&names[2]), BLEND_SIDE, false, rgb)
}

/// Read a blend map as RGB. A picture saved with an alpha channel has it
/// dropped.
pub fn read_blend(dir: &Path, names: &[String; 3]) -> Result<Vec<u8>, String> {
    let picture = dir.join(&names[2]);
    let (width, height, colour, depth, bytes) = read_png(&picture)?;
    if width != BLEND_SIDE || height != BLEND_SIDE {
        return Err(said(
            &picture,
            format!("{width} x {height}, and a blend map is {BLEND_SIDE} x {BLEND_SIDE}"),
        ));
    }
    match (colour, depth) {
        (png::ColorType::Rgb, png::BitDepth::Eight) => Ok(bytes),
        (png::ColorType::Rgba, png::BitDepth::Eight) => Ok(bytes
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect()),
        other => Err(said(&picture, format!("{other:?}, and a blend map is 8-bit RGB"))),
    }
}

/// Write both pictures of every open tile in `tiles`. Returns the status
/// line.
pub fn export(session: &EditSession, tiles: &[(u32, u32)]) -> String {
    let dir = folder(session);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return said(&dir, e);
    }
    let mut written = 0usize;
    for &at in tiles {
        let Some(tile) = session.tiles.get(&at) else {
            continue;
        };
        let names = names(&session.map, at);
        let Some(map) = image::export_heights(tile) else {
            return format!("tile {}, {} is missing chunks and was not exported", at.0, at.1);
        };
        if let Err(e) = write_heights(&dir, &names, &map) {
            return e;
        }
        if let Err(e) = write_blend(&dir, &names, &image::export_blend(tile)) {
            return e;
        }
        written += 1;
    }
    match written {
        0 => "none of the selection is open: fly to a tile to open it".to_string(),
        1 => format!("exported 1 tile's height and blend maps to {}", dir.display()),
        n => format!("exported {n} tiles' height and blend maps to {}", dir.display()),
    }
}

/// Read whichever pictures the folder holds for each open tile in `tiles`
/// and write them onto the tile, as one undo entry. Returns the status line.
///
/// A picture that cannot be read stops the import at that tile and the line
/// says why; the tiles before it stay imported, in the entry.
pub fn import(session: &mut EditSession, tiles: &[(u32, u32)], objects_follow: bool) -> String {
    let dir = folder(session);
    let (mut tiles_changed, mut chunks) = (0usize, 0usize);
    let mut found = 0usize;
    let mut stopped: Option<String> = None;
    let mut standing = Vec::new();
    session.history.begin("Import images");
    for &at in tiles {
        if !session.tiles.contains_key(&at) {
            continue;
        }
        let names = names(&session.map, at);
        let heights = match dir.join(&names[0]).exists() {
            true => match read_heights(&dir, &names) {
                Ok(map) => Some(map),
                Err(e) => {
                    stopped = Some(e);
                    break;
                }
            },
            false => None,
        };
        let blend = match dir.join(&names[2]).exists() {
            true => match read_blend(&dir, &names) {
                Ok(rgb) => Some(rgb),
                Err(e) => {
                    stopped = Some(e);
                    break;
                }
            },
            false => None,
        };
        if heights.is_none() && blend.is_none() {
            continue;
        }
        found += 1;
        let key = session.key(at);
        let Some(tile) = session.tiles.get_mut(&at) else {
            continue;
        };
        let mut edits = Vec::new();
        if let Some(map) = &heights {
            if objects_follow {
                standing.push((at, vale_edit::ops::follow::standing(tile)));
            }
            edits.extend(image::import_heights(tile, map));
        }
        if let Some(rgb) = &blend {
            edits.extend(image::import_blend(tile, rgb));
        }
        if edits.is_empty() {
            continue;
        }
        tiles_changed += 1;
        chunks += edits
            .iter()
            .filter(|edit| {
                matches!(
                    edit,
                    vale_edit::ops::Edit::Heights { .. } | vale_edit::ops::Edit::Paint { .. }
                )
            })
            .count();
        session.history.record(&key, edits);
    }
    let carried = super::terrain::carry(session, standing);
    session.history.end();
    // The whole tile is read again: an import can move every vertex and
    // every blend of it, which is more than the live paths patch.
    for &at in tiles {
        if session.tiles.contains_key(&at) && tiles_changed > 0 {
            session.publish(at);
            session.stale.insert(at);
            session.unsaved.insert(at);
        }
    }
    if let Some(why) = stopped {
        return format!("import stopped: {why}");
    }
    match (found, tiles_changed) {
        (0, _) => format!("no picture of the selection is in {}", dir.display()),
        (_, 0) => "the pictures say what the tiles already hold: nothing changed".to_string(),
        (_, n) => {
            let mut line = format!(
                "imported {n} tile{}: {chunks} chunk change{}",
                if n == 1 { "" } else { "s" },
                if chunks == 1 { "" } else { "s" }
            );
            if carried > 0 {
                line.push_str(&format!(", {carried} objects followed the ground"));
            }
            line
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vale-images-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A height map written and read back is the same sixteen bits and the
    /// same range, and one with no range file is refused.
    #[test]
    fn a_height_map_file_reads_back_as_it_was_written() {
        let dir = scratch("height");
        let names = names("Azeroth", (32, 48));
        assert_eq!(names[0], "Azeroth_32_48.height.png");
        let map = HeightMap {
            low: 18.25,
            high: 170.5,
            pixels: (0..HEIGHT_SIDE * HEIGHT_SIDE).map(|i| (i * 7 % 65536) as u16).collect(),
        };
        write_heights(&dir, &names, &map).unwrap();
        assert_eq!(read_heights(&dir, &names).unwrap(), map);

        std::fs::remove_file(dir.join(&names[1])).unwrap();
        let refused = read_heights(&dir, &names).unwrap_err();
        assert!(refused.contains("missing"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A blend map written and read back is the same bytes, and a picture of
    /// another size is refused with both sizes named.
    #[test]
    fn a_blend_map_file_reads_back_as_it_was_written() {
        let dir = scratch("blend");
        let names = names("Azeroth", (32, 48));
        let rgb: Vec<u8> = (0..BLEND_SIDE * BLEND_SIDE * 3).map(|i| (i % 251) as u8).collect();
        write_blend(&dir, &names, &rgb).unwrap();
        assert_eq!(read_blend(&dir, &names).unwrap(), rgb);

        write_png(&dir.join(&names[2]), 64, false, &vec![0u8; 64 * 64 * 3]).unwrap();
        let refused = read_blend(&dir, &names).unwrap_err();
        assert!(refused.contains("64 x 64") && refused.contains("1024 x 1024"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
