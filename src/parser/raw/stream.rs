//! PDF stream decompression.

use super::tokenizer::{dict_get, PdfDict, PdfObject, PdfStream};
use crate::error::{Error, Result};
use std::io::Read;

/// Decompress a PDF stream based on its Filter entry.
/// Also applies predictor decoding from DecodeParms if present.
pub fn decompress(stream: &PdfStream) -> Result<Vec<u8>> {
    let filter = dict_get(&stream.dict, b"Filter");

    let decompressed = match filter {
        None => return Ok(stream.raw_data.clone()),
        Some(PdfObject::Name(name)) => decompress_single(name, &stream.raw_data)?,
        Some(PdfObject::Array(filters)) => {
            let mut data = stream.raw_data.clone();
            for f in filters {
                if let Some(name) = f.as_name() {
                    data = decompress_single(name, &data)?;
                }
            }
            data
        }
        _ => return Ok(stream.raw_data.clone()),
    };

    // Apply predictor decoding if DecodeParms is present
    let decode_parms = dict_get(&stream.dict, b"DecodeParms");
    if let Some(parms) = decode_parms {
        if let Some(parms_dict) = parms.as_dict() {
            return apply_predictor(parms_dict, &decompressed);
        }
    }

    Ok(decompressed)
}

fn decompress_single(filter_name: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    match filter_name {
        b"FlateDecode" | b"Fl" => decompress_flate(data),
        b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(data),
        _ => Err(Error::PdfParse(format!(
            "unsupported filter: {}",
            String::from_utf8_lossy(filter_name)
        ))),
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
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut output)
        .map_err(|e| Error::PdfParse(format!("decompression failed: {}", e)))?;
    Ok(output)
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

    if input_row_len == 0 || data.len() % input_row_len != 0 {
        // If data doesn't divide evenly, try using columns directly as row_bytes
        // (common when Columns already accounts for all bytes per row)
        let alt_row_bytes = columns;
        let alt_input_row_len = 1 + alt_row_bytes;
        if alt_input_row_len > 0 && data.len() % alt_input_row_len == 0 {
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
    use std::collections::HashMap;

    #[test]
    fn test_decompress_uncompressed() {
        let stream = PdfStream {
            dict: HashMap::new(),
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

        let mut dict = HashMap::new();
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
        let mut dict = HashMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"LZWDecode".to_vec()));
        let stream = PdfStream {
            dict,
            raw_data: vec![1, 2, 3],
        };
        assert!(decompress(&stream).is_err());
    }
}
