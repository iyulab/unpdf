//! Re-encode already-decompressed raw pixel bytes (from a `/FlateDecode` image XObject) into a
//! valid PNG container. `RawBackend::page_xobjects` already inflates the stream (and reverses
//! any PNG/TIFF predictor) before this runs, so the only job here is wrapping the scanlines in
//! PNG's own filter-byte-per-row + zlib + chunk/CRC framing.
//!
//! [`image_to_png`] first turns the image's samples into 8-bit gray or RGB pixels: it unpacks
//! 1, 2, 4, 8 and 16-bit samples, applies `/Decode`, converts `DeviceCMYK` to RGB (as PDFium
//! does, [`super::cmyk`]), converts `Lab` to sRGB ([`super::lab`]) and looks `Indexed` samples up
//! in their palette. Anything else (a `Separation` or `DeviceN` whose transform it does not read,
//! ...) is `None` — the caller falls back to the raw/undecoded-drop path, which the
//! resource-inventory layer reports as an "unsupported image" quality signal rather than
//! silent absence.

use super::backend::ImageColorSpace;
use super::cmyk::cmyk_to_rgb_unit;
use super::lab::lab_to_rgb_unit;

/// The widest or tallest image this decodes, in pixels: PDFium's `kMaxImageDimension`.
pub(crate) const MAX_IMAGE_DIMENSION: u32 = 0x01_FFFF;

/// Whether an image of `width` × `height` pixels, `channels` bytes each once decoded, is one
/// this decodes: each side within [`MAX_IMAGE_DIMENSION`], and the decoded buffer within what a
/// `u32` counts, as PDFium bounds a bitmap's pitch times its height.
///
/// The samples are already bounded by the stream they come from, but a pixel costs more than a
/// sample: a 1-bit palette image becomes 24 bits of RGB, 32 of RGBA when drawn, so the stream
/// limit alone lets an image ask for 32 times its decoded data.
pub(crate) fn decodable(width: u32, height: u32, channels: u32) -> bool {
    width <= MAX_IMAGE_DIMENSION
        && height <= MAX_IMAGE_DIMENSION
        && u64::from(width) * u64::from(height) * u64::from(channels) <= u64::from(u32::MAX)
}

/// Convert an image XObject's decompressed samples to a PNG.
///
/// `data` holds `height` rows of `width` samples of `color.components()` components each,
/// `bits_per_component` bits per component, every row starting on a byte boundary (ISO
/// 32000-1 §8.9.3). `decode` is the image's `/Decode` array, if it has one. `None` when the
/// color space or bit depth is outside what this converts, or the data is too short.
pub(crate) fn image_to_png(
    width: u32,
    height: u32,
    bits_per_component: u8,
    color: &ImageColorSpace,
    decode: Option<&[f32]>,
    data: &[u8],
) -> Option<Vec<u8>> {
    let (output, pixels) = image_pixels(width, height, bits_per_component, color, decode, data)?;
    encode(width, height, output, &pixels)
}

/// Convert an image XObject's decompressed samples to 8-bit pixels: gray or RGB, as the
/// returned colour type says, `width * height` of them row by row. Same inputs and same
/// `None` cases as [`image_to_png`].
pub(crate) fn image_pixels(
    width: u32,
    height: u32,
    bits_per_component: u8,
    color: &ImageColorSpace,
    decode: Option<&[f32]>,
    data: &[u8],
) -> Option<(PngColorType, Vec<u8>)> {
    if !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) || width == 0 || height == 0 {
        return None;
    }
    let output = color.output();
    if !decodable(width, height, output.channels()) {
        return None;
    }
    let components = color.components();
    let bpc = bits_per_component as usize;
    let samples_per_row = (width as usize).checked_mul(components)?;
    let row_bytes = samples_per_row.checked_mul(bpc)?.div_ceil(8);
    if data.len() < row_bytes.checked_mul(height as usize)? {
        return None;
    }
    let max = ((1u32 << bpc) - 1) as f32;
    // The `[Dmin Dmax]` pair each component's raw sample maps through.
    let range = |i: usize| -> (f32, f32) {
        match decode {
            Some(d) if d.len() >= 2 * components => (d[2 * i], d[2 * i + 1]),
            _ => match color {
                ImageColorSpace::Indexed { .. } => (0.0, max),
                _ => color.default_decode(i),
            },
        }
    };

    let lut: Option<Vec<Vec<u8>>> = match lut_entries(color, bits_per_component) {
        Some(entries) => {
            let (lo, hi) = range(0);
            Some(
                (0..entries)
                    .map(|s| {
                        let mut entry = Vec::with_capacity(output.channels() as usize);
                        color
                            .push_pixel(&[lo + s as f32 * (hi - lo) / max], &mut entry)
                            .map(|()| entry)
                    })
                    .collect::<Option<Vec<_>>>()?,
            )
        }
        None => None,
    };
    let mut pixels =
        Vec::with_capacity(width as usize * height as usize * output.channels() as usize);
    let mut values = vec![0f32; components];
    for row in data.chunks(row_bytes).take(height as usize) {
        for x in 0..width as usize {
            if let Some(lut) = &lut {
                pixels.extend_from_slice(&lut[sample_at(row, x * bpc, bpc) as usize]);
                continue;
            }
            for (i, value) in values.iter_mut().enumerate() {
                let sample = sample_at(row, (x * components + i) * bpc, bpc) as f32;
                let (lo, hi) = range(i);
                *value = lo + sample * (hi - lo) / max;
            }
            color.push_pixel(&values, &mut pixels)?;
        }
    }
    Some((output, pixels))
}

/// How many colors [`image_pixels`] converts up front for samples of `bits` bits in `color`, one
/// for each value a sample can take: a tint of one colorant has no more colors than that. `None`
/// when it converts each pixel instead. An image one of whose colors does not convert is not
/// converted, whether its samples take that value or not.
pub(crate) fn lut_entries(color: &ImageColorSpace, bits: u8) -> Option<u32> {
    (matches!(color, ImageColorSpace::Tint(_)) && color.components() == 1 && bits <= 8)
        .then(|| 1 << bits)
}

/// The `bits`-wide sample starting `bit` bits into `row`, high bits first.
fn sample_at(row: &[u8], bit: usize, bits: usize) -> u32 {
    match bits {
        16 => u32::from(row[bit / 8]) << 8 | u32::from(row[bit / 8 + 1]),
        8 => u32::from(row[bit / 8]),
        _ => {
            let byte = row[bit / 8];
            let shift = 8 - bits - bit % 8;
            u32::from(byte >> shift) & ((1 << bits) - 1)
        }
    }
}

fn to_byte(unit: f32) -> u8 {
    (unit.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl ImageColorSpace {
    /// The PNG colour type this space converts to.
    fn output(&self) -> PngColorType {
        match self {
            ImageColorSpace::Gray => PngColorType::Gray,
            ImageColorSpace::Indexed { base, .. } => base.output(),
            ImageColorSpace::Tint(space) => space.alternate().output(),
            ImageColorSpace::Rgb | ImageColorSpace::Cmyk | ImageColorSpace::Lab { .. } => {
                PngColorType::Rgb
            }
        }
    }

    /// Append the pixel whose components (decoded, `0..=1` -- or the palette index, for
    /// `Indexed`) are `values`.
    fn push_pixel(&self, values: &[f32], out: &mut Vec<u8>) -> Option<()> {
        match self {
            ImageColorSpace::Gray => out.push(to_byte(values[0])),
            ImageColorSpace::Rgb => out.extend(values[..3].iter().map(|&v| to_byte(v))),
            ImageColorSpace::Cmyk => {
                let rgb = cmyk_to_rgb_unit(values[0], values[1], values[2], values[3]);
                out.extend(rgb.map(to_byte));
            }
            ImageColorSpace::Lab { white, range } => {
                let rgb = lab_to_rgb_unit(*white, *range, [values[0], values[1], values[2]]);
                out.extend(rgb.map(to_byte));
            }
            ImageColorSpace::Indexed {
                base,
                hival,
                lookup,
            } => {
                let index = (values[0].round().max(0.0) as usize).min(*hival as usize);
                let n = base.components();
                let entry = lookup.get(index * n..index * n + n)?;
                base.push_pixel(&base.palette_entry(entry), out)?;
            }
            ImageColorSpace::Tint(space) => {
                let (alternate, components) = space.alternate_color(values)?;
                alternate.push_pixel(&components, out)?;
            }
        }
        Some(())
    }
}

/// Number of color channels a PNG color type carries — the only two this encoder emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PngColorType {
    Gray,
    Rgb,
}

impl PngColorType {
    pub(crate) fn channels(self) -> u32 {
        match self {
            PngColorType::Gray => 1,
            PngColorType::Rgb => 3,
        }
    }

    /// PNG IHDR "colour type" byte (spec §11.2.2).
    fn ihdr_byte(self) -> u8 {
        match self {
            PngColorType::Gray => 0,
            PngColorType::Rgb => 2,
        }
    }
}

/// Encode raw, unfiltered 8-bit scanlines into a PNG byte buffer.
///
/// `pixel_data` must be exactly `width * height * color_type.channels()` bytes — one byte per
/// sample, row-major, no filter bytes, no padding. Returns `None` if the length doesn't match
/// (the caller has no reliable recovery for a malformed source, so it falls back to reporting
/// the image as unsupported rather than emitting a corrupt PNG).
pub(crate) fn encode(
    width: u32,
    height: u32,
    color_type: PngColorType,
    pixel_data: &[u8],
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let channels = color_type.channels();
    let row_bytes = (width as usize).checked_mul(channels as usize)?;
    let expected_len = row_bytes.checked_mul(height as usize)?;
    if pixel_data.len() != expected_len {
        return None;
    }

    let mut png = Vec::with_capacity(pixel_data.len() + 64);
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(color_type.ihdr_byte());
    ihdr.push(0); // compression method (deflate, the only defined value)
    ihdr.push(0); // filter method (adaptive, the only defined value)
    ihdr.push(0); // interlace method (none)
    write_chunk(&mut png, b"IHDR", &ihdr);

    let mut filtered = Vec::with_capacity(pixel_data.len() + height as usize);
    for row in pixel_data.chunks_exact(row_bytes) {
        filtered.push(0); // filter type 0 = None, per row
        filtered.extend_from_slice(row);
    }
    let idat = zlib_compress(&filtered);
    write_chunk(&mut png, b"IDAT", &idat);

    write_chunk(&mut png, b"IEND", &[]);

    Some(png)
}

fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .expect("compressing into an in-memory Vec cannot fail");
    encoder
        .finish()
        .expect("compressing into an in-memory Vec cannot fail")
}

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32fast::hash(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_wider_than_the_bound_is_not_decoded() {
        // One row of 0x20000 one-bit samples: the data is all there, the width is past
        // PDFium's bound.
        let width = MAX_IMAGE_DIMENSION + 1;
        let data = vec![0u8; (width as usize).div_ceil(8)];
        assert!(image_pixels(width, 1, 1, &ImageColorSpace::Gray, None, &data).is_none());
        let data = vec![0u8; (MAX_IMAGE_DIMENSION as usize).div_ceil(8)];
        assert!(image_pixels(
            MAX_IMAGE_DIMENSION,
            1,
            1,
            &ImageColorSpace::Gray,
            None,
            &data
        )
        .is_some());
    }

    #[test]
    fn a_decoded_buffer_past_what_a_u32_counts_is_refused() {
        assert!(decodable(1000, 1000, 4));
        // 65 536 x 16 384 RGBA pixels are exactly 2^32 bytes.
        assert!(!decodable(65_536, 16_384, 4));
        assert!(decodable(65_536, 16_383, 4));
        assert!(!decodable(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION, 1));
    }

    fn decode_with_png_crate(bytes: &[u8]) -> (u32, u32, png::ColorType, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("valid PNG produced by encode()");
        let mut buf = vec![
            0u8;
            reader
                .output_buffer_size()
                .expect("PNG dimensions fit in a buffer")
        ];
        let info = reader.next_frame(&mut buf).expect("decodable IDAT");
        buf.truncate(info.buffer_size());
        (info.width, info.height, info.color_type, buf)
    }

    #[test]
    fn round_trips_a_2x2_rgb_image() {
        let pixels: Vec<u8> = vec![
            255, 0, 0, 0, 255, 0, // row 0: red, green
            0, 0, 255, 255, 255, 255, // row 1: blue, white
        ];
        let png_bytes = encode(2, 2, PngColorType::Rgb, &pixels).expect("valid input encodes");

        assert_eq!(
            &png_bytes[0..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );

        let (width, height, color_type, decoded) = decode_with_png_crate(&png_bytes);
        assert_eq!((width, height), (2, 2));
        assert_eq!(color_type, png::ColorType::Rgb);
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn round_trips_a_grayscale_image() {
        let pixels: Vec<u8> = vec![0, 64, 128, 192, 255, 32, 96, 160];
        let png_bytes = encode(4, 2, PngColorType::Gray, &pixels).expect("valid input encodes");

        let (width, height, color_type, decoded) = decode_with_png_crate(&png_bytes);
        assert_eq!((width, height), (4, 2));
        assert_eq!(color_type, png::ColorType::Grayscale);
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn rejects_pixel_data_of_the_wrong_length() {
        // 2x2 RGB needs 12 bytes; give it 11.
        let short = vec![0u8; 11];
        assert!(encode(2, 2, PngColorType::Rgb, &short).is_none());
    }

    fn converted(
        width: u32,
        height: u32,
        bpc: u8,
        color: &ImageColorSpace,
        decode: Option<&[f32]>,
        data: &[u8],
    ) -> (png::ColorType, Vec<u8>) {
        let png = image_to_png(width, height, bpc, color, decode, data).expect("converts");
        let (w, h, color_type, pixels) = decode_with_png_crate(&png);
        assert_eq!((w, h), (width, height));
        (color_type, pixels)
    }

    #[test]
    fn cmyk_converts_to_rgb() {
        // cyan, magenta, yellow, black, white (no ink)
        let data = [
            255, 0, 0, 0, //
            0, 255, 0, 0, //
            0, 0, 255, 0, //
            0, 0, 0, 255, //
            0, 0, 0, 0,
        ];
        let (color_type, pixels) = converted(5, 1, 8, &ImageColorSpace::Cmyk, None, &data);
        assert_eq!(color_type, png::ColorType::Rgb);
        assert_eq!(
            pixels,
            // The press colors (PDFium's table), not the naive complements.
            [0, 174, 239, 237, 2, 140, 255, 241, 1, 35, 31, 32, 255, 255, 255]
        );
    }

    /// Adobe-produced CMYK images are often stored inverted, with `/Decode [1 0 1 0 1 0 1 0]`.
    #[test]
    fn decode_inverts_the_samples_it_names() {
        let data = [0, 255, 255, 255]; // read through the inversion: full cyan
        let decode = [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let (_, pixels) = converted(1, 1, 8, &ImageColorSpace::Cmyk, Some(&decode), &data);
        assert_eq!(pixels, [0, 174, 239]);
    }

    /// Rows start on a byte boundary: a 3-pixel 1-bit row is one byte with 5 padding bits.
    #[test]
    fn one_bit_gray_unpacks_each_row_from_its_own_byte() {
        let data = [0b1010_0000, 0b0110_0000];
        let (color_type, pixels) = converted(3, 2, 1, &ImageColorSpace::Gray, None, &data);
        assert_eq!(color_type, png::ColorType::Grayscale);
        assert_eq!(pixels, [255, 0, 255, 0, 255, 255]);
    }

    #[test]
    fn indexed_samples_are_looked_up_in_the_palette() {
        let color = ImageColorSpace::Indexed {
            base: Box::new(ImageColorSpace::Rgb),
            hival: 2,
            lookup: vec![255, 0, 0, 0, 255, 0, 0, 0, 255],
        };
        // 4-bit indices 2, 0, 1 and a padding nibble.
        let data = [0x20, 0x10];
        let (color_type, pixels) = converted(3, 1, 4, &color, None, &data);
        assert_eq!(color_type, png::ColorType::Rgb);
        assert_eq!(pixels, [0, 0, 255, 255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn an_index_past_hival_takes_the_last_entry() {
        let color = ImageColorSpace::Indexed {
            base: Box::new(ImageColorSpace::Gray),
            hival: 1,
            lookup: vec![10, 20],
        };
        let (color_type, pixels) = converted(2, 1, 8, &color, None, &[0, 200]);
        assert_eq!(color_type, png::ColorType::Grayscale);
        assert_eq!(pixels, [10, 20]);
    }

    #[test]
    fn indexed_over_cmyk_converts_the_palette_entry() {
        let color = ImageColorSpace::Indexed {
            base: Box::new(ImageColorSpace::Cmyk),
            hival: 0,
            lookup: vec![0, 0, 0, 255],
        };
        let (_, pixels) = converted(1, 1, 8, &color, None, &[0]);
        assert_eq!(pixels, [35, 31, 32]);
    }

    const LAB: ImageColorSpace = ImageColorSpace::Lab {
        white: [0.9642, 1.0, 0.8249],
        range: [-100.0, 100.0, -60.0, 80.0],
    };

    #[test]
    fn lab_samples_span_the_default_decode_unless_the_image_names_one() {
        let data = [191, 64, 204, 191, 0, 255];
        let (color_type, pixels) = converted(2, 1, 8, &LAB, None, &data);
        assert_eq!(color_type, png::ColorType::Rgb);
        assert_eq!(pixels, [101, 207, 79, 0, 223, 0]);
        let decode = [0.0, 100.0, -50.0, 50.0, -50.0, 50.0];
        let (_, pixels) = converted(2, 1, 8, &LAB, Some(&decode), &data);
        assert_eq!(pixels, [152, 197, 127, 99, 207, 84]);
    }

    #[test]
    fn indexed_over_lab_spans_the_palette_bytes_across_the_lab_ranges() {
        let color = ImageColorSpace::Indexed {
            base: Box::new(LAB),
            hival: 0,
            lookup: vec![191, 64, 204],
        };
        let (_, pixels) = converted(1, 1, 8, &color, None, &[0]);
        assert_eq!(pixels, [101, 207, 79]);
    }

    #[test]
    fn sixteen_bit_samples_keep_their_high_byte() {
        let data = [0xFF, 0xFF, 0x80, 0x00];
        let (_, pixels) = converted(2, 1, 16, &ImageColorSpace::Gray, None, &data);
        assert_eq!(pixels, [255, 128]);
    }

    #[test]
    fn conversion_refuses_short_data_and_unknown_depths() {
        assert!(image_to_png(2, 2, 8, &ImageColorSpace::Rgb, None, &[0; 11]).is_none());
        assert!(image_to_png(1, 1, 3, &ImageColorSpace::Gray, None, &[0]).is_none());
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(encode(0, 4, PngColorType::Rgb, &[]).is_none());
        assert!(encode(4, 0, PngColorType::Rgb, &[]).is_none());
    }
}
