//! Shared synthetic PDF fixture builders for integration tests.
//!
//! 스캐너가 만드는 구조(전면 이미지 + 텍스트 레이어 유무)를 최소로 재현한다.
#![allow(dead_code)] // 각 테스트 파일이 필요한 빌더만 사용한다.

#[cfg(feature = "ai")]
pub mod mock_ai;

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

/// One page drawn the way a browser prints: the page opens with a flipped, scaled
/// CTM, and the text sits under a nested `cm` translation into a tall canvas. The
/// line lands mid-page (device y 500) only if `cm` is concatenated in the order
/// the PDF spec defines; the number is its own text run, as browsers emit it.
pub fn browser_printed_pdf() -> Vec<u8> {
    let content = b".24 0 0 -.24 0 842 cm \
        q 3.125 0 0 3.125 150 -46865.625 cm \
        BT /F1 16 Tf 1 0 0 -1 48 15453 Tm (Volume) Tj ET \
        BT /F1 16 Tf 1 0 0 -1 120 15453 Tm (396) Tj ET \
        Q\n";
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

/// Helvetica-Bold with its `/Widths` declared for codes 32..=122 (space and a–z;
/// everything between is 0). The widths are the font's real advances, so a producer
/// that positions each glyph by its advance lines them up exactly.
fn helvetica_bold_with_widths() -> Vec<u8> {
    let widths: Vec<String> = (32u8..=122)
        .map(|code| helvetica_bold_advance(char::from(code)).to_string())
        .collect();
    format!(
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica-Bold/FirstChar 32/LastChar 122/Widths[{}]>>",
        widths.join(" ")
    )
    .into_bytes()
}

/// Helvetica-Bold advance of a lowercase letter or space, in thousandths of an em.
pub fn helvetica_bold_advance(c: char) -> f32 {
    const LOWER: [f32; 26] = [
        556.0, 611.0, 556.0, 611.0, 556.0, 333.0, 611.0, 611.0, 278.0, 278.0, 556.0, 278.0, 889.0,
        611.0, 611.0, 611.0, 611.0, 389.0, 556.0, 333.0, 611.0, 556.0, 778.0, 556.0, 556.0, 500.0,
    ];
    match c {
        ' ' => 278.0,
        'a'..='z' => LOWER[(c as u8 - b'a') as usize],
        _ => 0.0,
    }
}

/// One page whose single line is drawn one glyph per `Tj`, each glyph moved into place
/// with `Td` by the previous glyph's advance — the way browsers print text.
///
/// With `space_glyphs`, a space is drawn as a glyph like any other; without, it is
/// left out and the word gap exists only as a larger move.
pub fn glyph_per_operator_pdf(line: &str, font_size: f32, space_glyphs: bool) -> Vec<u8> {
    glyph_per_operator_pdf_in(helvetica_bold_with_widths(), line, font_size, space_glyphs)
}

/// [`glyph_per_operator_pdf`] with Helvetica-Bold named by `/BaseFont` alone — no
/// `/FirstChar`, no `/Widths` — as a PDF before 1.5 may do for the standard 14 fonts.
/// The glyphs are still placed by the font's real advances.
pub fn glyph_per_operator_pdf_without_widths(line: &str, font_size: f32) -> Vec<u8> {
    glyph_per_operator_pdf_in(
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica-Bold>>".to_vec(),
        line,
        font_size,
        true,
    )
}

fn glyph_per_operator_pdf_in(
    font: Vec<u8>,
    line: &str,
    font_size: f32,
    space_glyphs: bool,
) -> Vec<u8> {
    let mut content = format!("BT /F1 {font_size} Tf 72 700 Td ");
    let mut pending_dx: Option<f32> = None;
    for c in line.chars() {
        if c == ' ' && !space_glyphs {
            *pending_dx.get_or_insert(0.0) += helvetica_bold_advance(' ') / 1000.0 * font_size;
            continue;
        }
        if let Some(dx) = pending_dx {
            content.push_str(&format!("{dx} 0 Td "));
        }
        content.push_str(&format!("({c}) Tj "));
        pending_dx = Some(helvetica_bold_advance(c) / 1000.0 * font_size);
    }
    content.push_str("ET\n");
    let content = content.into_bytes();
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), &content),
        font,
    ];
    assemble(objects)
}

/// One page with a single line drawn as consecutive runs, each `(base_font, text)` in
/// a standard font named by `/BaseFont` alone — no `/Encoding`, no `/Widths`, no
/// `/ToUnicode` — so each font's built-in encoding decides what its codes mean. Runs
/// are placed 30pt apart, which is wider than any run used with it.
pub fn standard_fonts_line_pdf(runs: &[(&str, &str)]) -> Vec<u8> {
    let mut content = String::from("BT 72 700 Td ");
    let mut fonts = String::new();
    let mut font_objects = Vec::new();
    for (i, (base_font, text)) in runs.iter().enumerate() {
        if i > 0 {
            content.push_str("30 0 Td ");
        }
        content.push_str(&format!("/F{i} 12 Tf ({text}) Tj "));
        fonts.push_str(&format!("/F{i} {} 0 R", 5 + i));
        font_objects
            .push(format!("<</Type/Font/Subtype/Type1/BaseFont/{base_font}>>").into_bytes());
    }
    content.push_str("ET\n");
    let content = content.into_bytes();
    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
             /Resources<</Font<<{fonts}>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream_object(&format!("<</Length {}>>", content.len()), &content),
    ];
    objects.extend(font_objects);
    assemble(objects)
}

/// One page with two body lines carrying scripts the way typeset text does: a subscript
/// lowered at the end of the first line (`C` + `4`) and a citation superscript raised at
/// the end of the second (`right.` + `[35]`), both in a smaller size. Each script sits
/// closer to the gap between the lines than a same-line tolerance allows. Each script
/// starts exactly where its line ends by Helvetica's advances (271.42 and 285.42).
pub fn scripted_lines_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 700 Td (is both an evolutionary precursor to C) Tj ET \
        BT /F1 9.6 Tf 271.42 696.8 Td (4) Tj ET \
        BT /F1 12 Tf 72 686 Td (and a useful mechanism in its own right.) Tj ET \
        BT /F1 9.6 Tf 285.42 690.2 Td ([35]) Tj ET \
        BT /F1 12 Tf 72 672 Td (The next sentence follows.) Tj ET\n";
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

/// One page of five body lines, each drawn as three runs that abut exactly — the way a
/// producer breaks a line at link or style boundaries — with the run boundaries falling
/// at the same x on every line. The runs are one paragraph, not three table columns.
pub fn fragmented_body_lines_pdf() -> Vec<u8> {
    // Every line is a rotation of the same three chunks, so the chunk widths — and so the
    // run boundaries — line up from row to row.
    const CHUNKS: [&str; 3] = ["carbon fix", "ation uses ", "light energy"];
    let width = |s: &str| s.chars().map(helvetica_bold_advance).sum::<f32>() / 1000.0 * 12.0;
    let mut content = String::new();
    for row in 0..5 {
        let y = 700 - row * 14;
        let mut x = 72.0;
        for chunk in CHUNKS {
            let text: String = chunk.chars().cycle().skip(row).take(chunk.len()).collect();
            content.push_str(&format!("BT /F1 12 Tf {x} {y} Td ({text}) Tj ET "));
            x += width(&text);
        }
    }
    content.push('\n');
    let content = content.into_bytes();
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), &content),
        helvetica_bold_with_widths(),
    ];
    assemble(objects)
}

/// One page whose single row holds a left run and a right-aligned run, the way TeX sets
/// `\hfill`: the second text object starts at the text origin right where the first run
/// ends and opens its TJ with a large offset, so its first glyph is drawn near the right
/// margin.
pub fn tj_offset_row_pdf() -> Vec<u8> {
    let left = "university of somewhere";
    let left_width: f32 = left.chars().map(helvetica_bold_advance).sum::<f32>() / 1000.0 * 10.0;
    // A full-width body line below keeps the page from reading as two columns.
    let content = format!(
        "BT /F1 10 Tf 60 600 Td ({left}) Tj ET \
         BT /F1 10 Tf {x} 600 Td [-30000 (october)] TJ ET \
         BT /F1 10 Tf 60 586 Td (a body line of plain prose that runs across the full width of the text block and on to the margin) Tj ET\n",
        x = 60.0 + left_width
    )
    .into_bytes();
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), &content),
        helvetica_bold_with_widths(),
    ];
    assemble(objects)
}

/// One page with a figure framed twice (a background box and its inset border, as
/// browsers draw it) on the left, a short separator rule far below it, and a
/// heading line in between. The frames and the rule never touch, so nothing on the
/// page is a bordered table.
pub fn figure_frame_and_rule_pdf() -> Vec<u8> {
    let content = b"1 w \
        44 465 m 177 465 l 177 636 l 44 636 l h S \
        46 467 m 174 467 l 174 633 l 46 633 l h S \
        179 100 m 356 100 l S \
        BT /F1 14 Tf 44 300 Td (Development of the concept) Tj ET\n";
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

/// One page with a bordered 2x2 grid (ruling lines drawn via `m`/`l`/`S`) and
/// real text in each cell — a lattice-mode table, not just aligned text.
pub fn bordered_table_pdf() -> Vec<u8> {
    let content = b"2 w \
        70 660 m 70 700 l S \
        170 660 m 170 700 l S \
        270 660 m 270 700 l S \
        70 700 m 270 700 l S \
        70 680 m 270 680 l S \
        70 660 m 270 660 l S \
        BT /F1 12 Tf 80 690 Td (Name) Tj ET \
        BT /F1 12 Tf 180 690 Td (Age) Tj ET \
        BT /F1 12 Tf 80 670 Td (Alice) Tj ET \
        BT /F1 12 Tf 180 670 Td (30) Tj ET\n";
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

/// One page: a lone larger-font title line followed by a two-line body
/// paragraph — the minimal shape that exercises heading promotion
/// (`LayoutAnalyzer::detect_headings`) and paragraph line-merging together.
pub fn heading_paragraph_pdf() -> Vec<u8> {
    let content = b"BT /F1 20 Tf 72 750 Td (Chapter One) Tj ET \
        BT /F1 12 Tf 72 700 Td (This is the first paragraph of the chapter.) Tj ET \
        BT /F1 12 Tf 72 685 Td (It continues on a second line of body text.) Tj ET\n";
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

/// One page: two bullet items followed by two numbered items, all at the same
/// line spacing as ordinary body text (15pt, single font size) — proving that
/// list-item detection breaks a block at each marker line on its own, not by
/// riding an incidental spacing gap the way paragraph breaks do.
pub fn list_items_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 750 Td (- First bullet item) Tj ET \
        BT /F1 12 Tf 72 735 Td (- Second bullet item) Tj ET \
        BT /F1 12 Tf 72 720 Td (1. First numbered item) Tj ET \
        BT /F1 12 Tf 72 705 Td (2. Second numbered item) Tj ET\n";
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

/// One page with two side-by-side single-line text columns, emitted
/// **right column first** in the content stream and at a **higher y** than
/// the left column — so a pipeline that merely preserved content-stream/span
/// emission order, or sorted purely by y-descending, would put the right
/// column first. Only XY-Cut column grouping (left-to-right) gets the
/// reading order right. One line per column deliberately (not two evenly
/// spaced lines each): a uniform-row-spacing pattern across two x-aligned
/// columns is itself a stream-mode table signal, and this fixture wants to
/// isolate column *ordering* from table-vs-prose classification (a separate,
/// already-covered concern — `bordered_table_pdf`).
pub fn two_column_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 350 760 Td (Right column paragraph text here.) Tj ET \
        BT /F1 12 Tf 72 700 Td (Left column paragraph text here.) Tj ET\n";
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

/// One page whose text is Adobe-Korea1 Identity-H CIDs, no embedded font
/// program and no `ToUnicode` map — only the predictive-CMap decode path
/// (`parser::cmap_table::lookup_cid`, driven by the descendant font's
/// `/CIDSystemInfo`) can resolve it. CID 1086 -> '가', CID 2001 -> '방'
/// ("가방", "bag") — verified against `cmap_table::lookup_cid("Adobe",
/// "Korea1", ..)` directly, not guessed. Mirrors the composite-font PDF
/// structure `suppression_reporting_test.rs`'s `unresolvable_composite_pdf`
/// deliberately defeats (no `CIDSystemInfo` there) — this is its readable
/// counterpart.
pub fn cjk_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 720 Td (\\004\\076\\007\\321) Tj ET\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        b"<</Type/Font/Subtype/Type0/BaseFont/Batang/Encoding/Identity-H\
          /DescendantFonts[6 0 R]>>"
            .to_vec(),
        b"<</Type/Font/Subtype/CIDFontType2/BaseFont/Batang\
          /CIDSystemInfo<</Registry(Adobe)/Ordering(Korea1)/Supplement 0>>>>"
            .to_vec(),
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

/// One page drawn as a single full-page **JPEG** image, no text operators — the
/// same shape as [`image_only_pdf`], but with a `/DCTDecode` XObject that survives
/// resource extraction (an unfiltered raw image is dropped as unsupported), so the
/// image reaches `Page::images` with bytes attached.
pub fn image_only_jpeg_pdf() -> Vec<u8> {
    let content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</XObject<</Im0 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        jpeg_image(),
    ];
    assemble(objects)
}

/// One page: two paragraphs with a small inline JPEG image XObject drawn between
/// them (not covering the page) — the "individual image on an otherwise-text page"
/// shape wiring point B targets. `extract_resources` must be enabled to get image
/// bytes into `Page::images`.
pub fn text_with_inline_image_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 750 Td (First paragraph before the image.) Tj ET \
        q 40 0 0 40 72 650 cm /Im0 Do Q \
        BT /F1 12 Tf 72 600 Td (Second paragraph after the image.) Tj ET\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>/XObject<</Im0 6 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        HELVETICA.to_vec(),
        jpeg_image(),
    ];
    assemble(objects)
}

/// `pages` pages, each with a line of text and **the same image XObject** — one shared object,
/// the way a running-header logo appears in a real document.
///
/// The duplication this is used to exercise is not in the file: the PDF holds exactly one
/// image, referenced from every page's resource dictionary. Any count above one downstream was
/// produced by extraction, not read out of the document.
pub fn repeated_logo_pdf(pages: usize) -> Vec<u8> {
    assert!(pages >= 1, "a document has at least one page");

    const FIRST_PAGE_OBJ: usize = 4;
    let font_obj = FIRST_PAGE_OBJ + pages * 2;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", FIRST_PAGE_OBJ + i * 2))
        .collect();

    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        format!("<</Type/Pages/Kids[{}]/Count {}>>", kids.join(" "), pages).into_bytes(),
        // Object 3: the one image. `/DCTDecode`, because an unfiltered sample has no
        // recognisable format and is dropped as unsupported before extraction sees it.
        jpeg_image(),
    ];

    for i in 0..pages {
        let content_obj = FIRST_PAGE_OBJ + i * 2 + 1;
        objects.push(
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Resources\
                 <</XObject<</Logo 3 0 R>>/Font<</F1 {font_obj} 0 R>>>>/Contents {content_obj} 0 R>>"
            )
            .into_bytes(),
        );
        let content = format!(
            "BT /F1 12 Tf 72 700 Td (Body text on page {}.) Tj ET\nq 100 0 0 40 20 780 cm /Logo Do Q\n",
            i + 1
        );
        objects.push(stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ));
    }
    objects.push(HELVETICA.to_vec());

    assemble(objects)
}

/// One page carrying a single AcroForm text field, `FirstName` = `John`, as a widget
/// annotation on that page.
pub fn form_pdf() -> Vec<u8> {
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R/AcroForm<</Fields[4 0 R]>>>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Annots[4 0 R]>>".to_vec(),
        b"<</Type/Annot/Subtype/Widget/FT/Tx/T(FirstName)/V(John)/Rect[72 700 272 720]>>".to_vec(),
    ];
    assemble(objects)
}

// Damaged documents. Deliberately **not** listed in `all_fixtures`, whose sweeps assert
// properties every well-formed document must have.

/// Bytes that claim `/FlateDecode` but no decoder accepts: no zlib header begins 0xFF 0xFF.
const NOT_FLATE: &[u8] = &[0xFF; 16];

/// One text page whose only content stream cannot be decoded.
pub fn undecodable_content_pdf() -> Vec<u8> {
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(
            &format!("<</Length {}/Filter/FlateDecode>>", NOT_FLATE.len()),
            NOT_FLATE,
        ),
        HELVETICA.to_vec(),
    ];
    assemble(objects)
}

/// One text page whose content is an array of two streams: the first draws "Hello World",
/// the second cannot be decoded.
pub fn partly_undecodable_content_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 720 Td (Hello World) Tj ET\n";
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]\
          /Resources<</Font<</F1 6 0 R>>>>/Contents[4 0 R 5 0 R]>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        stream_object(
            &format!("<</Length {}/Filter/FlateDecode>>", NOT_FLATE.len()),
            NOT_FLATE,
        ),
        HELVETICA.to_vec(),
    ];
    assemble(objects)
}

/// Every single-document fixture in this module, by name, for properties that must hold on
/// any well-formed document (the sweeps in `document_integrity_test` and `text_hygiene_test`).
/// Listing a new fixture here enrolls it in those sweeps.
pub fn all_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("image_only_pdf", image_only_pdf()),
        ("text_pdf", text_pdf()),
        ("bordered_table_pdf", bordered_table_pdf()),
        ("heading_paragraph_pdf", heading_paragraph_pdf()),
        ("list_items_pdf", list_items_pdf()),
        ("two_column_pdf", two_column_pdf()),
        ("cjk_pdf", cjk_pdf()),
        ("blank_pdf", blank_pdf()),
        ("mixed_pdf", mixed_pdf()),
        ("image_only_jpeg_pdf", image_only_jpeg_pdf()),
        ("text_with_inline_image_pdf", text_with_inline_image_pdf()),
        ("repeated_logo_pdf(3)", repeated_logo_pdf(3)),
        ("form_pdf", form_pdf()),
    ]
}

/// A 100×100 `/DCTDecode` image XObject. The bytes are a JPEG SOI/EOI stub, not a
/// decodable image — `unpdf` passes `/DCTDecode` data through untouched, and the
/// tests that use this never decode it either.
fn jpeg_image() -> Vec<u8> {
    let data = [0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0xFF, 0xD9];
    stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 100/Height 100/ColorSpace/DeviceGray\
              /BitsPerComponent 8/Filter/DCTDecode/Length {}>>",
            data.len()
        ),
        &data,
    )
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
