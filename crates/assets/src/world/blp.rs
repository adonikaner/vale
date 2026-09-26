//! BLP2 — the texture format everything in 1.12 is stored in. Decodes to
//! straight RGBA8, the top mip alone or the whole chain.
//!
//! Ported from an earlier JavaScript implementation, which is known to render
//! this game's textures correctly. Writing one from scratch is a bad
//! trade: the format is simple but the DXT corner cases (1-bit alpha, the
//! `c0 <= c1` branch, DXT5's 3-bit index stream straddling byte boundaries) are
//! exactly where a fresh implementation produces *plausible* garbage — colours
//! that look almost right, on some blocks, in some textures.
//!
//! ```text
//! 0x00 u32  magic "BLP2"
//! 0x04 u32  type          0 JPEG (never used in WoW), 1 everything else
//! 0x08 u8   compression   1 palettized, 2 DXT, 3 raw BGRA
//! 0x09 u8   alphaDepth    0, 1, 4 or 8 bits
//! 0x0A u8   alphaType     used to tell DXT3 from DXT5
//! 0x0B u8   hasMips
//! 0x0C u32  width
//! 0x10 u32  height
//! 0x14 u32  mipOffsets[16]
//! 0x54 u32  mipSizes[16]
//! 0x94 ---  palette (256 x BGRA), present only for compression == 1
//! ```

use crate::AssetError;

/// Header offsets, named so the reads below are checkable against the layout in
/// the module docs rather than being bare numbers.
const OFF_TYPE: usize = 0x04;
const OFF_COMPRESSION: usize = 0x08;
const OFF_ALPHA_DEPTH: usize = 0x09;
const OFF_ALPHA_TYPE: usize = 0x0A;
const OFF_WIDTH: usize = 0x0C;
const OFF_HEIGHT: usize = 0x10;
const OFF_MIP_OFFSETS: usize = 0x14;
const OFF_MIP_SIZES: usize = 0x54;
/// The palette sits immediately after the two mip tables, and is always present
/// in the file even for DXT textures that never read it.
const OFF_PALETTE: usize = 0x94;
const HEADER_SIZE: usize = OFF_PALETTE + 256 * 4;

/// A decoded texture: tightly packed RGBA8.
#[derive(Debug, Clone)]
pub struct Blp {
    pub width: u32,
    pub height: u32,
    /// The top mip.
    pub rgba: Vec<u8>,
    /// Levels 1..n, smallest last, each half the previous in both axes and
    /// never below 1x1. Empty unless the file was read with [`decode_mipped`].
    pub mips: Vec<Vec<u8>>,
}

impl Blp {
    /// Bytes ready for `texImage2D` / `RGBA8`.
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }

    /// Dimensions of mip `level`, 0 being the top. Halving never goes below 1,
    /// so a 256x1 texture keeps its single row all the way down.
    pub fn level_size(&self, level: usize) -> (u32, u32) {
        (
            (self.width >> level).max(1),
            (self.height >> level).max(1),
        )
    }

    /// Every level, top first — the top mip and `mips` seen as one chain.
    pub fn levels(&self) -> impl Iterator<Item = &Vec<u8>> {
        std::iter::once(&self.rgba).chain(self.mips.iter())
    }
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Decode the largest mip level of a BLP2 texture.
///
/// The returned [`Blp::mips`] is empty: use [`decode_mipped`] when the texture
/// is going to be minified, which for anything drawn in the world it is.
pub fn decode(buf: &[u8]) -> Result<Blp, AssetError> {
    decode_levels(buf, 1)
}

/// Decode a BLP2 texture and every mip level the file carries.
///
/// **The mips are in the file already.** Blizzard authored them, so there is
/// nothing to generate and nothing to get wrong: level `n` is
/// `(width >> n).max(1)` by `(height >> n).max(1)`, and its bytes sit at
/// `mipOffsets[n]`.
///
/// Not using them is what makes the ground and the distant foliage shimmer: a
/// ground texture repeats eight times across a 33-yard chunk, so at any real
/// viewing angle one screen pixel covers many texels and sampling only the top
/// mip aliases them into a moire pattern that crawls as the camera moves.
pub fn decode_mipped(buf: &[u8]) -> Result<Blp, AssetError> {
    decode_levels(buf, MAX_MIPS)
}

/// The mip tables hold 16 entries, so that is the ceiling regardless — and 16
/// levels covers a 32768-pixel texture, which is well past the 4096 the header
/// check allows.
const MAX_MIPS: usize = 16;

fn decode_levels(buf: &[u8], want: usize) -> Result<Blp, AssetError> {
    let bad = |d: String| AssetError::malformed("BLP", d);

    if buf.len() < HEADER_SIZE {
        return Err(bad(format!("file is {} bytes, too small", buf.len())));
    }
    if &buf[0..4] != b"BLP2" {
        return Err(bad(format!("magic is {:?}, not BLP2", &buf[0..4])));
    }
    // Type 0 is JPEG-compressed BLP1-era content. It does not appear in 1.12
    // game data, and guessing at it would be worse than saying so.
    let kind = u32_at(buf, OFF_TYPE);
    if kind != 1 {
        return Err(bad(format!("type {kind} (JPEG) is not supported")));
    }

    let compression = buf[OFF_COMPRESSION];
    let alpha_depth = buf[OFF_ALPHA_DEPTH];
    let alpha_type = buf[OFF_ALPHA_TYPE];
    let width = u32_at(buf, OFF_WIDTH);
    let height = u32_at(buf, OFF_HEIGHT);

    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(bad(format!("implausible dimensions {width}x{height}")));
    }

    let rgba = decode_level(buf, 0, width, height, compression, alpha_depth, alpha_type)
        .map_err(bad)?;

    // Levels below the top are a bonus, not a requirement: a file that stops
    // early, or whose tail was lost to a patch, keeps the levels it did give and
    // is drawn with them. Stopping at the first gap rather than skipping it is
    // deliberate — a mip chain has to be contiguous from the top or the sampler
    // is reading level n's bytes as level n+1's.
    let mut mips = Vec::new();
    for level in 1..want.min(MAX_MIPS) {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        match decode_level(buf, level, w, h, compression, alpha_depth, alpha_type) {
            Ok(data) => mips.push(data),
            Err(_) => break,
        }
        if w == 1 && h == 1 {
            break;
        }
    }

    Ok(Blp {
        width,
        height,
        rgba,
        mips,
    })
}

/// One mip level as RGBA8, `width * height * 4` bytes.
fn decode_level(
    buf: &[u8],
    level: usize,
    width: u32,
    height: u32,
    compression: u8,
    alpha_depth: u8,
    alpha_type: u8,
) -> Result<Vec<u8>, String> {
    let offset = u32_at(buf, OFF_MIP_OFFSETS + level * 4) as usize;
    let size = u32_at(buf, OFF_MIP_SIZES + level * 4) as usize;
    if offset == 0 || size == 0 {
        return Err(format!("mip {level} is not present"));
    }
    let end = offset
        .checked_add(size)
        .ok_or_else(|| format!("mip {level} overflows"))?;
    if end > buf.len() {
        return Err(format!(
            "mip {level} runs to {end} but the file is {}",
            buf.len()
        ));
    }
    let data = &buf[offset..end];

    let pixels = (width as usize) * (height as usize);
    let mut rgba = vec![0u8; pixels * 4];

    match compression {
        // Raw BGRA — a straight channel swap.
        3 => {
            if data.len() < pixels * 4 {
                return Err(format!("raw BGRA mip {level} is short"));
            }
            for i in 0..pixels {
                rgba[i * 4] = data[i * 4 + 2];
                rgba[i * 4 + 1] = data[i * 4 + 1];
                rgba[i * 4 + 2] = data[i * 4];
                rgba[i * 4 + 3] = data[i * 4 + 3];
            }
        }

        // 256-colour palette: one index byte per pixel, then a packed alpha
        // plane whose width depends on `alphaDepth`.
        1 => decode_palettized(buf, data, pixels, alpha_depth, &mut rgba)
            .map_err(|_| format!("palettized mip {level} is short"))?,

        // DXT. `alphaDepth <= 1` is DXT1 (with or without its 1-bit alpha);
        // above that, `alphaType == 7` distinguishes DXT5 from DXT3.
        2 => {
            let mode = if alpha_depth > 1 {
                if alpha_type == 7 {
                    Dxt::Five
                } else {
                    Dxt::Three
                }
            } else {
                Dxt::One
            };
            decode_dxt(data, width as usize, height as usize, mode, alpha_depth, &mut rgba);
        }

        other => return Err(format!("unsupported compression {other}")),
    }

    Ok(rgba)
}

fn decode_palettized(
    buf: &[u8],
    data: &[u8],
    pixels: usize,
    alpha_depth: u8,
    rgba: &mut [u8],
) -> Result<(), AssetError> {
    if data.len() < pixels {
        return Err(AssetError::malformed("BLP", "palettized mip is short"));
    }
    // The alpha plane follows the index plane within the same mip.
    let alpha_base = pixels;

    for i in 0..pixels {
        let p = OFF_PALETTE + data[i] as usize * 4;
        rgba[i * 4] = buf[p + 2];
        rgba[i * 4 + 1] = buf[p + 1];
        rgba[i * 4 + 2] = buf[p];
        rgba[i * 4 + 3] = match alpha_depth {
            1 => {
                let byte = data.get(alpha_base + (i >> 3)).copied().unwrap_or(0xFF);
                if (byte >> (i & 7)) & 1 != 0 {
                    255
                } else {
                    0
                }
            }
            // A 4-bit nibble scaled by 17 maps 0..15 onto 0..255 exactly.
            4 => {
                let byte = data.get(alpha_base + (i >> 1)).copied().unwrap_or(0xFF);
                ((byte >> ((i & 1) * 4)) & 0x0F) * 17
            }
            8 => data.get(alpha_base + i).copied().unwrap_or(0xFF),
            _ => 255,
        };
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Dxt {
    One,
    Three,
    Five,
}

fn decode_dxt(
    data: &[u8],
    width: usize,
    height: usize,
    mode: Dxt,
    alpha_depth: u8,
    out: &mut [u8],
) {
    let block_bytes = if mode == Dxt::One { 8 } else { 16 };
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);

    let mut src = 0usize;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if src + block_bytes > data.len() {
                // A truncated tail leaves the remaining blocks transparent
                // black rather than panicking — these are 20-year-old files
                // from patched archives.
                return;
            }
            // DXT3/5 put their 8 alpha bytes first; the colour block follows.
            let co = if mode == Dxt::One { src } else { src + 8 };
            let c0 = data[co] as u16 | ((data[co + 1] as u16) << 8);
            let c1 = data[co + 2] as u16 | ((data[co + 3] as u16) << 8);
            let bits = u32::from_le_bytes([data[co + 4], data[co + 5], data[co + 6], data[co + 7]]);

            let mut c = [[0u8; 4]; 4];
            expand565(c0, &mut c[0]);
            expand565(c1, &mut c[1]);

            // In DXT1, `c0 <= c1` switches the block to three colours plus one
            // transparent — but only when the texture declares 1-bit alpha.
            // Otherwise index 3 is opaque black, which is a real colour.
            let punch_through = mode == Dxt::One && alpha_depth >= 1 && c0 <= c1;
            if mode != Dxt::One || c0 > c1 {
                for k in 0..3 {
                    c[2][k] = ((2 * c[0][k] as u16 + c[1][k] as u16 + 1) / 3) as u8;
                    c[3][k] = ((c[0][k] as u16 + 2 * c[1][k] as u16 + 1) / 3) as u8;
                }
                c[2][3] = 255;
                c[3][3] = 255;
            } else {
                for k in 0..3 {
                    c[2][k] = ((c[0][k] as u16 + c[1][k] as u16) >> 1) as u8;
                    c[3][k] = 0;
                }
                c[2][3] = 255;
                c[3][3] = if punch_through { 0 } else { 255 };
            }

            for py in 0..4 {
                let y = by * 4 + py;
                if y >= height {
                    break;
                }
                for px in 0..4 {
                    let x = bx * 4 + px;
                    if x >= width {
                        continue;
                    }
                    let texel = py * 4 + px;
                    let idx = ((bits >> (texel * 2)) & 3) as usize;
                    let o = (y * width + x) * 4;
                    out[o] = c[idx][0];
                    out[o + 1] = c[idx][1];
                    out[o + 2] = c[idx][2];
                    out[o + 3] = match mode {
                        Dxt::One => c[idx][3],
                        Dxt::Three => ((data[src + (texel >> 1)] >> ((texel & 1) * 4)) & 0x0F) * 17,
                        Dxt::Five => dxt5_alpha(data, src, texel),
                    };
                }
            }
            src += block_bytes;
        }
    }
}

fn expand565(v: u16, dst: &mut [u8; 4]) {
    dst[0] = (((v >> 11) & 31) as u32 * 255 / 31) as u8;
    dst[1] = (((v >> 5) & 63) as u32 * 255 / 63) as u8;
    dst[2] = ((v & 31) as u32 * 255 / 31) as u8;
    dst[3] = 255;
}

/// DXT5 alpha: two endpoints, then a 48-bit stream of 3-bit indices.
///
/// The indices are *not* byte-aligned — texel 2 straddles bytes 2 and 3 — so
/// this reads a 16-bit window and shifts, which is what makes it worth copying
/// rather than rewriting.
fn dxt5_alpha(data: &[u8], src: usize, texel: usize) -> u8 {
    let a0 = data[src] as u16;
    let a1 = data[src + 1] as u16;

    let bit_pos = texel * 3;
    let byte = src + 2 + (bit_pos >> 3);
    let lo = data.get(byte).copied().unwrap_or(0) as u16;
    let hi = data.get(byte + 1).copied().unwrap_or(0) as u16;
    let code = ((lo | (hi << 8)) >> (bit_pos & 7)) & 7;

    let v = match code {
        0 => a0,
        1 => a1,
        _ if a0 > a1 => ((8 - code) * a0 + (code - 1) * a1 + 3) / 7,
        6 => 0,
        7 => 255,
        _ => ((6 - code) * a0 + (code - 1) * a1 + 2) / 5,
    };
    v as u8
}

// ---------------------------------------------------------------- the writer

/// **Write a BLP2 the reference client reads.** DXT1, one level, no alpha.
///
/// ## Why that shape and not another
///
/// It is the shape the game's own minimap tiles are in, measured rather than
/// chosen: `textures\Minimap\ea283abc…blp` — Azeroth's tile 32, 48 — is
/// **BLP2, type 1, compression 2, alphaDepth 0, alphaType 0, hasMips 0, 256x256,
/// one mip of 32,768 bytes**, which at 0.5 bytes a pixel is DXT1 and nothing
/// else. A 256x256 raw BGRA file would decode here and be eight times the size,
/// and nothing in the archives looks like one.
///
/// Only the writer this project actually needs exists: there is no palettized
/// encoder and no DXT5, because nothing writes either. The decoder above reads
/// all three because the game ships all three.
///
/// ## The encoder is a bounding box, and that is a decision
///
/// Each 4x4 block takes the per-channel minimum and maximum of its sixteen
/// texels as the two endpoints and picks the nearest of the four interpolated
/// colours for each. That is the simplest DXT1 encoder that is not wrong, and it
/// is a poor one for photographic art — a block straddling two very different
/// colours gets a ramp between them rather than the two colours. For what this
/// writes it is the right trade: a minimap tile is broad regions of ground
/// colour, which is the case a bounding box handles exactly.
///
/// **`c0 > c1` is enforced**, which is what selects DXT1's opaque four-colour
/// mode. The other branch reserves index 3 for transparent black, so a block
/// that landed in it would punch holes in the ground.
pub fn encode_dxt1(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, AssetError> {
    let bad = |d: String| AssetError::malformed("BLP", d);
    if width == 0 || height == 0 || width % 4 != 0 || height % 4 != 0 {
        return Err(bad(format!("{width}x{height} is not a whole number of 4x4 blocks")));
    }
    let want = (width as usize) * (height as usize) * 4;
    if rgba.len() != want {
        return Err(bad(format!("{} bytes of pixels, expected {want}", rgba.len())));
    }

    let mut blocks = Vec::with_capacity(want / 8);
    for by in (0..height as usize).step_by(4) {
        for bx in (0..width as usize).step_by(4) {
            blocks.extend_from_slice(&encode_block(rgba, width as usize, bx, by));
        }
    }

    let mut out = vec![0u8; HEADER_SIZE];
    out[0..4].copy_from_slice(b"BLP2");
    out[OFF_TYPE..OFF_TYPE + 4].copy_from_slice(&1u32.to_le_bytes());
    out[OFF_COMPRESSION] = 2;
    out[OFF_ALPHA_DEPTH] = 0;
    out[OFF_ALPHA_TYPE] = 0;
    // hasMips stays 0, and only mip 0 is filled in, which is what the shipped
    // minimap tiles do.
    out[OFF_WIDTH..OFF_WIDTH + 4].copy_from_slice(&width.to_le_bytes());
    out[OFF_HEIGHT..OFF_HEIGHT + 4].copy_from_slice(&height.to_le_bytes());
    out[OFF_MIP_OFFSETS..OFF_MIP_OFFSETS + 4].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    out[OFF_MIP_SIZES..OFF_MIP_SIZES + 4].copy_from_slice(&(blocks.len() as u32).to_le_bytes());
    out.extend_from_slice(&blocks);
    Ok(out)
}

/// One 4x4 block: two RGB565 endpoints and sixteen two-bit indices.
fn encode_block(rgba: &[u8], width: usize, bx: usize, by: usize) -> [u8; 8] {
    let mut texels = [[0u8; 3]; 16];
    for y in 0..4 {
        for x in 0..4 {
            let at = ((by + y) * width + bx + x) * 4;
            texels[y * 4 + x] = [rgba[at], rgba[at + 1], rgba[at + 2]];
        }
    }

    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for t in &texels {
        for c in 0..3 {
            lo[c] = lo[c].min(t[c]);
            hi[c] = hi[c].max(t[c]);
        }
    }

    let (mut c0, mut c1) = (to_565(hi), to_565(lo));
    // Four-colour mode is the one without a transparent index — see the note on
    // `encode_dxt1`. Equal endpoints are a flat block and stay as they are:
    // every index is 0, which is `c0`, which is the colour.
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }

    let palette = dxt1_palette(c0, c1);
    let mut indices = 0u32;
    for (i, t) in texels.iter().enumerate() {
        let mut best = (0usize, u32::MAX);
        for (n, p) in palette.iter().enumerate() {
            let d = (0..3)
                .map(|c| {
                    let delta = i32::from(t[c]) - i32::from(p[c]);
                    (delta * delta) as u32
                })
                .sum::<u32>();
            if d < best.1 {
                best = (n, d);
            }
        }
        indices |= (best.0 as u32) << (i * 2);
    }

    let mut block = [0u8; 8];
    block[0..2].copy_from_slice(&c0.to_le_bytes());
    block[2..4].copy_from_slice(&c1.to_le_bytes());
    block[4..8].copy_from_slice(&indices.to_le_bytes());
    block
}

fn to_565(c: [u8; 3]) -> u16 {
    (u16::from(c[0] >> 3) << 11) | (u16::from(c[1] >> 2) << 5) | u16::from(c[2] >> 3)
}

fn from_565(v: u16) -> [u8; 3] {
    let (r, g, b) = ((v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F);
    [
        ((r << 3) | (r >> 2)) as u8,
        ((g << 2) | (g >> 4)) as u8,
        ((b << 3) | (b >> 2)) as u8,
    ]
}

/// The four colours a `c0 > c1` block interpolates: the two endpoints and two
/// thirds between them. The same arithmetic [`decode_dxt`] does on the way in.
fn dxt1_palette(c0: u16, c1: u16) -> [[u8; 3]; 4] {
    let (a, b) = (from_565(c0), from_565(c1));
    let mix = |num: u32, den: u32| {
        let mut out = [0u8; 3];
        for c in 0..3 {
            out[c] = ((u32::from(a[c]) * num + u32::from(b[c]) * (den - num)) / den) as u8;
        }
        out
    };
    [a, b, mix(2, 3), mix(1, 3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal well-formed BLP2 header with room for the palette.
    fn header(compression: u8, alpha_depth: u8, alpha_type: u8, w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0u8; HEADER_SIZE];
        b[0..4].copy_from_slice(b"BLP2");
        b[OFF_TYPE..OFF_TYPE + 4].copy_from_slice(&1u32.to_le_bytes());
        b[OFF_COMPRESSION] = compression;
        b[OFF_ALPHA_DEPTH] = alpha_depth;
        b[OFF_ALPHA_TYPE] = alpha_type;
        b[OFF_WIDTH..OFF_WIDTH + 4].copy_from_slice(&w.to_le_bytes());
        b[OFF_HEIGHT..OFF_HEIGHT + 4].copy_from_slice(&h.to_le_bytes());
        b
    }

    fn with_mip(mut header: Vec<u8>, mip: &[u8]) -> Vec<u8> {
        let offset = header.len() as u32;
        header[OFF_MIP_OFFSETS..OFF_MIP_OFFSETS + 4].copy_from_slice(&offset.to_le_bytes());
        header[OFF_MIP_SIZES..OFF_MIP_SIZES + 4]
            .copy_from_slice(&(mip.len() as u32).to_le_bytes());
        header.extend_from_slice(mip);
        header
    }

    /// Append several levels, filling both mip tables — the layout a real BLP
    /// has and the one the chain walker reads.
    fn with_mips(mut header: Vec<u8>, mips: &[&[u8]]) -> Vec<u8> {
        for (level, mip) in mips.iter().enumerate() {
            let offset = header.len() as u32;
            let o = OFF_MIP_OFFSETS + level * 4;
            let s = OFF_MIP_SIZES + level * 4;
            header[o..o + 4].copy_from_slice(&offset.to_le_bytes());
            header[s..s + 4].copy_from_slice(&(mip.len() as u32).to_le_bytes());
            header.extend_from_slice(mip);
        }
        header
    }

    /// The mips are Blizzard's own, so the only thing that can go wrong is
    /// reading them at the wrong offset or the wrong size — which yields
    /// *plausible* smaller images rather than an error. Two levels of a 2x1 raw
    /// BGRA texture, distinguishable by colour.
    #[test]
    fn the_mip_chain_is_read_from_the_files_own_tables() {
        let top: &[u8] = &[0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF]; // 2x1 red
        let next: &[u8] = &[0xFF, 0x00, 0x00, 0xFF]; // 1x1 blue
        let file = with_mips(header(3, 8, 0, 2, 1), &[top, next]);

        // The default entry point stays top-mip-only, so the CLI's whole-archive
        // surveys do not silently start decoding four times the data.
        let one = decode(&file).expect("decoded");
        assert!(one.mips.is_empty());
        assert_eq!(one.rgba.len(), 2 * 4);

        let all = decode_mipped(&file).expect("decoded");
        assert_eq!(all.mips.len(), 1, "2x1 halves once and stops at 1x1");
        assert_eq!(all.level_size(0), (2, 1));
        assert_eq!(all.level_size(1), (1, 1));
        assert_eq!(&all.rgba[0..3], &[0xFF, 0x00, 0x00], "top mip is red");
        assert_eq!(&all.mips[0][0..3], &[0x00, 0x00, 0xFF], "level 1 is blue");
        // Every level has to be exactly its own size, or the upload reads one
        // level's bytes as the next one's.
        for (level, data) in all.levels().enumerate() {
            let (w, h) = all.level_size(level);
            assert_eq!(data.len(), (w * h * 4) as usize, "level {level}");
        }
    }

    /// A file whose chain stops early keeps the levels it did give. These are
    /// twenty-year-old files out of patched archives, and a partial chain is
    /// still better than none — but it must stop at the gap rather than skip
    /// it, or the sampler reads one level's bytes as another's.
    #[test]
    fn a_chain_that_stops_early_keeps_what_it_had() {
        let top: &[u8] = &[0u8; 4 * 4 * 4]; // 4x4
        let file = with_mips(header(3, 8, 0, 4, 4), &[top]);
        let all = decode_mipped(&file).expect("decoded");
        assert!(all.mips.is_empty(), "no level 1 in the tables");
        assert_eq!(all.rgba.len(), 4 * 4 * 4);
    }

    #[test]
    fn raw_bgra_is_swapped_to_rgba() {
        let file = with_mip(header(3, 8, 0, 1, 1), &[0x11, 0x22, 0x33, 0x44]);
        let blp = decode(&file).expect("decoded");
        assert_eq!(blp.rgba, vec![0x33, 0x22, 0x11, 0x44]);
    }

    #[test]
    fn a_palette_entry_is_read_bgra() {
        let mut h = header(1, 0, 0, 2, 1);
        // Palette slot 5 = BGRA(0x10, 0x20, 0x30, 0)  ->  RGB(0x30, 0x20, 0x10)
        let p = OFF_PALETTE + 5 * 4;
        h[p] = 0x10;
        h[p + 1] = 0x20;
        h[p + 2] = 0x30;
        let file = with_mip(h, &[5, 5]);
        let blp = decode(&file).expect("decoded");
        assert_eq!(&blp.rgba[0..4], &[0x30, 0x20, 0x10, 255]);
        assert_eq!(&blp.rgba[4..8], &[0x30, 0x20, 0x10, 255]);
    }

    #[test]
    fn four_bit_alpha_scales_a_nibble_to_full_range() {
        let file = with_mip(header(1, 4, 0, 2, 1), &[0, 0, 0xF0]);
        let blp = decode(&file).expect("decoded");
        // Low nibble first: pixel 0 gets 0, pixel 1 gets 0xF * 17 = 255.
        assert_eq!(blp.rgba[3], 0);
        assert_eq!(blp.rgba[7], 255);
    }

    #[test]
    fn dxt1_expands_both_endpoints_and_interpolates() {
        // c0 = white (0xFFFF), c1 = black (0x0000), c0 > c1 so it is the
        // four-colour branch. Indices 0,1,2,3 on the first row.
        let mut block = vec![0u8; 8];
        block[0] = 0xFF;
        block[1] = 0xFF;
        block[2] = 0x00;
        block[3] = 0x00;
        // Texels 0..3 = indices 0,1,2,3 -> bits 0b11_10_01_00
        block[4] = 0b1110_0100;
        let file = with_mip(header(2, 0, 0, 4, 4), &block);
        let blp = decode(&file).expect("decoded");

        assert_eq!(&blp.rgba[0..4], &[255, 255, 255, 255], "index 0 is c0");
        assert_eq!(&blp.rgba[4..8], &[0, 0, 0, 255], "index 1 is c1");
        // index 2 = (2*c0 + c1)/3 = 170
        assert_eq!(blp.rgba[8], 170);
        // index 3 = (c0 + 2*c1)/3 = 85
        assert_eq!(blp.rgba[12], 85);
    }

    #[test]
    fn dxt1_punch_through_alpha_needs_both_the_ordering_and_the_depth() {
        // c0 <= c1 puts the block in the three-colour branch, where index 3 is
        // transparent — but only when the file declares 1-bit alpha. Getting
        // this wrong makes black pixels vanish, or holes turn black.
        let mut block = vec![0u8; 8];
        block[2] = 0xFF;
        block[3] = 0xFF; // c1 = white, c0 = black, so c0 < c1
        block[4] = 0b0000_0011; // texel 0 = index 3

        let transparent = decode(&with_mip(header(2, 1, 0, 4, 4), &block)).expect("decoded");
        assert_eq!(transparent.rgba[3], 0, "1-bit alpha => index 3 transparent");

        let opaque = decode(&with_mip(header(2, 0, 0, 4, 4), &block)).expect("decoded");
        assert_eq!(opaque.rgba[3], 255, "no alpha => index 3 is opaque black");
    }

    #[test]
    fn dxt5_alpha_indices_straddle_byte_boundaries() {
        let mut block = vec![0u8; 16];
        block[0] = 255; // a0
        block[1] = 0; // a1
        // Every index = 1, i.e. a1 = 0, for all 16 texels: 3 bits x 16 = 48 bits
        // of 0b001 repeating = 0x49 0x92 0x24 repeated.
        for (i, b) in [0x49u8, 0x92, 0x24, 0x49, 0x92, 0x24].iter().enumerate() {
            block[2 + i] = *b;
        }
        block[8] = 0xFF;
        block[9] = 0xFF; // c0 = white
        let file = with_mip(header(2, 8, 7, 4, 4), &block);
        let blp = decode(&file).expect("decoded");
        for texel in 0..16 {
            assert_eq!(blp.rgba[texel * 4 + 3], 0, "texel {texel}");
        }
    }

    #[test]
    fn a_truncated_dxt_tail_leaves_the_rest_blank_rather_than_panicking() {
        // 8x8 needs four blocks; supply one and a half.
        let file = with_mip(header(2, 0, 0, 8, 8), &vec![0xAAu8; 12]);
        let blp = decode(&file).expect("decoded");
        assert_eq!(blp.rgba.len(), 8 * 8 * 4);
    }

    // ------------------------------------------------------------ the writer

    /// **The encoder's only real check: what it writes, this file's own decoder
    /// reads back.** A DXT encoder that is subtly wrong produces plausible
    /// colours, which is exactly the failure mode the module comment opens by
    /// warning about — so the assertion is on the pixels and not on the header.
    #[test]
    fn a_flat_block_survives_the_round_trip_exactly() {
        // One colour the 565 quantisation can represent without loss.
        let rgba: Vec<u8> = std::iter::repeat([0x18u8, 0x44, 0x08, 255])
            .take(8 * 8)
            .flatten()
            .collect();
        let file = encode_dxt1(&rgba, 8, 8).expect("encoded");
        let back = decode(&file).expect("decoded");
        assert_eq!(back.width, 8);
        assert_eq!(back.height, 8);
        for texel in 0..8 * 8 {
            assert_eq!(&back.rgba[texel * 4..texel * 4 + 3], &[0x18, 0x44, 0x08], "texel {texel}");
            assert_eq!(back.rgba[texel * 4 + 3], 255, "texel {texel} alpha");
        }
    }

    /// …and the file it writes is the shape the shipped minimap tiles are in,
    /// which is what says the real client will read it: BLP2, type 1,
    /// compression 2, no alpha, no mips, one level at half a byte a pixel.
    #[test]
    fn the_file_is_the_shape_the_shipped_minimaps_are() {
        let rgba = vec![0u8; 256 * 256 * 4];
        let file = encode_dxt1(&rgba, 256, 256).expect("encoded");
        assert_eq!(&file[0..4], b"BLP2");
        assert_eq!(u32_at(&file, OFF_TYPE), 1);
        assert_eq!(file[OFF_COMPRESSION], 2);
        assert_eq!(file[OFF_ALPHA_DEPTH], 0);
        assert_eq!(file[OFF_ALPHA_TYPE], 0);
        assert_eq!(file[OFF_MIP_OFFSETS + 3 + 1], 0, "only one mip is declared");
        assert_eq!(u32_at(&file, OFF_WIDTH), 256);
        assert_eq!(u32_at(&file, OFF_HEIGHT), 256);
        // 4096 blocks of eight bytes, which is the 32,768 the shipped file has.
        assert_eq!(u32_at(&file, OFF_MIP_SIZES), 32768);
        assert_eq!(file.len(), HEADER_SIZE + 32768);
    }

    /// **A two-colour block keeps both colours**, which is the case a bounding
    /// box is chosen for and the one where an encoder that averaged would be
    /// visibly wrong: half the block is ground and half is water.
    #[test]
    fn a_two_colour_block_keeps_both_colours() {
        let dark = [0x10u8, 0x20, 0x40];
        let light = [0xC0u8, 0xB0, 0x50];
        let mut rgba = Vec::new();
        for y in 0..4 {
            for _ in 0..4 {
                let c = if y < 2 { dark } else { light };
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let back = decode(&encode_dxt1(&rgba, 4, 4).unwrap()).unwrap();
        for texel in 0..16 {
            let want = if texel < 8 { dark } else { light };
            let got = &back.rgba[texel * 4..texel * 4 + 3];
            for c in 0..3 {
                let delta = i32::from(got[c]) - i32::from(want[c]);
                assert!(delta.abs() <= 8, "texel {texel} channel {c}: {got:?} vs {want:?}");
            }
        }
    }

    /// A ramp comes back as a ramp rather than as two colours, which is what
    /// the two interpolated indices are for.
    #[test]
    fn a_gradient_keeps_its_middle() {
        let mut rgba = Vec::new();
        for i in 0..16u8 {
            let v = i * 16;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        let back = decode(&encode_dxt1(&rgba, 4, 4).unwrap()).unwrap();
        let greys: Vec<u8> = (0..16).map(|t| back.rgba[t * 4]).collect();
        assert!(greys[0] < greys[15], "the ramp is inverted: {greys:?}");
        let distinct: std::collections::BTreeSet<u8> = greys.iter().copied().collect();
        assert!(distinct.len() >= 3, "only {} levels: {greys:?}", distinct.len());
    }

    /// Sizes that are not whole blocks are refused rather than padded: a
    /// padded texture is a picture with a margin nobody asked for.
    #[test]
    fn a_size_that_is_not_whole_blocks_is_refused() {
        assert!(encode_dxt1(&vec![0u8; 6 * 6 * 4], 6, 6).is_err());
        assert!(encode_dxt1(&[], 0, 0).is_err());
        // …and so is a buffer that does not match the size it claims.
        assert!(encode_dxt1(&vec![0u8; 10], 4, 4).is_err());
    }

    #[test]
    fn a_non_blp_file_is_rejected_by_name() {
        let mut b = vec![0u8; HEADER_SIZE];
        b[0..4].copy_from_slice(b"BLP1");
        assert!(decode(&b).is_err());
    }
}
