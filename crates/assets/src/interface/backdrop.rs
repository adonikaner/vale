//! The layout of a `<Backdrop>`'s `edgeFile`: one strip of eight square cells,
//! two of which are stored rotated 90°.
//!
//! ```xml
//! <Backdrop bgFile="Interface\Tooltips\UI-Tooltip-Background"
//!           edgeFile="Interface\Tooltips\UI-Tooltip-Border" tile="true">
//!     <EdgeSize><AbsValue val="16"/></EdgeSize>
//!     <TileSize><AbsValue val="16"/></TileSize>
//!     <BackgroundInsets><AbsInset left="5" right="5" top="5" bottom="5"/></BackgroundInsets>
//! </Backdrop>
//! ```
//!
//! The file it names is a strip eight cells wide and one cell tall.
//! `UI-Tooltip-Border.blp` is 128x16 and `UI-DialogBox-Border.blp` is 256x32:
//! `8 * height` in both cases, with the cell size equal to the `<EdgeSize>` the
//! markup gives. The cell size is therefore the strip's height, and the numbers
//! in the XML are not needed to find it.
//!
//! ## Cell order, and the two rotated cells
//!
//! ```text
//! 0 LEFT   1 RIGHT   2 TOP   3 BOTTOM   4 TOPLEFT  5 TOPRIGHT  6 BOTTOMLEFT  7 BOTTOMRIGHT
//! ```
//!
//! Six of the eight are upright. `TOP` and `BOTTOM` are stored rotated 90°: the
//! client maps the atlas's u to screen y and its v to screen x reversed.
//! Reading them unrotated draws the tooltip's top border on its side, which
//! looks plausible rather than obviously wrong. They are un-rotated here so that
//! every piece is drawn with upright coordinates:
//!
//! ```text
//! dst(x, y) = src(2 * cell + y, cell - 1 - x)     TOP
//! dst(x, y) = src(3 * cell + y, cell - 1 - x)     BOTTOM
//! ```
//!
//! The order and the un-rotation follow the client's UV constants. This project
//! measured only the implied shape: the
//! two edge files 5875's FrameXML names are both `8 * height` wide. That is the
//! only part of the rule a file can confirm.
//!
//! ## Why the strip is split into cells at decode time
//!
//! A renderer could sample the strip in place for six of the pieces but would
//! still need a rotated quad for the other two. Splitting once, at decode, lets
//! the renderer draw eight ordinary quads. The un-rotation is a property of the
//! file, not of the window, so it belongs in the decoder.

/// The eight pieces, in the strip's own order.
pub const PIECES: [&str; 8] = [
    "LEFT",
    "RIGHT",
    "TOP",
    "BOTTOM",
    "TOPLEFT",
    "TOPRIGHT",
    "BOTTOMLEFT",
    "BOTTOMRIGHT",
];

/// Index into [`PIECES`] and into [`split_edges`]'s answer.
pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const TOP: usize = 2;
pub const BOTTOM: usize = 3;
pub const TOPLEFT: usize = 4;
pub const TOPRIGHT: usize = 5;
pub const BOTTOMLEFT: usize = 6;
pub const BOTTOMRIGHT: usize = 7;

/// The four pieces that tile along their length instead of mapping once.
///
/// A border's edges repeat at the cell's period whatever the side's length and
/// are never stretched; a corner maps `[0, 1]` exactly once. This decides how
/// the texture is sampled, so it is stated here beside the order rather than in
/// the pass that draws it.
pub fn tiles(piece: usize) -> bool {
    matches!(piece, LEFT | RIGHT | TOP | BOTTOM)
}

/// Split an edge strip into its eight upright cells.
///
/// `rgba` is the decoded strip, `width` and `height` its size in texels. Returns
/// `None` for anything that is not `8 * height` wide; that is the only
/// validation. A file of the wrong shape is not drawn, so the border is never
/// built from the wrong texels.
pub fn split_edges(width: usize, height: usize, rgba: &[u8]) -> Option<[Vec<u8>; 8]> {
    let cell = height;
    if cell == 0 || width != PIECES.len() * cell || rgba.len() < width * height * 4 {
        return None;
    }
    let texel = |x: usize, y: usize| {
        let at = (y * width + x) * 4;
        &rgba[at..at + 4]
    };
    let mut cells: [Vec<u8>; 8] = std::array::from_fn(|_| vec![0u8; cell * cell * 4]);
    for y in 0..cell {
        for x in 0..cell {
            let at = (y * cell + x) * 4;
            let mut put = |piece: usize, source: &[u8]| {
                cells[piece][at..at + 4].copy_from_slice(source);
            };
            put(LEFT, texel(x, y));
            put(RIGHT, texel(cell + x, y));
            // The two on their side — see the module comment.
            put(TOP, texel(2 * cell + y, cell - 1 - x));
            put(BOTTOM, texel(3 * cell + y, cell - 1 - x));
            put(TOPLEFT, texel(4 * cell + x, y));
            put(TOPRIGHT, texel(5 * cell + x, y));
            put(BOTTOMLEFT, texel(6 * cell + x, y));
            put(BOTTOMRIGHT, texel(7 * cell + x, y));
        }
    }
    Some(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip whose every texel says where it came from: red is x, green is y.
    fn strip(cell: usize) -> (usize, usize, Vec<u8>) {
        let width = PIECES.len() * cell;
        let mut rgba = vec![0u8; width * cell * 4];
        for y in 0..cell {
            for x in 0..width {
                let at = (y * width + x) * 4;
                rgba[at] = x as u8;
                rgba[at + 1] = y as u8;
                rgba[at + 3] = 0xFF;
            }
        }
        (width, cell, rgba)
    }

    /// Six pieces are copies and two are rotated upright, checked by texel
    /// coordinates.
    #[test]
    fn the_strip_splits_into_eight_cells_with_top_and_bottom_un_rotated() {
        let cell = 4;
        let (width, height, rgba) = strip(cell);
        let cells = split_edges(width, height, &rgba).expect("an 8-cell strip");
        let at = |piece: usize, x: usize, y: usize| {
            let i = (y * cell + x) * 4;
            (cells[piece][i], cells[piece][i + 1])
        };

        // The upright six are their slice, texel for texel.
        assert_eq!(at(LEFT, 0, 0), (0, 0));
        assert_eq!(at(LEFT, 3, 2), (3, 2));
        assert_eq!(at(RIGHT, 0, 0), (cell as u8, 0));
        assert_eq!(at(TOPLEFT, 0, 0), (4 * cell as u8, 0));
        assert_eq!(at(TOPRIGHT, 1, 1), (5 * cell as u8 + 1, 1));
        assert_eq!(at(BOTTOMLEFT, 0, 0), (6 * cell as u8, 0));
        assert_eq!(at(BOTTOMRIGHT, 0, 0), (7 * cell as u8, 0));

        // TOP and BOTTOM: dst(x, y) = src(2c + y, c - 1 - x).
        assert_eq!(at(TOP, 0, 0), (2 * cell as u8, cell as u8 - 1));
        assert_eq!(at(TOP, 1, 0), (2 * cell as u8, cell as u8 - 2));
        assert_eq!(at(TOP, 0, 1), (2 * cell as u8 + 1, cell as u8 - 1));
        assert_eq!(at(BOTTOM, 0, 0), (3 * cell as u8, cell as u8 - 1));

        // Every cell is exactly one square — nothing padded, nothing stretched.
        assert!(cells.iter().all(|c| c.len() == cell * cell * 4));
    }

    /// A file of the wrong shape is declined instead of split from the wrong
    /// texels. The two edge files 5875 names are 128x16 and 256x32; any other
    /// shape means this client resolved the wrong path, and a border built from
    /// an unrelated texture would look like a bad texture rather than an error.
    #[test]
    fn a_strip_that_is_not_eight_cells_wide_is_declined() {
        let (_, height, rgba) = strip(4);
        assert!(split_edges(31, height, &rgba).is_none());
        assert!(split_edges(64, 8, &rgba).is_none(), "too few texels for the claim");
        assert!(split_edges(0, 0, &[]).is_none());
    }

    /// The two shapes the archives ship satisfy the `8 * height` rule.
    #[test]
    fn the_two_edge_files_the_directory_names_are_eight_cells_wide() {
        for (width, height) in [(128usize, 16usize), (256, 32)] {
            assert_eq!(width, PIECES.len() * height);
            let rgba = vec![0u8; width * height * 4];
            assert!(split_edges(width, height, &rgba).is_some());
        }
    }
}
