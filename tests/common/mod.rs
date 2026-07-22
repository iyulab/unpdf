//! Shared synthetic PDF fixture builders for integration tests.
//!
//! 스캐너가 만드는 구조(전면 이미지 + 텍스트 레이어 유무)를 최소로 재현한다.
#![allow(dead_code)] // 각 테스트 파일이 필요한 빌더만 사용한다.

const HELVETICA: &[u8] = b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>";

/// One page drawn as a single full-page image, no text operators at all.
pub fn image_only_pdf() -> Vec<u8> {
    let content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        gray_pixel_image(),
    ];
    assemble(objects)
}

/// One page with a single line of visible Helvetica text.
pub fn text_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 720 Td (Hello World) Tj ET\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        HELVETICA.to_vec(),
    ];
    assemble(objects)
}

/// One page whose content stream paints nothing.
pub fn blank_pdf() -> Vec<u8> {
    let content = b"q Q\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Contents 4 0 R>>".to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
    ];
    assemble(objects)
}

/// Two pages: page 1 text, page 2 image-only.
pub fn mixed_pdf() -> Vec<u8> {
    let text_content = b"BT /F1 12 Tf 72 720 Td (Hello World) Tj ET\n";
    let image_content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R 5 0 R]/Count 2>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 7 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", text_content.len()), text_content),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</XObject<</Im0 8 0 R>>>>/Contents 6 0 R>>"
            .to_vec(),
        stream_object(
            &format!("<</Length {}>>", image_content.len()),
            image_content,
        ),
        HELVETICA.to_vec(),
        gray_pixel_image(),
    ];
    assemble(objects)
}

/// A 1×1 grey image XObject — the CTM it is drawn with does the scaling.
fn gray_pixel_image() -> Vec<u8> {
    stream_object(
        "<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray\
          /BitsPerComponent 8/Length 1>>",
        &[0x80u8],
    )
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
