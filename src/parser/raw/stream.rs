//! PDF stream decompression.

use super::tokenizer::{dict_get, PdfDict, PdfObject, PdfStream};
use crate::error::{Error, Result};
use std::io::Read;

/// The bytes of a stream with every lossless filter in its chain applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The stream's data after the lossless filters.
    pub data: Vec<u8>,
    /// The image codec the chain ends in (`DCTDecode`, `JPXDecode`, `JBIG2Decode`,
    /// `CCITTFaxDecode`), by its full name, when `data` is still encoded in it. Such a
    /// filter is the last in a chain: its output is an image, not bytes another filter
    /// reads.
    pub codec: Option<&'static str>,
}

/// Decode a stream completely, every filter in its chain applied in order.
///
/// A stream that ends in an image codec cannot be decoded to bytes here and is an
/// error; [`decode`] stops before the codec instead.
pub fn decompress(stream: &PdfStream) -> Result<Vec<u8>> {
    let decoded = decode(stream)?;
    match decoded.codec {
        None => Ok(decoded.data),
        Some(codec) => Err(Error::PdfParse(format!("unsupported filter: {codec}"))),
    }
}

/// Apply a stream's filter chain in order (PDF 32000-1 §7.4.1: each filter decodes the
/// output of the one before it), each stage with its own `DecodeParms`, stopping before
/// an image codec.
///
/// `Filter` is a name or an array of names, abbreviated (`AHx`, `A85`, `LZW`, `Fl`, `RL`,
/// `DCT`, `CCF`, as inline images write them) or not. `DecodeParms` is a dictionary for a
/// single filter, or an array parallel to `Filter` whose entries may be `null`. A lone
/// dictionary beside a filter array — not what the specification allows, but written —
/// is taken as the parameters of the last filter, which is where a predictor belongs.
pub fn decode(stream: &PdfStream) -> Result<Decoded> {
    let filters: Vec<&[u8]> = match dict_get(&stream.dict, b"Filter") {
        None | Some(PdfObject::Null) => Vec::new(),
        Some(PdfObject::Name(name)) => vec![name.as_slice()],
        Some(PdfObject::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_name()
                    .ok_or_else(|| Error::PdfParse("filter array entry is not a name".into()))
            })
            .collect::<Result<_>>()?,
        Some(_) => return Err(Error::PdfParse("Filter is not a name or an array".into())),
    };

    let mut parms: Vec<Option<&PdfDict>> = vec![None; filters.len()];
    match dict_get(&stream.dict, b"DecodeParms") {
        Some(PdfObject::Array(items)) => {
            for (slot, item) in parms.iter_mut().zip(items) {
                *slot = item.as_dict();
            }
        }
        Some(other) => {
            if let (Some(dict), Some(last)) = (other.as_dict(), parms.last_mut()) {
                *last = Some(dict);
            }
        }
        None => {}
    }

    let mut data = stream.raw_data.clone();
    for (index, (&name, parms)) in filters.iter().zip(&parms).enumerate() {
        if let Some(codec) = image_codec(name) {
            if index + 1 != filters.len() {
                return Err(Error::PdfParse(format!(
                    "image filter {codec} is followed by another filter"
                )));
            }
            return Ok(Decoded {
                data,
                codec: Some(codec),
            });
        }
        data = decode_stage(name, &data, *parms)?;
    }
    Ok(Decoded { data, codec: None })
}

/// The full name of an image codec filter, or `None` for a filter that yields bytes.
fn image_codec(name: &[u8]) -> Option<&'static str> {
    match name {
        b"DCTDecode" | b"DCT" => Some("DCTDecode"),
        b"JPXDecode" => Some("JPXDecode"),
        b"JBIG2Decode" => Some("JBIG2Decode"),
        b"CCITTFaxDecode" | b"CCF" => Some("CCITTFaxDecode"),
        _ => None,
    }
}

/// Apply one lossless filter, with its predictor where the filter takes one.
fn decode_stage(name: &[u8], data: &[u8], parms: Option<&PdfDict>) -> Result<Vec<u8>> {
    let predicted = |decoded: Vec<u8>| match parms {
        Some(parms) => apply_predictor(parms, &decoded),
        None => Ok(decoded),
    };
    match name {
        b"FlateDecode" | b"Fl" => predicted(decompress_flate(data)?),
        b"LZWDecode" | b"LZW" => {
            let early_change = parms
                .and_then(|p| dict_get(p, b"EarlyChange"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                != 0;
            predicted(decode_lzw(data, early_change)?)
        }
        b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(data),
        b"ASCII85Decode" | b"A85" => decode_ascii85(data),
        b"RunLengthDecode" | b"RL" => Ok(decode_run_length(data)),
        _ => Err(Error::PdfParse(format!(
            "unsupported filter: {}",
            String::from_utf8_lossy(name)
        ))),
    }
}

/// `ASCII85Decode` (PDF 32000-1 §7.4.3): groups of five characters `!`..`u` encode four
/// bytes, `z` stands for four zero bytes, whitespace is ignored and `~>` ends the data.
/// A final group of n characters encodes n - 1 bytes.
fn decode_ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    let mut out = Vec::with_capacity(body.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut filled = 0;
    for &byte in body {
        match byte {
            b'~' => break,
            b'z' if filled == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[filled] = byte - b'!';
                filled += 1;
                if filled == 5 {
                    out.extend_from_slice(&ascii85_group(&group)?);
                    filled = 0;
                }
            }
            _ if byte.is_ascii_whitespace() => {}
            _ => {
                return Err(Error::PdfParse(format!(
                    "invalid character 0x{byte:02X} in ASCII85Decode"
                )))
            }
        }
    }
    if filled == 1 {
        return Err(Error::PdfParse("truncated ASCII85Decode group".into()));
    }
    if filled > 1 {
        // Pad with the highest digit and keep the bytes the short group encodes.
        group[filled..].fill(b'u' - b'!');
        out.extend_from_slice(&ascii85_group(&group)?[..filled - 1]);
    }
    Ok(out)
}

fn ascii85_group(digits: &[u8; 5]) -> Result<[u8; 4]> {
    let value = digits
        .iter()
        .try_fold(0u64, |acc, &d| Some(acc * 85 + u64::from(d)))
        .filter(|&v| v <= u64::from(u32::MAX))
        .ok_or_else(|| Error::PdfParse("ASCII85Decode group out of range".into()))?;
    Ok((value as u32).to_be_bytes())
}

/// `RunLengthDecode` (PDF 32000-1 §7.4.5): a length byte n below 128 copies the next
/// n + 1 bytes, above 128 repeats the next byte 257 - n times, and 128 ends the data.
fn decode_run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&length) = data.get(i) {
        i += 1;
        match length {
            128 => break,
            0..=127 => {
                let end = (i + length as usize + 1).min(data.len());
                out.extend_from_slice(&data[i..end]);
                i = end;
            }
            _ => {
                if let Some(&byte) = data.get(i) {
                    out.extend(std::iter::repeat_n(byte, 257 - length as usize));
                }
                i += 1;
            }
        }
    }
    out
}

/// `LZWDecode` (PDF 32000-1 §7.4.4): variable-width codes from 9 to 12 bits, 256 clears
/// the table and 257 ends the data. With `early_change` (the default) the code width
/// grows one code earlier than the table strictly needs, as TIFF writers do.
fn decode_lzw(data: &[u8], early_change: bool) -> Result<Vec<u8>> {
    const CLEAR: usize = 256;
    const END: usize = 257;
    const MAX_CODES: usize = 4096;

    let fresh = || -> Vec<Vec<u8>> {
        let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
        table.push(Vec::new()); // CLEAR
        table.push(Vec::new()); // END
        table
    };
    let mut table = fresh();
    let mut out = Vec::new();
    let mut previous: Option<usize> = None;
    let mut width = 9u32;
    let (mut buffer, mut buffered, mut pos) = (0u32, 0u32, 0usize);

    loop {
        while buffered < width {
            let Some(&byte) = data.get(pos) else {
                return Ok(out); // data ran out without END: keep what was decoded
            };
            buffer = (buffer << 8) | u32::from(byte);
            buffered += 8;
            pos += 1;
        }
        let code = ((buffer >> (buffered - width)) & ((1 << width) - 1)) as usize;
        buffered -= width;
        buffer &= (1u32 << buffered) - 1;

        match code {
            CLEAR => {
                table = fresh();
                width = 9;
                previous = None;
                continue;
            }
            END => return Ok(out),
            _ => {}
        }

        let entry = match (table.get(code), previous) {
            (Some(entry), _) if !(CLEAR..=END).contains(&code) => entry.clone(),
            (None, Some(prev)) if code == table.len() => {
                let mut entry = table[prev].clone();
                entry.push(table[prev][0]);
                entry
            }
            _ => return Err(Error::PdfParse(format!("invalid LZW code {code}"))),
        };
        out.extend_from_slice(&entry);
        if let Some(prev) = previous {
            if table.len() < MAX_CODES {
                let mut grown = table[prev].clone();
                grown.push(entry[0]);
                table.push(grown);
            }
        }
        previous = Some(code);

        let next = table.len() + usize::from(early_change);
        width = match next {
            n if n >= 2048 => 12,
            n if n >= 1024 => 11,
            n if n >= 512 => 10,
            _ => 9,
        };
    }
}

fn decompress_flate(data: &[u8]) -> Result<Vec<u8>> {
    // Try zlib first (most common)
    let mut output = Vec::new();
    if flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut output)
        .is_ok()
    {
        return Ok(output);
    }

    // Fallback: raw deflate (some PDF producers omit zlib header)
    output.clear();
    if flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut output)
        .is_ok()
    {
        return Ok(output);
    }

    // Lenient fallback: read as much as we can from partial/corrupt streams.
    // Some PDFs have trailing garbage, wrong /Length values, or minor stream
    // corruption. We try both zlib and raw deflate in chunked reads and return
    // whatever partial data was successfully decompressed.
    output.clear();
    if let Some(partial) = decompress_flate_partial(data) {
        if !partial.is_empty() {
            return Ok(partial);
        }
    }

    Err(Error::PdfParse(
        "decompression failed: corrupt deflate stream".to_string(),
    ))
}

/// Attempt to decompress as much data as possible from a potentially
/// corrupt or truncated flate stream. Returns `Some(data)` with whatever
/// bytes were successfully decompressed, or `None` on total failure.
fn decompress_flate_partial(data: &[u8]) -> Option<Vec<u8>> {
    // Try zlib partial read
    let mut best = try_partial_read(flate2::read::ZlibDecoder::new(data));

    // Try raw deflate partial read
    let raw = try_partial_read(flate2::read::DeflateDecoder::new(data));
    if raw.len() > best.len() {
        best = raw;
    }

    // Also try trimming trailing bytes (some PDFs append extra zeros/garbage).
    // Try stripping up to 8 trailing bytes.
    for trim in 1..=std::cmp::min(8, data.len().saturating_sub(1)) {
        let trimmed = &data[..data.len() - trim];

        let z = try_partial_read(flate2::read::ZlibDecoder::new(trimmed));
        if z.len() > best.len() {
            best = z;
        }

        let d = try_partial_read(flate2::read::DeflateDecoder::new(trimmed));
        if d.len() > best.len() {
            best = d;
        }
    }

    if best.is_empty() {
        None
    } else {
        Some(best)
    }
}

/// Read from a decoder in small chunks, returning whatever was successfully
/// decompressed before an error occurs.
fn try_partial_read<R: Read>(mut reader: R) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
            Err(_) => break, // stop at first error, keep what we have
        }
    }
    output
}

/// Apply predictor decoding as specified by DecodeParms.
///
/// PDF supports two families of predictors:
/// - TIFF Predictor 2: horizontal differencing
/// - PNG Predictors 10-15: PNG filter methods (None, Sub, Up, Average, Paeth, Optimum)
fn apply_predictor(parms: &PdfDict, data: &[u8]) -> Result<Vec<u8>> {
    let predictor = dict_get(parms, b"Predictor")
        .and_then(|o| o.as_i64())
        .unwrap_or(1);

    // Predictor 1 = no prediction
    if predictor == 1 {
        return Ok(data.to_vec());
    }

    let columns = dict_get(parms, b"Columns")
        .and_then(|o| o.as_i64())
        .unwrap_or(1) as usize;

    let colors = dict_get(parms, b"Colors")
        .and_then(|o| o.as_i64())
        .unwrap_or(1) as usize;

    let bits_per_component = dict_get(parms, b"BitsPerComponent")
        .and_then(|o| o.as_i64())
        .unwrap_or(8) as usize;

    if predictor == 2 {
        // TIFF Predictor 2: horizontal differencing
        return apply_tiff_predictor(data, columns, colors, bits_per_component);
    }

    if (10..=15).contains(&predictor) {
        // PNG predictors
        return apply_png_predictor(data, columns, colors, bits_per_component);
    }

    // Unknown predictor, return data as-is
    Ok(data.to_vec())
}

/// Apply TIFF Predictor 2 (horizontal differencing).
fn apply_tiff_predictor(
    data: &[u8],
    columns: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<Vec<u8>> {
    if bits_per_component != 8 {
        // Only 8-bit components are commonly used; return as-is for others
        return Ok(data.to_vec());
    }
    let row_bytes = columns * colors;
    if row_bytes == 0 {
        return Ok(data.to_vec());
    }

    let mut result = data.to_vec();
    let num_rows = result.len() / row_bytes;

    for row in 0..num_rows {
        let row_start = row * row_bytes;
        for col in colors..row_bytes {
            let idx = row_start + col;
            if idx < result.len() {
                result[idx] = result[idx].wrapping_add(result[idx - colors]);
            }
        }
    }

    Ok(result)
}

/// Apply PNG predictor decoding.
///
/// Each row is prefixed with a 1-byte filter type:
/// 0 = None, 1 = Sub, 2 = Up, 3 = Average, 4 = Paeth
fn apply_png_predictor(
    data: &[u8],
    columns: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<Vec<u8>> {
    // bytes per pixel (for Sub/Paeth filter lookback)
    let bpp = std::cmp::max(1, (colors * bits_per_component).div_ceil(8));
    // row data bytes (excluding filter byte)
    let row_bytes = columns * colors * bits_per_component / 8;
    // each input row = 1 filter byte + row_bytes data bytes
    let input_row_len = 1 + row_bytes;

    if input_row_len == 0 || !data.len().is_multiple_of(input_row_len) {
        // If data doesn't divide evenly, try using columns directly as row_bytes
        // (common when Columns already accounts for all bytes per row)
        let alt_row_bytes = columns;
        let alt_input_row_len = 1 + alt_row_bytes;
        if alt_input_row_len > 0 && data.len().is_multiple_of(alt_input_row_len) {
            return apply_png_predictor_raw(data, alt_row_bytes, bpp);
        }
        // Fall back: return data as-is rather than fail
        return Ok(data.to_vec());
    }

    apply_png_predictor_raw(data, row_bytes, bpp)
}

/// Core PNG predictor un-filtering.
fn apply_png_predictor_raw(data: &[u8], row_bytes: usize, bpp: usize) -> Result<Vec<u8>> {
    let input_row_len = 1 + row_bytes;
    let num_rows = data.len() / input_row_len;
    let mut result = Vec::with_capacity(num_rows * row_bytes);
    let mut prev_row = vec![0u8; row_bytes];

    for row_idx in 0..num_rows {
        let row_start = row_idx * input_row_len;
        let filter_type = data[row_start];
        let row_data = &data[row_start + 1..row_start + input_row_len];

        let mut current_row = vec![0u8; row_bytes];

        match filter_type {
            0 => {
                // None
                current_row.copy_from_slice(row_data);
            }
            1 => {
                // Sub
                for i in 0..row_bytes {
                    let left = if i >= bpp { current_row[i - bpp] } else { 0 };
                    current_row[i] = row_data[i].wrapping_add(left);
                }
            }
            2 => {
                // Up
                for i in 0..row_bytes {
                    current_row[i] = row_data[i].wrapping_add(prev_row[i]);
                }
            }
            3 => {
                // Average
                for i in 0..row_bytes {
                    let left = if i >= bpp {
                        current_row[i - bpp] as u16
                    } else {
                        0
                    };
                    let up = prev_row[i] as u16;
                    current_row[i] = row_data[i].wrapping_add(((left + up) / 2) as u8);
                }
            }
            4 => {
                // Paeth
                for i in 0..row_bytes {
                    let left = if i >= bpp { current_row[i - bpp] } else { 0 };
                    let up = prev_row[i];
                    let up_left = if i >= bpp { prev_row[i - bpp] } else { 0 };
                    current_row[i] = row_data[i].wrapping_add(paeth_predictor(left, up, up_left));
                }
            }
            _ => {
                // Unknown filter type — treat as None
                current_row.copy_from_slice(row_data);
            }
        }

        result.extend_from_slice(&current_row);
        prev_row = current_row;
    }

    Ok(result)
}

/// Paeth predictor function (used in PNG filter type 4).
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn decode_ascii_hex(data: &[u8]) -> Result<Vec<u8>> {
    let hex: String = data
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .take_while(|&&b| b != b'>')
        .map(|&b| b as char)
        .collect();
    let mut result = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.chars();
    while let Some(h) = chars.next() {
        let l = chars.next().unwrap_or('0');
        let byte = u8::from_str_radix(&format!("{}{}", h, l), 16)
            .map_err(|_| Error::PdfParse("invalid hex in ASCIIHexDecode".to_string()))?;
        result.push(byte);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_decompress_uncompressed() {
        let stream = PdfStream {
            dict: BTreeMap::new(),
            raw_data: b"Hello World".to_vec(),
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_decompress_flate() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"Hello Compressed").unwrap();
        let compressed = encoder.finish().unwrap();

        let mut dict = BTreeMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"FlateDecode".to_vec()));

        let stream = PdfStream {
            dict,
            raw_data: compressed,
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello Compressed");
    }

    #[test]
    fn test_decode_ascii_hex() {
        let result = decode_ascii_hex(b"48 65 6C 6C 6F>").unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_unsupported_filter() {
        let mut dict = BTreeMap::new();
        dict.insert(
            b"Filter".to_vec(),
            PdfObject::Name(b"Crypt2Decode".to_vec()),
        );
        let stream = PdfStream {
            dict,
            raw_data: vec![1, 2, 3],
        };
        assert!(decompress(&stream).is_err());
    }

    fn stream_with(filter: PdfObject, parms: Option<PdfObject>, raw: Vec<u8>) -> PdfStream {
        let mut dict = BTreeMap::new();
        dict.insert(b"Filter".to_vec(), filter);
        if let Some(parms) = parms {
            dict.insert(b"DecodeParms".to_vec(), parms);
        }
        PdfStream {
            dict,
            raw_data: raw,
        }
    }

    fn names(names: &[&str]) -> PdfObject {
        PdfObject::Array(
            names
                .iter()
                .map(|n| PdfObject::Name(n.as_bytes().to_vec()))
                .collect(),
        )
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// ASCII85 of `bytes`, as Adobe writes it (`z` for zero groups, `~>` at the end).
    fn ascii85(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in bytes.chunks(4) {
            let mut group = [0u8; 4];
            group[..chunk.len()].copy_from_slice(chunk);
            let mut value = u32::from_be_bytes(group);
            if chunk.len() == 4 && value == 0 {
                out.push(b'z');
                continue;
            }
            let mut digits = [0u8; 5];
            for d in digits.iter_mut().rev() {
                *d = (value % 85) as u8 + b'!';
                value /= 85;
            }
            out.extend_from_slice(&digits[..chunk.len() + 1]);
        }
        out.extend_from_slice(b"~>");
        out
    }

    /// The chain ReportLab writes for every content stream when its ASCII output is on.
    #[test]
    fn ascii85_then_flate_is_decoded_in_order() {
        let content = b"BT /F1 24 Tf 72 720 Td (Hello ASCII85 filter chain) Tj ET";
        let raw = ascii85(&zlib(content));
        let stream = stream_with(names(&["ASCII85Decode", "FlateDecode"]), None, raw.clone());
        assert_eq!(decompress(&stream).unwrap(), content);

        let abbreviated = stream_with(names(&["A85", "Fl"]), None, raw);
        assert_eq!(decompress(&abbreviated).unwrap(), content);
    }

    #[test]
    fn ascii85_handles_zero_groups_whitespace_and_a_short_last_group() {
        let bytes = [0u8, 0, 0, 0, 1, 2, 3, 4, 5, 6];
        let mut encoded = ascii85(&bytes);
        encoded.insert(3, b'\n');
        encoded.insert(1, b' ');
        assert_eq!(decode_ascii85(&encoded).unwrap(), bytes);
        assert_eq!(
            decode_ascii85(b"<~87cURD]i,\"Ebo7~>").unwrap(),
            b"Hello World"
        );
        assert!(decode_ascii85(b"ab{c~>").is_err());
    }

    #[test]
    fn run_length_copies_and_repeats() {
        // 2 → copy "abc", 254 → repeat 'x' 3 times, 128 → end.
        let raw = [2, b'a', b'b', b'c', 254, b'x', 128, b'z'];
        let stream = stream_with(PdfObject::Name(b"RL".to_vec()), None, raw.to_vec());
        assert_eq!(decompress(&stream).unwrap(), b"abcxxx");
    }

    /// The example of PDF 32000-1 §7.4.4.2: `-----A---B` encoded with EarlyChange 1.
    #[test]
    fn lzw_decodes_the_specification_example() {
        let encoded = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let stream = stream_with(
            PdfObject::Name(b"LZWDecode".to_vec()),
            None,
            encoded.to_vec(),
        );
        assert_eq!(decompress(&stream).unwrap(), b"-----A---B");
    }

    #[test]
    fn decode_parms_array_runs_parallel_to_the_filters() {
        // Two rows of three bytes, PNG "Up" predictor on each row.
        let rows = [2, 1, 2, 3, 2, 1, 1, 1];
        let raw = ascii85(&zlib(&rows));
        let mut predictor = BTreeMap::new();
        predictor.insert(b"Predictor".to_vec(), PdfObject::Integer(12));
        predictor.insert(b"Columns".to_vec(), PdfObject::Integer(3));
        let parms = PdfObject::Array(vec![PdfObject::Null, PdfObject::Dict(predictor)]);
        let stream = stream_with(names(&["A85", "FlateDecode"]), Some(parms), raw);
        assert_eq!(decompress(&stream).unwrap(), vec![1, 2, 3, 2, 3, 4]);
    }

    /// An image chain stops before its codec: the bytes handed on are the JPEG itself.
    #[test]
    fn an_image_codec_ends_the_lossless_part_of_a_chain() {
        let jpeg = b"\xFF\xD8\xFF\xE0 not really a jpeg".to_vec();
        let stream = stream_with(names(&["ASCII85Decode", "DCTDecode"]), None, ascii85(&jpeg));
        let decoded = decode(&stream).unwrap();
        assert_eq!(decoded.data, jpeg);
        assert_eq!(decoded.codec, Some("DCTDecode"));
        assert!(decompress(&stream).is_err());

        let inline = stream_with(PdfObject::Name(b"DCT".to_vec()), None, jpeg.clone());
        assert_eq!(decode(&inline).unwrap().codec, Some("DCTDecode"));
    }
}
