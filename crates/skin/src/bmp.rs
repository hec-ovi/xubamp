//! Hand-rolled BMP decoder for classic Winamp skin bitmaps.
//!
//! Classic `.wsz` skins ship BMPs at 1/4/8/24 (occasionally 32) bits per pixel,
//! normally with a 40-byte `BITMAPINFOHEADER`, sometimes stored bottom-up. Common
//! image libraries (and stb_image) mishandle the 1-bit BMPs that some skins use, so
//! we decode ourselves and keep it allocation-tight: exactly one output buffer,
//! sized once, no per-pixel allocation.
//!
//! Output is always top-down `RGBA8888`, 4 bytes per pixel, alpha = 255. Colour-key
//! and region-based transparency belong to higher layers, not here.

/// A decoded image: top-down rows, `RGBA8888`, 4 bytes per pixel.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, top-down.
    pub rgba: Vec<u8>,
}

impl core::fmt::Debug for Image {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .finish()
    }
}

/// Why a BMP failed to decode. Skins are a long tail, so callers fall back to the
/// default skin for a missing or corrupt file rather than aborting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpError {
    TooShort,
    BadMagic,
    UnsupportedHeader(u32),
    UnsupportedBpp(u16),
    UnsupportedCompression(u32),
    BadDimensions,
    Truncated,
}

const FILE_HEADER: usize = 14;
/// Sanity cap before allocating. Classic skin sheets are a few hundred px per side;
/// this rejects a corrupt header claiming a huge canvas.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

#[inline]
fn rd_u16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
#[inline]
fn rd_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
#[inline]
fn rd_i32(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// Decode a Windows BMP (`BITMAPINFOHEADER` or the V4/V5 supersets) into RGBA.
pub fn decode(data: &[u8]) -> Result<Image, BmpError> {
    if data.len() < FILE_HEADER + 40 {
        return Err(BmpError::TooShort);
    }
    if &data[0..2] != b"BM" {
        return Err(BmpError::BadMagic);
    }

    let mut off_bits = rd_u32(data, 10) as usize;
    let dib_size = rd_u32(data, 14);
    // V4 (108) and V5 (124) start with the same 40-byte layout we read below.
    if dib_size < 40 {
        return Err(BmpError::UnsupportedHeader(dib_size));
    }
    if data.len() < FILE_HEADER + dib_size as usize {
        return Err(BmpError::Truncated);
    }

    let width_i = rd_i32(data, 18);
    let height_i = rd_i32(data, 22);
    let bpp = rd_u16(data, 28);
    let compression = rd_u32(data, 30);
    let mut clr_used = rd_u32(data, 46);

    // BI_RGB (0) only. BI_BITFIELDS (3) is accepted for 24/32-bit where the default
    // byte order applies; RLE / JPEG / PNG payloads we decline.
    if compression != 0 && compression != 3 {
        return Err(BmpError::UnsupportedCompression(compression));
    }
    if width_i <= 0 || height_i == 0 {
        return Err(BmpError::BadDimensions);
    }

    let width = width_i as u32;
    let top_down = height_i < 0;
    let height = height_i.unsigned_abs();

    let px = (width as u64)
        .checked_mul(height as u64)
        .ok_or(BmpError::BadDimensions)?;
    if px == 0 || px > MAX_PIXELS {
        return Err(BmpError::BadDimensions);
    }

    // Palette (indexed modes only) sits right after the DIB header.
    let palette_off = FILE_HEADER + dib_size as usize;
    let (palette, palette_len) = if bpp <= 8 {
        let max_entries = 1u32 << bpp;
        if clr_used == 0 || clr_used > max_entries {
            clr_used = max_entries;
        }
        let bytes = clr_used as usize * 4;
        if data.len() < palette_off + bytes {
            return Err(BmpError::Truncated);
        }
        (&data[palette_off..palette_off + bytes], clr_used as usize)
    } else {
        (&[][..], 0)
    };

    // Recover a missing/zero offBits: header + palette.
    if off_bits == 0 {
        off_bits = palette_off + palette_len * 4;
    }
    if off_bits > data.len() {
        return Err(BmpError::Truncated);
    }

    let row_bytes = (bpp as usize * width as usize).div_ceil(32) * 4;
    let need = off_bits
        .checked_add(
            row_bytes
                .checked_mul(height as usize)
                .ok_or(BmpError::BadDimensions)?,
        )
        .ok_or(BmpError::BadDimensions)?;
    if data.len() < need {
        return Err(BmpError::Truncated);
    }

    let row_w = width as usize;
    let mut rgba = vec![0u8; px as usize * 4];

    for y in 0..height as usize {
        let src_row = if top_down { y } else { height as usize - 1 - y };
        let base = off_bits + src_row * row_bytes;
        let row = &data[base..base + row_bytes];
        let dst = &mut rgba[y * row_w * 4..(y + 1) * row_w * 4];
        match bpp {
            1 | 4 | 8 => {
                for x in 0..row_w {
                    let idx = match bpp {
                        1 => (row[x >> 3] >> (7 - (x & 7))) & 1,
                        4 => {
                            let b = row[x >> 1];
                            if x & 1 == 0 {
                                b >> 4
                            } else {
                                b & 0x0f
                            }
                        }
                        _ => row[x],
                    } as usize;
                    // palette entries are BGRA; the 4th byte is reserved, ignored.
                    let p = idx.min(palette_len.saturating_sub(1)) * 4;
                    let o = x * 4;
                    dst[o] = palette[p + 2];
                    dst[o + 1] = palette[p + 1];
                    dst[o + 2] = palette[p];
                    dst[o + 3] = 255;
                }
            }
            24 => {
                for x in 0..row_w {
                    let s = x * 3;
                    let o = x * 4;
                    dst[o] = row[s + 2];
                    dst[o + 1] = row[s + 1];
                    dst[o + 2] = row[s];
                    dst[o + 3] = 255;
                }
            }
            32 => {
                for x in 0..row_w {
                    let s = x * 4;
                    let o = x * 4;
                    // classic 32-bit skin BMPs are BGRX; the stored alpha is ignored.
                    dst[o] = row[s + 2];
                    dst[o + 1] = row[s + 1];
                    dst[o + 2] = row[s];
                    dst[o + 3] = 255;
                }
            }
            other => return Err(BmpError::UnsupportedBpp(other)),
        }
    }

    Ok(Image {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16b(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }
    fn u32b(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }
    fn i32b(v: i32) -> [u8; 4] {
        v.to_le_bytes()
    }

    /// Assemble a BMP with a 40-byte INFOHEADER from raw palette + pixel bytes.
    fn build(width: i32, height: i32, bpp: u16, palette: &[[u8; 4]], pixels: &[u8]) -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&u32b(40)); // biSize
        dib.extend_from_slice(&i32b(width));
        dib.extend_from_slice(&i32b(height));
        dib.extend_from_slice(&u16b(1)); // planes
        dib.extend_from_slice(&u16b(bpp));
        dib.extend_from_slice(&u32b(0)); // BI_RGB
        dib.extend_from_slice(&u32b(0)); // sizeImage
        dib.extend_from_slice(&i32b(2835)); // x px/m
        dib.extend_from_slice(&i32b(2835)); // y px/m
        dib.extend_from_slice(&u32b(palette.len() as u32)); // clrUsed
        dib.extend_from_slice(&u32b(0)); // clrImportant

        let mut pal = Vec::new();
        for e in palette {
            pal.extend_from_slice(e);
        }

        let off_bits = (FILE_HEADER + dib.len() + pal.len()) as u32;
        let total = off_bits as usize + pixels.len();

        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&u32b(total as u32)); // fileSize
        out.extend_from_slice(&u16b(0));
        out.extend_from_slice(&u16b(0));
        out.extend_from_slice(&u32b(off_bits));
        out.extend_from_slice(&dib);
        out.extend_from_slice(&pal);
        out.extend_from_slice(pixels);
        out
    }

    fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * img.width + x) * 4) as usize;
        [
            img.rgba[o],
            img.rgba[o + 1],
            img.rgba[o + 2],
            img.rgba[o + 3],
        ]
    }

    #[test]
    fn decodes_24bit_bottom_up() {
        // Expected top-down image:
        //   row0: red,  green
        //   row1: blue, white
        // Stored bottom-up (bottom row first); pixels are BGR; rows padded to 4 bytes.
        let row_bottom = [255u8, 0, 0, 255, 255, 255, 0, 0]; // blue, white, pad
        let row_top = [0u8, 0, 255, 0, 255, 0, 0, 0]; // red, green, pad
        let mut pixels = Vec::new();
        pixels.extend_from_slice(&row_bottom);
        pixels.extend_from_slice(&row_top);

        let img = decode(&build(2, 2, 24, &[], &pixels)).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]); // red
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]); // green
        assert_eq!(px(&img, 0, 1), [0, 0, 255, 255]); // blue
        assert_eq!(px(&img, 1, 1), [255, 255, 255, 255]); // white
    }

    #[test]
    fn decodes_8bit_palette_top_down() {
        // Palette (BGRA): 0=black, 1=red, 2=green, 3=blue.
        let palette = [
            [0, 0, 0, 0],
            [0, 0, 255, 0],
            [0, 255, 0, 0],
            [255, 0, 0, 0],
        ];
        // 2x1, negative height = top-down; indices [1, 2] then pad to 4 bytes.
        let img = decode(&build(2, -1, 8, &palette, &[1, 2, 0, 0])).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]); // red
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]); // green
    }

    #[test]
    fn decodes_1bit_monochrome() {
        // The bit depth stb_image gets wrong. Palette: 0=black, 1=white.
        let palette = [[0, 0, 0, 0], [255, 255, 255, 0]];
        // 8x1 top-down; 0b1010_1010 => white, black, white, ...; padded to 4 bytes.
        let img = decode(&build(8, -1, 1, &palette, &[0b1010_1010, 0, 0, 0])).unwrap();
        assert_eq!((img.width, img.height), (8, 1));
        for x in 0..8 {
            let expect = if x % 2 == 0 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            };
            assert_eq!(px(&img, x, 0), expect, "pixel {x}");
        }
    }

    #[test]
    fn decodes_4bit_palette() {
        // Palette (BGRA): 0=black, 1=red, 2=green.
        let palette = [[0, 0, 0, 0], [0, 0, 255, 0], [0, 255, 0, 0]];
        // 2x1 top-down; one byte packs two nibbles: high=1 (red), low=2 (green).
        let img = decode(&build(2, -1, 4, &palette, &[0x12, 0, 0, 0])).unwrap();
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]); // red
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]); // green
    }

    #[test]
    fn rejects_bad_input() {
        let mut not_bmp = vec![0u8; FILE_HEADER + 40];
        not_bmp[0] = b'X';
        not_bmp[1] = b'X';
        assert_eq!(decode(&not_bmp), Err(BmpError::BadMagic));
        assert_eq!(decode(b"BM"), Err(BmpError::TooShort));
    }
}
