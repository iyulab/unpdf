//! PDF stream decompression.

use std::io::Read;
use crate::error::{Error, Result};
use super::tokenizer::{PdfObject, PdfStream, dict_get};

/// Decompress a PDF stream based on its Filter entry.
pub fn decompress(stream: &PdfStream) -> Result<Vec<u8>> {
    let filter = dict_get(&stream.dict, b"Filter");

    match filter {
        None => Ok(stream.raw_data.clone()),
        Some(PdfObject::Name(name)) => decompress_single(name, &stream.raw_data),
        Some(PdfObject::Array(filters)) => {
            let mut data = stream.raw_data.clone();
            for f in filters {
                if let Some(name) = f.as_name() {
                    data = decompress_single(name, &data)?;
                }
            }
            Ok(data)
        }
        _ => Ok(stream.raw_data.clone()),
    }
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
    if flate2::read::ZlibDecoder::new(data).read_to_end(&mut output).is_ok() {
        return Ok(output);
    }

    // Fallback: raw deflate (some PDF producers omit zlib header)
    output.clear();
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut output)
        .map_err(|e| Error::PdfParse(format!("decompression failed: {}", e)))?;
    Ok(output)
}

fn decode_ascii_hex(data: &[u8]) -> Result<Vec<u8>> {
    let hex: String = data.iter()
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

        let stream = PdfStream { dict, raw_data: compressed };
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
        let stream = PdfStream { dict, raw_data: vec![1, 2, 3] };
        assert!(decompress(&stream).is_err());
    }
}
