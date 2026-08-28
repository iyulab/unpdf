//! `/FlateDecode` embedded images are re-encoded as PNG rather than unconditionally dropped —
//! docket `#125`'s companion gap
//! (`claudedocs/unpdf/issues/ISSUE-unpdf-20260828-123513-flatedecode-images-unconditionally-dropped.md`
//! in the umbrella repo). Stage 1 scope: 8-bit `DeviceGray`/`DeviceRGB` (including `ICCBased`
//! resolved to an equivalent component count), no `/DecodeParms` predictor beyond what the
//! existing stream decompressor already reverses. Anything outside that scope must still be
//! dropped (unchanged behavior) but counted as an "unsupported image" quality signal instead of
//! silently vanishing.

use std::io::Write;

use unpdf::{ParseOptions, PdfParser};

fn deflate(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn stream_object(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut obj = dict.as_bytes().to_vec();
    obj.extend_from_slice(b"\nstream\n");
    obj.extend_from_slice(data);
    obj.extend_from_slice(b"\nendstream");
    obj
}

fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
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

/// One page whose sole resource is a FlateDecode image XObject built from `image_obj`
/// (object 5) — with an optional extra object 6 (an `ICCBased` stream, when present).
fn one_page_with_image(image_obj: Vec<u8>, extra_obj: Option<Vec<u8>>) -> Vec<u8> {
    let content = b"q 100 0 0 100 0 0 cm /Im0 Do Q\n";
    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]\
          /Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        image_obj,
    ];
    if let Some(extra) = extra_obj {
        objects.push(extra);
    }
    assemble(objects)
}

fn rgb_2x2_pixels() -> Vec<u8> {
    vec![
        255, 0, 0, 0, 255, 0, // row 0: red, green
        0, 0, 255, 255, 255, 255, // row 1: blue, white
    ]
}

#[test]
fn flatedecode_devicergb_image_is_reencoded_as_png() {
    let pixels = rgb_2x2_pixels();
    let compressed = deflate(&pixels);
    let image_obj = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceRGB\
              /BitsPerComponent 8/Filter/FlateDecode/Length {}>>",
            compressed.len()
        ),
        &compressed,
    );
    let bytes = one_page_with_image(image_obj, None);

    let options = ParseOptions {
        extract_resources: true,
        min_image_dimension: 0,
        ..Default::default()
    };
    let doc = PdfParser::from_bytes_with_options(&bytes, options)
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(
        doc.resources.len(),
        1,
        "a FlateDecode DeviceRGB image is a reconstructable PNG, not a raw drop"
    );
    let resource = doc.resources.values().next().unwrap();
    assert_eq!(resource.mime_type, "image/png");

    let decoder = png::Decoder::new(resource.data.as_slice());
    let mut reader = decoder
        .read_info()
        .expect("valid PNG produced by the re-encoder");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    assert_eq!(buf, pixels);
    assert_eq!(doc.extraction_quality.unsupported_image_count, 0);
}

#[test]
fn flatedecode_iccbased_rgb_image_resolves_component_count_and_reencodes() {
    let pixels = rgb_2x2_pixels();
    let compressed = deflate(&pixels);
    let image_obj = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace[/ICCBased 6 0 R]\
              /BitsPerComponent 8/Filter/FlateDecode/Length {}>>",
            compressed.len()
        ),
        &compressed,
    );
    // A stand-in ICC profile stream — only `/N` (component count) is ever read.
    let icc_profile = stream_object("<</N 3/Alternate/DeviceRGB/Length 4>>", b"\0\0\0\0");
    let bytes = one_page_with_image(image_obj, Some(icc_profile));

    let options = ParseOptions {
        extract_resources: true,
        min_image_dimension: 0,
        ..Default::default()
    };
    let doc = PdfParser::from_bytes_with_options(&bytes, options)
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(
        doc.resources.len(),
        1,
        "ICCBased with N=3 is component-count-equivalent to DeviceRGB"
    );
    assert_eq!(
        doc.resources.values().next().unwrap().mime_type,
        "image/png"
    );
}

#[test]
fn flatedecode_unsupported_colorspace_is_dropped_and_counted() {
    // DeviceCMYK is out of stage-1 scope — 4 bytes/pixel, 2x2 = 16 bytes.
    let pixels = vec![0u8; 16];
    let compressed = deflate(&pixels);
    let image_obj = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceCMYK\
              /BitsPerComponent 8/Filter/FlateDecode/Length {}>>",
            compressed.len()
        ),
        &compressed,
    );
    let bytes = one_page_with_image(image_obj, None);

    let options = ParseOptions {
        extract_resources: true,
        min_image_dimension: 0,
        ..Default::default()
    };
    let doc = PdfParser::from_bytes_with_options(&bytes, options)
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(
        doc.resources.len(),
        0,
        "out-of-scope color spaces are still dropped, same as before"
    );
    assert_eq!(
        doc.extraction_quality.unsupported_image_count, 1,
        "but the drop must be visible as a quality signal, not silent"
    );
    let warning = doc.extraction_quality.warning_message();
    assert!(
        warning.is_some_and(|w| w.to_lowercase().contains("image")),
        "warning_message() should mention the unsupported image"
    );
}
