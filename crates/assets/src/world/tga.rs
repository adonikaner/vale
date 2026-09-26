//! **TGA, the other texture format the interface reads.**
//!
//! Every texture in the archives is BLP ([`super::blp`]). An addon's art is
//! not: of the 46 images in a real `pfUI\img\`, 45 are `.tga`, and the client
//! resolves a texture path with no extension by trying `.blp` and then `.tga`.
//! This is the reader for the second.
//!
//! The format is the 18-byte Truevision header, an optional id field, and the
//! pixels bottom row first unless bit 5 of the descriptor says top. Four of its
//! image types are read: 2 (true colour), 3 (greyscale), 10 (true colour,
//! run-length packed) and 11 (greyscale, run-length packed), at 8, 24 or 32
//! bits a pixel. Pixels are stored BGR(A); the output is RGBA, top row first,
//! so it goes where a decoded BLP goes. A colour-mapped image is refused: no
//! addon art measured uses one.
//!
//! ```text
//! 00 00 0A 00 00 00 00 00 00 00 00 00 08 00 08 00 20 08
//! id 0, no map, type 10, map spec 0, origin 0,0, 8 x 8, 32 bpp, descriptor 8
//! ```
//!
//! That is `pfUI\img\bar.tga`'s head: run-length packed, 32-bit, 8 alpha bits,
//! bottom-left origin.

use crate::AssetError;

/// A decoded image: RGBA8, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tga {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const HEADER: usize = 18;

/// Decode one file. A truncated pixel stream is filled with zero bytes rather
/// than refused, on the terms every parser here has: a damaged file costs its
/// own contents and nothing else.
pub fn decode(buf: &[u8]) -> Result<Tga, AssetError> {
    if buf.len() < HEADER {
        return Err(AssetError::malformed("TGA", "shorter than its header"));
    }
    let id_len = usize::from(buf[0]);
    let colour_map = buf[1];
    let image_type = buf[2];
    let width = u32::from(u16::from_le_bytes([buf[12], buf[13]]));
    let height = u32::from(u16::from_le_bytes([buf[14], buf[15]]));
    let bpp = buf[16];
    let descriptor = buf[17];
    if colour_map != 0 {
        return Err(AssetError::malformed("TGA", "colour-mapped images are not read"));
    }
    let (packed, grey) = match image_type {
        2 => (false, false),
        3 => (false, true),
        10 => (true, false),
        11 => (true, true),
        other => {
            return Err(AssetError::malformed("TGA", format!("image type {other}")));
        }
    };
    let bytes_per_pixel = match (grey, bpp) {
        (true, 8) => 1,
        (false, 24) => 3,
        (false, 32) => 4,
        _ => return Err(AssetError::malformed("TGA", format!("{bpp} bits a pixel"))),
    };
    if width == 0 || height == 0 {
        return Err(AssetError::malformed("TGA", "empty image"));
    }
    let count = (width as usize) * (height as usize);
    let mut pixels: Vec<u8> = Vec::with_capacity(count * bytes_per_pixel);
    let mut data = &buf[(HEADER + id_len).min(buf.len())..];
    if packed {
        // A packet is one count byte: high bit set means the next pixel repeats
        // `count + 1` times; clear means `count + 1` literal pixels follow.
        while pixels.len() < count * bytes_per_pixel && !data.is_empty() {
            let header = data[0];
            data = &data[1..];
            let run = usize::from(header & 0x7f) + 1;
            if header & 0x80 != 0 {
                if data.len() < bytes_per_pixel {
                    break;
                }
                let pixel = &data[..bytes_per_pixel];
                for _ in 0..run {
                    pixels.extend_from_slice(pixel);
                }
                data = &data[bytes_per_pixel..];
            } else {
                let take = (run * bytes_per_pixel).min(data.len());
                pixels.extend_from_slice(&data[..take]);
                data = &data[take..];
            }
        }
    } else {
        let take = (count * bytes_per_pixel).min(data.len());
        pixels.extend_from_slice(&data[..take]);
    }
    pixels.resize(count * bytes_per_pixel, 0);

    let mut rgba = vec![0u8; count * 4];
    let top_first = descriptor & 0x20 != 0;
    for row in 0..height as usize {
        let source_row = if top_first { row } else { height as usize - 1 - row };
        for column in 0..width as usize {
            let at = (source_row * width as usize + column) * bytes_per_pixel;
            let out = (row * width as usize + column) * 4;
            let pixel = &pixels[at..at + bytes_per_pixel];
            rgba[out..out + 4].copy_from_slice(&match bytes_per_pixel {
                1 => [pixel[0], pixel[0], pixel[0], 255],
                3 => [pixel[2], pixel[1], pixel[0], 255],
                _ => [pixel[2], pixel[1], pixel[0], pixel[3]],
            });
        }
    }
    Ok(Tga { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(image_type: u8, width: u16, height: u16, bpp: u8, descriptor: u8) -> Vec<u8> {
        let mut h = vec![0u8; HEADER];
        h[2] = image_type;
        h[12..14].copy_from_slice(&width.to_le_bytes());
        h[14..16].copy_from_slice(&height.to_le_bytes());
        h[16] = bpp;
        h[17] = descriptor;
        h
    }

    /// Two rows, bottom-left origin: the file's first row is the picture's
    /// last, and BGRA becomes RGBA.
    #[test]
    fn an_unpacked_image_is_flipped_and_reordered() {
        let mut file = header(2, 1, 2, 32, 0x08);
        file.extend_from_slice(&[1, 2, 3, 4]); // bottom row: B=1 G=2 R=3 A=4
        file.extend_from_slice(&[5, 6, 7, 8]); // top row
        let tga = decode(&file).unwrap();
        assert_eq!((tga.width, tga.height), (1, 2));
        assert_eq!(tga.rgba, [7, 6, 5, 8, 3, 2, 1, 4]);
        // …and with the top-left bit set, in file order.
        file[17] = 0x28;
        assert_eq!(decode(&file).unwrap().rgba, [3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn a_run_length_image_expands_runs_and_literals() {
        let mut file = header(10, 4, 1, 24, 0x20);
        file.extend_from_slice(&[0x81, 10, 20, 30]); // two of BGR 10,20,30
        file.extend_from_slice(&[0x01, 1, 2, 3, 4, 5, 6]); // two literals
        let tga = decode(&file).unwrap();
        assert_eq!(
            tga.rgba,
            [30, 20, 10, 255, 30, 20, 10, 255, 3, 2, 1, 255, 6, 5, 4, 255]
        );
    }

    #[test]
    fn greyscale_and_truncation_and_refusals() {
        let mut file = header(3, 2, 1, 8, 0x20);
        file.extend_from_slice(&[9]); // one of two pixels present
        let tga = decode(&file).unwrap();
        assert_eq!(tga.rgba, [9, 9, 9, 255, 0, 0, 0, 255], "the missing pixel is zero bytes: black");
        assert!(decode(&header(1, 1, 1, 8, 0)).is_err(), "colour-mapped");
        assert!(decode(&header(2, 1, 1, 16, 0)).is_err(), "16-bit");
        assert!(decode(&[0; 10]).is_err(), "short");
        let mut with_id = header(2, 1, 1, 24, 0x20);
        with_id[0] = 2;
        with_id.extend_from_slice(&[0xaa, 0xbb, 1, 2, 3]);
        assert_eq!(decode(&with_id).unwrap().rgba, [3, 2, 1, 255], "the id field is skipped");
    }
}
