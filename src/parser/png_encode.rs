//! Re-encode already-decompressed raw pixel bytes (from a `/FlateDecode` image XObject) into a
//! valid PNG container. `RawBackend::page_xobjects` already inflates the stream (and reverses
//! any PNG/TIFF predictor) before this runs, so the only job here is wrapping the scanlines in
//! PNG's own filter-byte-per-row + zlib + chunk/CRC framing.
//!
//! Scope: 8-bit-per-component `DeviceGray`/`DeviceRGB` only (see
//! `claudedocs/unpdf/issues/ISSUE-unpdf-20260828-123513-flatedecode-images-unconditionally-dropped.md`
//! for the staged-rollout rationale). Anything else is `None` — the caller falls back to the
//! existing raw/undecoded-drop path, which the resource-inventory layer now reports as an
//! "unsupported image" quality signal rather than silent absence.

/// Number of color channels a PNG color type carries — the only two this encoder emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PngColorType {
    Gray,
    Rgb,
}

impl PngColorType {
    fn channels(self) -> u32 {
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
pub(crate) fn encode(width: u32, height: u32, color_type: PngColorType, pixel_data: &[u8]) -> Option<Vec<u8>> {
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

    fn decode_with_png_crate(bytes: &[u8]) -> (u32, u32, png::ColorType, Vec<u8>) {
        let decoder = png::Decoder::new(bytes);
        let mut reader = decoder.read_info().expect("valid PNG produced by encode()");
        let mut buf = vec![0u8; reader.output_buffer_size()];
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

        assert_eq!(&png_bytes[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

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

    #[test]
    fn rejects_zero_dimensions() {
        assert!(encode(0, 4, PngColorType::Rgb, &[]).is_none());
        assert!(encode(4, 0, PngColorType::Rgb, &[]).is_none());
    }
}
