//! `/FlateDecode` embedded images are re-encoded as PNG rather than unconditionally
//! dropped: gray, RGB, CMYK (converted to RGB) and Indexed images (looked up in their palette),
//! including `ICCBased` resolved by component count, at 1 to 16 bits per component. A color
//! space outside that (`Lab`, `Separation`, `DeviceN`) is still dropped, but counted as an
//! "unsupported image" quality signal instead of silently vanishing.

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

    let decoder = png::Decoder::new(std::io::Cursor::new(resource.data.as_slice()));
    let mut reader = decoder
        .read_info()
        .expect("valid PNG produced by the re-encoder");
    let mut buf = vec![
        0u8;
        reader
            .output_buffer_size()
            .expect("PNG dimensions fit in a buffer")
    ];
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

/// The single PNG resource `bytes` yields, decoded: (colour type, pixels).
fn only_png(bytes: &[u8]) -> (png::ColorType, Vec<u8>) {
    let options = ParseOptions {
        extract_resources: true,
        min_image_dimension: 0,
        ..Default::default()
    };
    let doc = PdfParser::from_bytes_with_options(bytes, options)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(doc.extraction_quality.unsupported_image_count, 0);
    assert_eq!(doc.resources.len(), 1);
    let resource = doc.resources.values().next().unwrap();
    assert_eq!(resource.mime_type, "image/png");

    let decoder = png::Decoder::new(std::io::Cursor::new(resource.data.as_slice()));
    let mut reader = decoder.read_info().expect("valid PNG");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("fits")];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.color_type, buf)
}

#[test]
fn flatedecode_cmyk_image_is_converted_to_rgb() {
    // cyan, magenta / yellow, black
    let samples = [255u8, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255];
    let compressed = deflate(&samples);
    let image_obj = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceCMYK\
              /BitsPerComponent 8/Filter/FlateDecode/Length {}>>",
            compressed.len()
        ),
        &compressed,
    );

    let (color_type, pixels) = only_png(&one_page_with_image(image_obj, None));
    assert_eq!(color_type, png::ColorType::Rgb);
    assert_eq!(pixels, [0, 255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0]);
}

#[test]
fn flatedecode_indexed_image_is_looked_up_in_its_palette() {
    // 1-bit indices into a two-entry RGB palette held in a hex string: 0 = red, 1 = blue.
    let samples = [0b0100_0000u8, 0b1000_0000];
    let compressed = deflate(&samples);
    let image_obj = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2\
              /ColorSpace[/Indexed/DeviceRGB 1<FF00000000FF>]\
              /BitsPerComponent 1/Filter/FlateDecode/Length {}>>",
            compressed.len()
        ),
        &compressed,
    );

    let (color_type, pixels) = only_png(&one_page_with_image(image_obj, None));
    assert_eq!(color_type, png::ColorType::Rgb);
    assert_eq!(pixels, [255, 0, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0]);
}

#[test]
fn flatedecode_unsupported_colorspace_is_dropped_and_counted() {
    // Lab is not converted -- 3 bytes/pixel, 2x2 = 12 bytes.
    let pixels = vec![0u8; 12];
    let compressed = deflate(&pixels);
    let image_obj = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2\
              /ColorSpace[/Lab<</WhitePoint[0.9505 1 1.089]>>]\
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
