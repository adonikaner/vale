//! `vale cursor` — the pointer the game draws, and where its point is.
//!
//! **The hotspot is the whole reason this exists.** A cursor bitmap says nothing
//! about which of its texels is the one the click lands on; the 1.12 client
//! knows and the file does not, so the alternative to measuring is a guess that
//! is wrong by a few pixels for the life of the project — and a few pixels of
//! cursor offset is exactly the kind of thing that is felt and never diagnosed.
//!
//! What can be measured from the archives is the **art's own tip**: these are
//! arrows and hands drawn against a transparent field, and the first opaque
//! texel scanning from the top-left corner is where the point is drawn. That is
//! not a proof — a cursor whose active spot is its middle (the four-way move
//! cursor, say) would be reported wrong — so this prints the shape it measured
//! beside it and the caller decides. For `Point.blp`, an arrow drawn into the
//! corner, they agree.
//!
//! It also answers the question that decides whether the renderer can use these
//! at all: winit wants 8-bit RGBA, and a BLP2 palette with a separate alpha
//! table is neither until `blp::decode` has run.

use crate::common::*;
use vale_assets::world::blp;
use vale_config::Config;

/// Everything under here is a mouse cursor.
const CURSORS: &str = "interface\\cursor\\";

pub fn cmd_cursor(cfg: &Config, one: Option<&str>) -> Result<(), String> {
    if let Some(name) = one {
        return trace(cfg, name);
    }
    let mut assets = open_assets(cfg)?;
    let files = assets.list_prefix(CURSORS);
    if files.is_empty() {
        return Err(format!(
            "no files under {CURSORS} — is the archive chain right?"
        ));
    }
    println!("{} file(s) under {CURSORS}\n", files.len());

    println!(
        "  {:<24} {:>9}  {:>6}  {:>6}  {:>9}  {}",
        "cursor", "size", "opaque", "clear", "tip", "note"
    );

    let (mut decoded, mut failed) = (0, 0);
    let mut square = 0;
    for path in &files {
        if !path.ends_with(".blp") {
            continue;
        }
        let name = path.rsplit('\\').next().unwrap_or(path);
        let Ok(raw) = assets.read(path) else {
            println!("  {name:<24}  in the listing and not readable");
            failed += 1;
            continue;
        };
        let image = match blp::decode(&raw) {
            Ok(image) => image,
            Err(e) => {
                println!("  {name:<24}  will not decode: {e}");
                failed += 1;
                continue;
            }
        };
        decoded += 1;
        if image.width == image.height {
            square += 1;
        }

        // Alpha is what makes a cursor a cursor: a fully opaque one would draw a
        // black square around the pointer, which is the failure this counts.
        let (mut opaque, mut clear) = (0usize, 0usize);
        for texel in image.rgba.chunks_exact(4) {
            match texel[3] {
                0 => clear += 1,
                255 => opaque += 1,
                _ => {}
            }
        }
        let texels = (image.width * image.height) as usize;

        // The tip: the first texel with any coverage at all, scanning rows from
        // the top. Reported as `(x, y)` in the image's own pixels.
        let tip = (0..image.height)
            .flat_map(|y| (0..image.width).map(move |x| (x, y)))
            .find(|(x, y)| {
                let i = ((y * image.width + x) * 4 + 3) as usize;
                image.rgba.get(i).is_some_and(|a| *a > 8)
            });

        let note = match tip {
            Some((0..=2, 0..=2)) => "drawn into the corner — hotspot 0,0",
            Some(_) => "tip is inset; check before trusting it",
            None => "!! nothing drawn",
        };
        println!(
            "  {name:<24} {:>4}x{:<4} {:>6} {:>6}  {:>9}  {note} ({} texels)",
            image.width,
            image.height,
            opaque,
            clear,
            tip.map(|(x, y)| format!("{x},{y}")).unwrap_or_default(),
            texels,
        );
    }

    println!("\n{decoded} decode, {failed} do not, {square} square");
    Ok(())
}

/// One cursor, drawn.
///
/// **The survey's `tip` column is a number and this is the picture behind it**,
/// which is the only way to tell "the arrow's point is in the corner" from "the
/// whole bitmap has a faint wash over it and the corner is part of it". The
/// first is a hotspot of 0,0; the second is a threshold picked too low, and the
/// two are indistinguishable from the number alone.
fn trace(cfg: &Config, name: &str) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let path = format!("{CURSORS}{}.blp", name.trim_end_matches(".blp").to_ascii_lowercase());
    let raw = assets
        .read(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    let image = blp::decode(&raw).map_err(|e| format!("{path}: {e}"))?;
    println!("{path}: {}x{}", image.width, image.height);

    let alpha = |x: u32, y: u32| -> u8 {
        let i = ((y * image.width + x) * 4 + 3) as usize;
        image.rgba.get(i).copied().unwrap_or(0)
    };

    // Five buckets, so a faint wash reads differently from an edge.
    let mut buckets = [0usize; 5];
    for texel in image.rgba.chunks_exact(4) {
        buckets[(texel[3] as usize * 5 / 256).min(4)] += 1;
    }
    println!(
        "  alpha: {} zero-ish, {} low, {} mid, {} high, {} full",
        buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
    );

    println!("  coverage (' ' clear, '.' faint, '+' half, '#' solid):");
    for y in 0..image.height.min(48) {
        let row: String = (0..image.width.min(64))
            .map(|x| match alpha(x, y) {
                0..=8 => ' ',
                9..=95 => '.',
                96..=200 => '+',
                _ => '#',
            })
            .collect();
        println!("    |{row}|");
    }
    Ok(())
}
