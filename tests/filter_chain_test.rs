//! A stream's `/Filter` may be a chain — `[/ASCII85Decode /FlateDecode]` is what ReportLab
//! writes for every content stream when its ASCII output is on — and each filter decodes the
//! output of the one before it (PDF 32000-1 §7.4.1). Only `/FlateDecode` alone used to
//! decode: such a page came back with no text at all.
//!
//! Images take the same chain and stop at their codec: `[/ASCII85Decode /DCTDecode]` is a
//! JPEG once the ASCII85 is undone, and samples are re-encoded as PNG whichever lossless
//! filters — or none — produced them.

use std::io::Write;

use unpdf::{ParseOptions, PdfParser};

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// ASCII85 as Adobe writes it: `z` for a zero group, `~>` at the end.
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

fn stream_object(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut obj = dict.as_bytes().to_vec();
    obj.extend_from_slice(b"\nstream\n");
    obj.extend_from_slice(data);
    obj.extend_from_slice(b"\nendstream");
    obj
}

fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_start = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF\n")
            .as_bytes(),
    );
    pdf
}

/// One page of Helvetica text whose content stream is `encoded` under `filter`.
fn text_page(filter: &str, encoded: &[u8]) -> Vec<u8> {
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(
            &format!("<</Length {}/Filter {filter}>>", encoded.len()),
            encoded,
        ),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ])
}

/// One page drawing a single image XObject whose dictionary entries are `entries`.
fn image_page(entries: &str, data: &[u8]) -> Vec<u8> {
    let content = b"q 100 0 0 100 0 0 cm /Im0 Do Q\n";
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]\
          /Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        stream_object(
            &format!(
                "<</Type/XObject/Subtype/Image{entries}/Length {}>>",
                data.len()
            ),
            data,
        ),
    ])
}

fn text_of(pdf: &[u8]) -> String {
    PdfParser::from_bytes(pdf)
        .unwrap()
        .parse()
        .unwrap()
        .plain_text()
}

fn resources(pdf: &[u8]) -> Vec<unpdf::Resource> {
    let options = ParseOptions {
        extract_resources: true,
        min_image_dimension: 0,
        ..Default::default()
    };
    let doc = PdfParser::from_bytes_with_options(pdf, options)
        .unwrap()
        .parse()
        .unwrap();
    doc.resources().map(|(_, r)| r.clone()).collect()
}

const HELLO: &[u8] = b"BT /F1 24 Tf 72 720 Td (Hello ASCII85 filter chain) Tj ET";

#[test]
fn an_ascii85_then_flate_content_stream_is_read() {
    let pdf = text_page("[/ASCII85Decode /FlateDecode]", &ascii85(&zlib(HELLO)));
    assert!(text_of(&pdf).contains("Hello ASCII85 filter chain"));

    let control = text_page("/FlateDecode", &zlib(HELLO));
    assert!(text_of(&control).contains("Hello ASCII85 filter chain"));
}

#[test]
fn abbreviated_filter_names_are_read() {
    let pdf = text_page("[/A85 /Fl]", &ascii85(&zlib(HELLO)));
    assert!(text_of(&pdf).contains("Hello ASCII85 filter chain"));
}

#[test]
fn an_ascii85_wrapped_jpeg_is_handed_on_as_the_jpeg() {
    let jpeg = b"\xFF\xD8\xFF\xE0 stand-in JPEG body \xFF\xD9";
    let pdf = image_page(
        "/Width 20/Height 20/ColorSpace/DeviceRGB/BitsPerComponent 8\
         /Filter[/ASCII85Decode/DCTDecode]",
        &ascii85(jpeg),
    );
    let images = resources(&pdf);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].mime_type, "image/jpeg");
    assert_eq!(images[0].data, jpeg);
}

#[test]
fn samples_behind_a_chain_are_reencoded_as_png() {
    let gray = [0u8, 64, 128, 255];
    let pdf = image_page(
        "/Width 2/Height 2/ColorSpace/DeviceGray/BitsPerComponent 8\
         /Filter[/ASCII85Decode/FlateDecode]",
        &ascii85(&zlib(&gray)),
    );
    let images = resources(&pdf);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].mime_type, "image/png");
}

/// An image with no filter at all is samples too — as usable as a FlateDecode one.
#[test]
fn unfiltered_samples_are_reencoded_as_png() {
    let pdf = image_page(
        "/Width 2/Height 2/ColorSpace/DeviceGray/BitsPerComponent 8",
        &[0u8, 64, 128, 255],
    );
    let images = resources(&pdf);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].mime_type, "image/png");
}
