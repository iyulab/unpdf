//! PDF stream decompression.

use super::tokenizer::{dict_get, PdfDict, PdfObject, PdfStream};
use crate::error::{Error, Result};
use std::io::{self, Read};
use unparser_shared::inflate::{is_inflation_limit, InflateGuard, InflateLimits};

/// How far one filter's output may grow from its input before the stream is taken for a
/// decompression bomb — MuPDF's rule (`fz_read_best`): past 100 MiB, at most 200 times the
/// decoded length it expects of the filter, which it estimates as `expected` times the encoded
/// length (3 for Flate and RunLength, 2 for LZW, `pdf_guess_filter_length`). No stream passes
/// 1 GiB, PDFium's `kMaxTotalOutSize`. A blank 1-bit image mask compresses past a thousandfold,
/// so the ratio alone, without the 100 MiB grace, would refuse real files.
const fn pdf_limits(expected: u64) -> InflateLimits {
    InflateLimits {
        grace_size: 100 << 20,
        max_expansion: 200 * expected,
        max_size: 1 << 30,
    }
}

/// The most output an image's declared size excuses from the ratio check: a 10 000 x 10 000
/// RGB picture is 300 MB, and a hostile dictionary can declare any size.
const IMAGE_GRACE_CAP: u64 = 256 << 20;

/// `limits` with no ratio check up to `floor` bytes of output (at most [`IMAGE_GRACE_CAP`], and
/// never past `max_size`): an image says how big its samples are, so a stream that decodes to
/// no more than that is not a bomb however well it compresses. MuPDF checks no ratio there
/// either: it decodes an image stream as the image is read, bounded by its dimensions.
fn with_floor(limits: InflateLimits, floor: u64) -> InflateLimits {
    InflateLimits {
        grace_size: limits
            .grace_size
            .max(floor.min(IMAGE_GRACE_CAP).min(limits.max_size)),
        ..limits
    }
}

const FLATE_LIMITS: InflateLimits = pdf_limits(3);
const RUN_LENGTH_LIMITS: InflateLimits = pdf_limits(3);
const LZW_LIMITS: InflateLimits = pdf_limits(2);

/// The error of a filter whose output passed its limits.
fn bomb(error: io::Error) -> Error {
    Error::PdfParse(format!("decompression failed: {error}"))
}

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
    bytes(decode(stream)?)
}

/// [`decompress`] for a reader of only the data's first `len` bytes: the last filter stops once
/// it has given them, or, before a PNG predictor, the rows that hold them. A filter that expands
/// its input (Flate, LZW, RunLength) before the last, or a predictor row longer than `len`, is
/// an error.
pub fn decompress_prefix(stream: &PdfStream, len: u64) -> Result<Vec<u8>> {
    bytes(decode_up_to(stream, 0, Some(len))?)
}

fn bytes(decoded: Decoded) -> Result<Vec<u8>> {
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
    decode_up_to(stream, 0, None)
}

/// [`decode`] for an image whose samples are at most `expected_len` bytes: output up to that
/// size is not ratio-checked (see [`with_floor`]); the absolute limit still applies.
pub fn decode_image(stream: &PdfStream, expected_len: u64) -> Result<Decoded> {
    decode_up_to(stream, expected_len, None)
}

fn decode_up_to(stream: &PdfStream, floor: u64, prefix: Option<u64>) -> Result<Decoded> {
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
        let cap = match prefix {
            None => u64::MAX,
            Some(len) if index + 1 == filters.len() => {
                predicted_len(*parms, len).ok_or_else(|| {
                    Error::PdfParse("a predictor row is longer than the data read".into())
                })?
            }
            Some(_) if expands(name) => {
                return Err(Error::PdfParse(
                    "a filter that expands its input is followed by another".into(),
                ))
            }
            Some(_) => u64::MAX,
        };
        data = decode_stage(name, &data, *parms, floor, cap)?;
    }
    Ok(Decoded { data, codec: None })
}

/// The image codec the filter chain of `dict` ends in, by its full name; nothing is decoded.
pub fn image_codec_of(dict: &PdfDict) -> Option<&'static str> {
    let last = match dict_get(dict, b"Filter")? {
        PdfObject::Name(name) => name.as_slice(),
        PdfObject::Array(items) => items.last()?.as_name()?,
        _ => return None,
    };
    image_codec(last)
}

fn expands(name: &[u8]) -> bool {
    matches!(
        name,
        b"FlateDecode" | b"Fl" | b"LZWDecode" | b"LZW" | b"RunLengthDecode" | b"RL"
    )
}

/// How many bytes a filter must give for its predictor, set by `parms`, to give `len`: as many,
/// or under a PNG predictor every row holding them with its tag byte. `None` when a PNG row is
/// empty or longer than `len`.
fn predicted_len(parms: Option<&PdfDict>, len: u64) -> Option<u64> {
    let value = |key: &[u8], default: i64| {
        parms
            .and_then(|p| dict_get(p, key))
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    };
    if !(10..=15).contains(&value(b"Predictor", 1)) {
        return Some(len);
    }
    let bits = [b"Columns".as_slice(), b"Colors", b"BitsPerComponent"]
        .into_iter()
        .zip([1, 1, 8])
        .try_fold(1u64, |bits, (key, default)| {
            bits.checked_mul(u64::try_from(value(key, default)).ok()?)
        })?;
    let row = bits / 8;
    (row > 0 && row <= len).then(|| len.div_ceil(row) * (row + 1))
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

/// Apply one lossless filter, with its predictor where the filter takes one; a filter that
/// expands its input gives at most `cap` bytes.
fn decode_stage(
    name: &[u8],
    data: &[u8],
    parms: Option<&PdfDict>,
    floor: u64,
    cap: u64,
) -> Result<Vec<u8>> {
    let predicted = |decoded: Vec<u8>| match parms {
        Some(parms) => apply_predictor(parms, &decoded),
        None => Ok(decoded),
    };
    match name {
        b"FlateDecode" | b"Fl" => predicted(decompress_flate(data, floor, cap)?),
        b"LZWDecode" | b"LZW" => {
            let early_change = parms
                .and_then(|p| dict_get(p, b"EarlyChange"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                != 0;
            predicted(decode_lzw(data, early_change, floor, cap)?)
        }
        b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(data),
        b"ASCII85Decode" | b"A85" => decode_ascii85(data),
        b"RunLengthDecode" | b"RL" => decode_run_length(data, floor, cap),
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
fn decode_run_length(data: &[u8], floor: u64, cap: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&length) = data.get(i) {
        if out.len() as u64 >= cap {
            break;
        }
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
                    with_floor(RUN_LENGTH_LIMITS, floor)
                        .check(data.len() as u64, out.len() as u64)
                        .map_err(bomb)?;
                }
                i += 1;
            }
        }
    }
    out.truncate(usize::try_from(cap).unwrap_or(usize::MAX));
    Ok(out)
}

/// `LZWDecode` (PDF 32000-1 §7.4.4): variable-width codes from 9 to 12 bits, 256 clears
/// the table and 257 ends the data. With `early_change` (the default) the code width
/// grows one code earlier than the table strictly needs, as TIFF writers do.
fn decode_lzw(data: &[u8], early_change: bool, floor: u64, cap: u64) -> Result<Vec<u8>> {
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
        with_floor(LZW_LIMITS, floor)
            .check(data.len() as u64, out.len() as u64)
            .map_err(bomb)?;
        if out.len() as u64 >= cap {
            out.truncate(usize::try_from(cap).unwrap_or(usize::MAX));
            return Ok(out);
        }
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

fn decompress_flate(data: &[u8], floor: u64, cap: u64) -> Result<Vec<u8>> {
    // zlib first (what the specification writes), then raw deflate (some producers omit the
    // zlib header). Past its limits a stream is a bomb in any reading of it: no fallback runs.
    for zlib in [true, false] {
        match read_flate(data, zlib, floor, cap) {
            (Ok(()), output) => return Ok(output),
            (Err(e), _) if is_inflation_limit(&e) => return Err(bomb(e)),
            (Err(_), _) => {}
        }
    }

    // Lenient fallback: read as much as we can from partial/corrupt streams.
    // Some PDFs have trailing garbage, wrong /Length values, or minor stream
    // corruption. We try both zlib and raw deflate and return whatever partial
    // data was successfully decompressed.
    match decompress_flate_partial(data, floor, cap)? {
        Some(partial) => Ok(partial),
        None => Err(Error::PdfParse(
            "decompression failed: corrupt deflate stream".to_string(),
        )),
    }
}

/// Inflate `data` as zlib or as raw deflate under [`FLATE_LIMITS`], stopping at `cap` bytes: how
/// the reading ended, and the output — all of it, or what was decoded before the error.
fn read_flate(data: &[u8], zlib: bool, floor: u64, cap: u64) -> (io::Result<()>, Vec<u8>) {
    let decoder: Box<dyn Read + '_> = if zlib {
        Box::new(flate2::read::ZlibDecoder::new(data))
    } else {
        Box::new(flate2::read::DeflateDecoder::new(data))
    };
    let mut reader =
        InflateGuard::with_limits(decoder, data.len() as u64, with_floor(FLATE_LIMITS, floor))
            .take(cap);
    let mut output = Vec::new();
    let result = read_chunks(&mut reader, &mut output);
    (result, output)
}

/// Read `reader` to its end in chunks, keeping in `output` what was read before an error.
fn read_chunks<R: Read>(reader: &mut R, output: &mut Vec<u8>) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => output.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// Attempt to decompress as much data as possible from a potentially corrupt or truncated
/// flate stream: `Some(data)` with the longest output any reading of it produced before an
/// error, `None` when none produced any — and an error when a reading passed the limits.
fn decompress_flate_partial(data: &[u8], floor: u64, cap: u64) -> Result<Option<Vec<u8>>> {
    let mut best = Vec::new();
    // Both encodings, whole and with up to 8 trailing bytes stripped (some PDFs append extra
    // zeros or garbage).
    for trim in 0..=std::cmp::min(8, data.len().saturating_sub(1)) {
        let trimmed = &data[..data.len() - trim];
        for zlib in [true, false] {
            let (result, output) = read_flate(trimmed, zlib, floor, cap);
            if let Err(e) = result {
                if is_inflation_limit(&e) {
                    return Err(bomb(e));
                }
            }
            if output.len() > best.len() {
                best = output;
            }
        }
    }
    Ok((!best.is_empty()).then_some(best))
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

    // A count written out of range (negative, or so large the row length overflows) describes
    // no row layout: the data is returned as it is, as for an unknown predictor.
    let count = |key: &[u8], default: usize| match dict_get(parms, key).and_then(|o| o.as_i64()) {
        None => Some(default),
        Some(v) => usize::try_from(v).ok(),
    };
    let (Some(columns), Some(colors), Some(bits_per_component)) = (
        count(b"Columns", 1),
        count(b"Colors", 1),
        count(b"BitsPerComponent", 8),
    ) else {
        return Ok(data.to_vec());
    };
    if columns
        .checked_mul(colors)
        .and_then(|n| n.checked_mul(bits_per_component))
        .and_then(|n| n.checked_add(8))
        .is_none()
    {
        return Ok(data.to_vec());
    }

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
    let num_rows = result.len().div_ceil(row_bytes);

    for row in 0..num_rows {
        let row_start = row * row_bytes;
        let row_end = row_start.saturating_add(row_bytes).min(result.len());
        for idx in row_start + colors..row_end {
            result[idx] = result[idx].wrapping_add(result[idx - colors]);
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
    // Not one whole row (empty data divides evenly by any row length): nothing to un-filter,
    // and the row buffers below would be sized by the file's `Columns`, not by its data.
    if data.len() < input_row_len {
        return Ok(data.to_vec());
    }
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

    fn filtered(filter: &[u8], raw_data: Vec<u8>) -> PdfStream {
        let mut dict = BTreeMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(filter.to_vec()));
        PdfStream { dict, raw_data }
    }

    #[test]
    fn a_flate_bomb_is_refused_not_inflated() {
        // 120 MiB of zeros in about 120 KiB: a thousandfold, past the 100 MiB grace.
        let stream = filtered(b"FlateDecode", zlib(&vec![0u8; 120 << 20]));
        let err = decompress(&stream).unwrap_err();
        assert!(err.to_string().contains("decompression bomb"), "{err}");
        // The partial-read fallback refuses it too: a truncated bomb is still a bomb.
        let mut truncated = stream.raw_data.clone();
        truncated.truncate(truncated.len() - 4);
        assert!(decompress(&filtered(b"FlateDecode", truncated)).is_err());
    }

    #[test]
    fn an_image_that_inflates_to_its_declared_size_is_not_a_bomb_however_well_it_compresses() {
        // 128 MiB of one value in a few hundred KiB: past the ratio rule (a few-thousandfold),
        // but exactly what a large flat plot of that size holds.
        let samples = vec![200u8; 128 << 20];
        let stream = filtered(b"FlateDecode", zlib(&samples));
        assert!(
            decode(&stream).is_err(),
            "no declared size: the ratio rule refuses it"
        );
        let decoded = decode_image(&stream, samples.len() as u64).unwrap();
        assert_eq!(decoded.data.len(), samples.len());
        // A stream that inflates past what the image declares still is refused.
        assert!(decode_image(&stream, 1 << 20).is_err());
    }

    #[test]
    fn a_blank_mask_compressing_past_a_thousandfold_still_decodes() {
        // A blank 1-bit image mask: 8 MiB of zeros, under the grace size.
        let stream = filtered(b"FlateDecode", zlib(&vec![0u8; 8 << 20]));
        assert_eq!(decompress(&stream).unwrap().len(), 8 << 20);
    }

    /// LZW codes packed the way [`decode_lzw`] reads them (`EarlyChange` 1).
    fn lzw_pack(codes: &[usize]) -> Vec<u8> {
        let (mut out, mut buffer, mut bits) = (Vec::new(), 0u64, 0u32);
        let mut table_len = 258usize;
        let mut width = 9u32;
        for (index, &code) in codes.iter().enumerate() {
            buffer = (buffer << width) | code as u64;
            bits += width;
            while bits >= 8 {
                out.push((buffer >> (bits - 8)) as u8);
                bits -= 8;
            }
            if index > 0 && table_len < 4096 {
                table_len += 1;
            }
            width = match table_len + 1 {
                n if n >= 2048 => 12,
                n if n >= 1024 => 11,
                n if n >= 512 => 10,
                _ => 9,
            };
        }
        if bits > 0 {
            out.push((buffer << (8 - bits)) as u8);
        }
        out
    }

    #[test]
    fn an_lzw_bomb_is_refused_not_decoded() {
        // Each new code is the previous run plus one byte, until the table is full; then the
        // longest run (3839 bytes for 12 bits) again and again: about 110 MiB from 45 KiB.
        let mut codes: Vec<usize> = std::iter::once(0).chain(258..4096).collect();
        codes.extend(std::iter::repeat_n(4095, 30_000));
        let err = decompress(&filtered(b"LZWDecode", lzw_pack(&codes))).unwrap_err();
        assert!(err.to_string().contains("decompression bomb"), "{err}");
        // The same encoding, short of the grace size, decodes.
        let decoded = decompress(&filtered(b"LZWDecode", lzw_pack(&codes[..5000]))).unwrap();
        assert!(decoded.iter().all(|&b| b == 0) && decoded.len() > 1 << 20);
    }

    fn predictor_parms(entries: &[(&[u8], i64)]) -> PdfDict {
        entries
            .iter()
            .map(|(key, value)| (key.to_vec(), PdfObject::Integer(*value)))
            .collect()
    }

    #[test]
    fn predictor_counts_out_of_range_leave_the_data_as_it_is() {
        let data = [0u8, 1, 2, 3];
        for (key, value) in [
            (b"Columns".as_slice(), -1),
            (b"Colors".as_slice(), -4),
            (b"Columns".as_slice(), i64::MAX),
        ] {
            for predictor in [2, 12] {
                let parms = predictor_parms(&[(b"Predictor", predictor), (key, value)]);
                assert_eq!(apply_predictor(&parms, &data).unwrap(), data);
            }
        }
    }

    #[test]
    fn png_predictor_on_empty_data_does_not_size_rows_by_columns() {
        let parms = predictor_parms(&[(b"Predictor", 12), (b"Columns", 1 << 40)]);
        assert!(apply_predictor(&parms, &[]).unwrap().is_empty());
    }

    #[test]
    fn a_prefix_is_decoded_no_further_than_it_reaches() {
        let table = [[1, 2, 3, 4].as_slice(), &vec![0; 1 << 20]].concat();
        let flate = filtered(b"FlateDecode", zlib(&table));
        assert_eq!(decompress_prefix(&flate, 4).unwrap(), [1, 2, 3, 4]);
        assert_eq!(decompress(&flate).unwrap().len(), table.len());
        let rows = stream_with(
            PdfObject::Name(b"FlateDecode".to_vec()),
            Some(PdfObject::Dict(predictor_parms(&[
                (b"Predictor", 12),
                (b"Columns", 2),
            ]))),
            zlib(&[0, 1, 2, 0, 3, 4, 0, 5, 6]),
        );
        assert_eq!(decompress_prefix(&rows, 3).unwrap(), [1, 2, 3, 4]);
        let twice = stream_with(
            names(&["FlateDecode", "FlateDecode"]),
            None,
            zlib(&zlib(&table)),
        );
        assert!(decompress_prefix(&twice, 4).is_err());
        assert_eq!(decompress(&twice).unwrap().len(), table.len());
    }

    #[test]
    fn a_tiff_row_longer_than_its_data_is_rebuilt_as_far_as_the_data_goes() {
        let huge_rows = predictor_parms(&[(b"Predictor", 2), (b"Columns", 1 << 40)]);
        assert_eq!(apply_predictor(&huge_rows, &[1, 2]).unwrap(), [1, 3]);
    }

    #[test]
    fn a_prefix_cut_inside_a_filter_s_output_keeps_its_predictor_rows() {
        let parms = |predictor: i64, columns: i64| {
            Some(PdfObject::Dict(predictor_parms(&[
                (b"Predictor", predictor),
                (b"Columns", columns),
            ])))
        };
        let lzw = PdfObject::Name(b"LZWDecode".to_vec());
        let png_rows_in_entries_of_two_bytes = stream_with(
            lzw,
            parms(12, 1),
            vec![0x80, 0x00, 0x1f, 0xf0, 0x28, 0x23, 0xfe, 0x02],
        );
        assert_eq!(
            decompress_prefix(&png_rows_in_entries_of_two_bytes, 3).unwrap(),
            [255, 255, 255]
        );
        let tiff_rows_of_four = stream_with(
            PdfObject::Name(b"FlateDecode".to_vec()),
            parms(2, 4),
            zlib(&[10, 10, 10, 10, 20, 10, 10, 10]),
        );
        assert_eq!(
            decompress_prefix(&tiff_rows_of_four, 6).unwrap(),
            [10, 20, 30, 40, 20, 30]
        );
        let three_sevens = filtered(b"RunLengthDecode", vec![254, 7, 128]);
        assert_eq!(decompress_prefix(&three_sevens, 2).unwrap(), [7, 7]);
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
