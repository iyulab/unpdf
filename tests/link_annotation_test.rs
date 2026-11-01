mod common;

use common::{assemble, stream_object};
use unpdf::parser::backend::{PdfBackend, RawBackend, ResourceScope};
use unpdf::{Document, RenderOptions};

const TARGET: &str = "https://example.test/paper";

fn font(base: &str) -> String {
    let widths: Vec<_> = (32..=126)
        .map(|code| match code {
            105 => "200",
            87 => "1000",
            _ => "500",
        })
        .collect();
    format!(
        "<</Type/Font/Subtype/Type1/BaseFont/{base}/Encoding/WinAnsiEncoding\
         /FirstChar 32/LastChar 126/Widths[{}]>>",
        widths.join(" ")
    )
}

fn fallback_font() -> Vec<u8> {
    format!(
        "<</Type/Font/Subtype/Type1/BaseFont/UnknownFace/FirstChar 0/LastChar 255/Widths[{}]>>",
        vec!["500"; 256].join(" ")
    )
    .into_bytes()
}

fn pdf(content: &str, annotations: &str, page_entries: &str, catalog_entries: &str) -> Vec<u8> {
    assemble(objects(content, annotations, page_entries, catalog_entries))
}

fn objects(
    content: &str,
    annotations: &str,
    page_entries: &str,
    catalog_entries: &str,
) -> Vec<Vec<u8>> {
    vec![
        format!("<</Type/Catalog/Pages 2 0 R{catalog_entries}>>").into_bytes(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]{page_entries}\
             /Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>/Contents 4 0 R/Annots[{annotations}]>>"
        )
        .into_bytes(),
        stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ),
        font("Helvetica").into_bytes(),
        font("Helvetica-Bold").into_bytes(),
    ]
}

fn annotation(rect: &str, target: &str) -> String {
    format!("<</Subtype/Link/Rect[{rect}]/A<</S/URI/URI({target})>>>>")
}

fn parse(pdf: &[u8]) -> Document {
    unpdf::parse_bytes(pdf).unwrap()
}

fn markdown(document: &Document) -> String {
    unpdf::render::to_markdown(document, &RenderOptions::default()).unwrap()
}

#[test]
fn a_link_uses_the_annotated_target_and_only_covered_proportional_glyphs() {
    let content = "BT /F1 10 Tf 72 700 Td (iiWWWii) Tj ET";
    let linked = parse(&pdf(content, &annotation("76 697 106 709", TARGET), "", ""));
    let plain = parse(&pdf(content, "", "", ""));
    assert_eq!(linked.plain_text(), plain.plain_text());
    let md = markdown(&linked);
    assert!(md.contains(&format!("ii[WWW]({TARGET})ii")), "{md}");
}

#[test]
fn two_targets_on_one_run_keep_the_text_between_them_unlinked() {
    let annotations = format!(
        "{} {}",
        annotation("72 697 82 709", TARGET),
        annotation("92 697 102 709", "https://example.test/second")
    );
    let document = parse(&pdf(
        "BT /F1 10 Tf 72 700 Td (AB..CD) Tj ET",
        &annotations,
        "",
        "",
    ));
    let md = markdown(&document);
    assert!(
        md.contains(&format!(
            "[AB]({TARGET})..[CD](https://example.test/second)"
        )),
        "{md}"
    );
}

#[test]
fn separate_quads_do_not_capture_the_intervening_line() {
    let annotations = format!(
        "<</Subtype/Link/Rect[72 657 110 709]\
         /QuadPoints[72 709 110 709 72 697 110 697 72 669 110 669 72 657 110 657]\
         /A<</S/URI/URI({TARGET})>>>>"
    );
    let document = parse(&pdf(
        "BT /F1 10 Tf 72 700 Td (First) Tj ET \
         BT /F1 10 Tf 72 680 Td (Middle) Tj ET \
         BT /F1 10 Tf 72 660 Td (Last) Tj ET",
        &annotations,
        "",
        "",
    ));
    let md = markdown(&document);
    assert!(md.contains(&format!("[First]({TARGET})")), "{md}");
    assert!(md.contains(&format!("[Last]({TARGET})")), "{md}");
    assert!(!md.contains("[Middle]"), "{md}");
}

#[test]
fn unusable_quads_fall_back_to_the_annotation_rectangle() {
    for points in ["0 0 1", "0 0 10 0 0 10 10 10"] {
        let annotation = format!(
            "<</Subtype/Link/Rect[110 709 72 697]/QuadPoints[{points}]\
             /A<</S/URI/URI({TARGET})>>>>"
        );
        let document = parse(&pdf(
            "BT /F1 10 Tf 72 700 Td (Label) Tj ET",
            &annotation,
            "",
            "",
        ));
        let md = markdown(&document);
        assert!(md.contains(&format!("[Label]({TARGET})")), "{md}");
    }
}

#[test]
fn one_annotation_over_several_words_and_lines_is_one_link() {
    // Each word its own text object, as many typesetters write them, and the label wrapping
    // onto a second line under a second annotation rectangle with the same target.
    let annotations = format!(
        "{} {}",
        annotation("72 697 200 709", TARGET),
        annotation("72 683 120 695", TARGET)
    );
    let md = markdown(&parse(&pdf(
        "BT /F1 10 Tf 72 700 Td (Camels) Tj ET          BT /F1 10 Tf 110 700 Td (in) Tj ET          BT /F1 10 Tf 122 700 Td (a) Tj ET          BT /F1 10 Tf 132 700 Td (changing) Tj ET          BT /F1 10 Tf 72 686 Td (climate) Tj ET",
        &annotations,
        "",
        "",
    )));
    assert!(
        md.contains(&format!("[Camels in a changing climate]({TARGET})")),
        "{md}"
    );
    assert_eq!(md.matches(TARGET).count(), 1, "{md}");
}

#[test]
fn later_annotations_win_an_overlap() {
    let annotations = format!(
        "{} {}",
        annotation("72 697 110 709", "https://example.test/earlier"),
        annotation("72 697 110 709", TARGET)
    );
    let md = markdown(&parse(&pdf(
        "BT /F1 10 Tf 72 700 Td (Label) Tj ET",
        &annotations,
        "",
        "",
    )));
    assert!(md.contains(&format!("[Label]({TARGET})")), "{md}");
    assert!(!md.contains("earlier"), "{md}");
}

#[test]
fn relative_uris_resolve_against_the_catalog_base() {
    let document = parse(&pdf(
        "BT /F1 10 Tf 72 700 Td (Label) Tj ET",
        &annotation("72 697 110 709", "../paper?q=1#part"),
        "",
        "/URI<</Base(https://example.test/reports/2026/)>>",
    ));
    let md = markdown(&document);
    assert!(
        md.contains("[Label](https://example.test/reports/paper?q=1#part)"),
        "{md}"
    );
}

#[test]
fn unsupported_actions_and_invalid_targets_preserve_plain_text() {
    for annotation in [
        "<</Subtype/Link/Rect[72 697 110 709]/Dest[3 0 R/Fit]>>",
        "<</Subtype/Link/Rect[72 697 110 709]/A<</S/GoTo/D[3 0 R/Fit]>>>>",
        "<</Subtype/Link/Rect[72 697 110 709]/A<</S/JavaScript/JS(app.alert(1))>>>>",
        "<</Subtype/Link/Rect[72 697 110 709]/A<</S/URI/URI(https://example.test/\\000bad)>>>>",
    ] {
        let linked = parse(&pdf(
            "BT /F1 10 Tf 72 700 Td (Label) Tj ET",
            annotation,
            "",
            "",
        ));
        assert_eq!(linked.plain_text().trim(), "Label");
        assert!(!markdown(&linked).contains("[Label]"));
    }
}

#[test]
fn linked_text_keeps_bold_styling_and_list_markers() {
    let document = parse(&pdf(
        "BT /F2 10 Tf 72 700 Td (- WWW) Tj ET",
        &annotation("82 697 112 709", TARGET),
        "",
        "",
    ));
    let md = markdown(&document);
    assert!(md.contains(&format!("- [**WWW**]({TARGET})")), "{md}");
}

#[test]
fn page_coordinates_keep_shifted_boxes_and_page_rotation() {
    let document = parse(&pdf(
        "BT /F1 10 Tf 172 700 Td (Label) Tj ET",
        &annotation("172 697 210 709", TARGET),
        "/CropBox[100 100 600 780]/Rotate 90",
        "",
    ));
    let md = markdown(&document);
    assert!(md.contains(&format!("[Label]({TARGET})")), "{md}");
}

#[test]
fn links_reach_json_and_streaming_with_the_same_text() {
    let bytes = pdf(
        "BT /F1 10 Tf 72 700 Td (Label) Tj ET",
        &annotation("72 697 110 709", TARGET),
        "",
        "",
    );
    let parser = unpdf::PdfParser::from_bytes(&bytes).unwrap();
    let document = parser.parse().unwrap();
    let value = serde_json::to_value(&document).unwrap();
    assert_eq!(
        value["pages"][0]["elements"][0]["content"][0]["type"],
        "link"
    );
    let mut pages = Vec::new();
    parser
        .for_each_page(unpdf::PageStreamOptions::default(), |event| {
            if let unpdf::ParseEvent::PageParsed(page) = event {
                pages.push(page);
            }
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(pages.len(), 1);
    let mut streamed = Document::new();
    streamed.add_page(pages.remove(0));
    assert_eq!(document.plain_text(), streamed.plain_text());
    assert_eq!(markdown(&document), markdown(&streamed));
}

fn with_cmap(content: &str, annotations: &str, mappings: &str, count: usize) -> Vec<u8> {
    let mut parts = objects(content, annotations, "", "");
    parts[4] = font("Helvetica")
        .replace(">>", "/ToUnicode 7 0 R>>")
        .into_bytes();
    let cmap = format!("begincmap\n1 begincodespacerange\n<00> <ff>\nendcodespacerange\n{count} beginbfchar\n{mappings}\nendbfchar\nendcmap");
    parts.push(stream_object(
        &format!("<</Length {}>>", cmap.len()),
        cmap.as_bytes(),
    ));
    assemble(parts)
}

#[test]
fn partial_links_keep_atomic_ligatures_unicode_and_sanitized_cluster_offsets() {
    for (mapping, label) in [
        ("00660069", "fi"),
        ("D83DDE00", "😀"),
        ("000000660069", "fi"),
    ] {
        let bytes = with_cmap(
            "BT /F1 10 Tf 72 700 Td (iiWii) Tj ET",
            &annotation("76 697 86 709", TARGET),
            &format!("<57> <{mapping}>"),
            1,
        );
        let document = parse(&bytes);
        assert_eq!(document.plain_text().trim(), format!("ii{label}ii"));
        assert!(!document.pages[0].link_annotations_incomplete);
        let md = markdown(&document);
        assert!(md.contains(&format!("ii[{label}]({TARGET})ii")), "{md}");
        let backend = RawBackend::load_bytes(&bytes).unwrap();
        let run = backend.decode_run(ResourceScope::page((3, 0)), b"F1", b"iiWii");
        let decoded = backend.decode_text(ResourceScope::page((3, 0)), b"F1", b"iiWii");
        assert_eq!(run.decoded.text, decoded.text);
        assert_eq!(run.decoded.suppressed, decoded.suppressed);
        assert_eq!(run.decoded.undecoded_codes, decoded.undecoded_codes);
        assert_eq!(
            run.glyphs,
            backend.glyph_advances(ResourceScope::page((3, 0)), b"F1", b"iiWii")
        );
        let cluster = run
            .clusters
            .as_ref()
            .unwrap()
            .iter()
            .find(|c| run.decoded.text.get(c.text.clone()) == Some(label))
            .unwrap();
        assert_eq!(cluster.glyphs, 2..3);
    }
}

#[test]
fn a_source_code_without_text_keeps_its_advance_before_the_linked_cluster() {
    let document = parse(&with_cmap(
        "BT /F1 10 Tf 72 700 Td (ABC) Tj ET",
        &annotation("82 697 87 709", TARGET),
        "<41> <0041>\n<42> <0000>\n<43> <00660069>",
        3,
    ));
    let md = markdown(&document);
    assert!(md.contains(&format!("A[fi]({TARGET})")), "{md}");
    assert_eq!(document.plain_text().trim(), "Afi");
}

#[test]
fn odd_identity_h_tails_and_malformed_unicode_units_preserve_existing_decode_results() {
    let cmap = "begincmap\n1 begincodespacerange\n<0000> <ffff>\nendcodespacerange\n1 beginbfchar\n<0041> <0041>\nendbfchar\nendcmap";
    let mut parts = objects("", "", "", "");
    parts[4] = b"<</Type/Font/Subtype/Type0/BaseFont/Test/Encoding/Identity-H/ToUnicode 7 0 R/DescendantFonts[8 0 R]>>".to_vec();
    parts.push(stream_object(
        &format!("<</Length {}>>", cmap.len()),
        cmap.as_bytes(),
    ));
    parts.push(b"<</Type/Font/Subtype/CIDFontType0/BaseFont/Test/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>/DW 500>>".to_vec());
    let backend = RawBackend::load_bytes(&assemble(parts)).unwrap();
    let source = [0, 65, 0];
    let scope = ResourceScope::page((3, 0));
    let run = backend.decode_run(scope, b"F1", &source);
    let decoded = backend.decode_text(scope, b"F1", &source);
    assert_eq!(run.decoded.text, decoded.text);
    assert_eq!(run.decoded.undecoded_codes, decoded.undecoded_codes);
    assert_eq!(run.glyphs, backend.glyph_advances(scope, b"F1", &source));
    assert_eq!(run.clusters.unwrap().last().unwrap().glyphs, 1..2);

    for mapping in ["d800", "dc00", "006100", ""] {
        let bytes = with_cmap("", "", &format!("<57> <{mapping}>"), 1);
        let backend = RawBackend::load_bytes(&bytes).unwrap();
        let run = backend.decode_run(scope, b"F1", b"iWi");
        let decoded = backend.decode_text(scope, b"F1", b"iWi");
        assert_eq!(run.decoded.text, decoded.text, "{mapping}");
        assert_eq!(run.decoded.suppressed, decoded.suppressed);
        assert_eq!(run.glyphs, backend.glyph_advances(scope, b"F1", b"iWi"));
    }
}

#[test]
fn unavailable_partial_mapping_is_reported_without_guessing_or_changing_text() {
    for (rect, incomplete) in [("72 697 82 709", true), ("72 697 110 709", false)] {
        let mut parts = objects(
            "BT /F1 10 Tf 72 700 Td <c3a958> Tj ET",
            &annotation(rect, TARGET),
            "",
            "",
        );
        parts[4] = fallback_font();
        let document = parse(&assemble(parts));
        assert_eq!(document.plain_text().trim(), "éX");
        assert_eq!(document.pages[0].link_annotations_incomplete, incomplete);
        assert_eq!(markdown(&document).contains("[éX]"), !incomplete);
    }
}

#[test]
fn hidden_and_noview_links_are_inactive_but_invisible_alone_does_not_disable_a_link() {
    for (flags, active) in [(0, true), (1, true), (2, false), (32, false), (34, false)] {
        let annot =
            format!("<</Subtype/Link/F {flags}/Rect[72 697 110 709]/A<</S/URI/URI({TARGET})>>>>");
        let document = parse(&pdf("BT /F1 10 Tf 72 700 Td (Label) Tj ET", &annot, "", ""));
        assert_eq!(document.plain_text().trim(), "Label");
        assert_eq!(
            markdown(&document).contains("[Label]"),
            active,
            "flags={flags}"
        );
        assert!(!document.pages[0].link_annotations_incomplete);
    }
}

#[test]
fn indirect_annotation_action_and_uri_objects_are_read() {
    let mut parts = objects("BT /F1 10 Tf 72 700 Td (Label) Tj ET", "7 0 R", "", "");
    parts.extend([
        b"<</Subtype/Link/Rect[72 697 110 709]/A 8 0 R>>".to_vec(),
        b"<</S/URI/URI 9 0 R>>".to_vec(),
        format!("({TARGET})").into_bytes(),
    ]);
    assert!(markdown(&parse(&assemble(parts))).contains(&format!("[Label]({TARGET})")));
}

#[test]
fn transformed_form_text_uses_page_coordinates() {
    let content = "q 1 0 0 1 100 680 cm /Fm1 Do Q";
    let mut parts = objects(content, &annotation("100 697 138 709", TARGET), "", "");
    parts[2] = String::from_utf8(parts[2].clone())
        .unwrap()
        .replace("/F2 6 0 R>>", "/F2 6 0 R>>/XObject<</Fm1 7 0 R>>")
        .into_bytes();
    let form = "BT /F1 10 Tf 0 20 Td (Label) Tj ET";
    parts.push(stream_object(&format!("<</Type/XObject/Subtype/Form/BBox[0 0 100 50]/Resources<</Font<</F1 5 0 R>>>>/Length {}>>", form.len()), form.as_bytes()));
    let document = parse(&assemble(parts));
    assert_eq!(document.plain_text().trim(), "Label");
    assert!(markdown(&document).contains(&format!("[Label]({TARGET})")));
}

#[test]
fn rotated_text_and_clipped_prefixes_keep_exact_link_ownership() {
    let document = parse(&pdf(
        "BT /F1 10 Tf 0 1 -1 0 200 500 Tm (Label) Tj ET",
        &annotation("190 500 210 530", TARGET),
        "",
        "",
    ));
    assert!(markdown(&document).contains(&format!("[Label]({TARGET})")));
    let document = parse(&pdf(
        "q 82 690 100 30 re W n BT /F1 10 Tf 72 700 Td (ABLabel) Tj ET Q",
        &annotation("82 697 120 709", TARGET),
        "",
        "",
    ));
    assert!(markdown(&document).contains(&format!("AB[Label]({TARGET})")));
}

#[test]
fn rtl_reordering_and_presentation_form_expansion_keep_their_uri_owner() {
    for (mapping, source, rect, expected) in [
        ("<41> <05d0>\n<42> <05d1>", "xxABzz", "82 697 92 709", "בא"),
        ("<41> <fefb>", "xxAzz", "82 697 87 709", "لا"),
    ] {
        let content = format!("BT /F1 10 Tf 72 700 Td ({source}) Tj ET");
        let count = if source.contains('B') { 2 } else { 1 };
        let linked = parse(&with_cmap(
            &content,
            &annotation(rect, TARGET),
            mapping,
            count,
        ));
        let plain = parse(&with_cmap(&content, "", mapping, count));
        assert_eq!(linked.plain_text(), plain.plain_text());
        let md = markdown(&linked);
        assert!(md.contains(&format!("[{expected}]({TARGET})")), "{md}");
    }
}

#[test]
fn annotation_scan_and_index_limits_clear_all_links_and_survive_reparse() {
    let content = "BT /F1 10 Tf 72 700 Td (Label) Tj ET";
    let plain = parse(&pdf(content, "", "", "")).plain_text();
    for annotations in [
        format!(
            "{}{}",
            annotation("72 697 110 709", TARGET),
            " null".repeat(4096)
        ),
        annotation("0 0 612 792", TARGET).repeat(257),
    ] {
        let document = parse(&pdf(content, &annotations, "", ""));
        assert_eq!(document.plain_text(), plain);
        assert!(document.pages[0].link_annotations_incomplete);
        assert!(!markdown(&document).contains("[Label]"));
        let value = serde_json::to_value(&document.pages[0]).unwrap();
        assert_eq!(value["link_annotations_incomplete"], true);
    }
}

#[test]
fn geometry_work_is_bounded_even_when_every_candidate_misses() {
    let content = "BT /F1 10 Tf 72 700 Td (X) Tj ET ".repeat(400);
    let annotations = annotation("72 700 73 701", TARGET).repeat(3000);
    let document = parse(&pdf(&content, &annotations, "", ""));
    assert!(document.pages[0].link_annotations_incomplete);
    assert_eq!(
        document.plain_text(),
        parse(&pdf(&content, "", "", "")).plain_text()
    );
    assert!(!markdown(&document).contains(TARGET));
}

#[test]
fn a_page_without_annotations_keeps_zero_default_json_and_older_link_styles() {
    let document = parse(&pdf("BT /F1 10 Tf 72 700 Td (Label) Tj ET", "", "", ""));
    let value = serde_json::to_value(&document.pages[0]).unwrap();
    assert!(value.get("link_annotations_incomplete").is_none());
    let restored: unpdf::model::Page = serde_json::from_value(value).unwrap();
    assert!(!restored.link_annotations_incomplete);
    let link: unpdf::model::InlineContent = serde_json::from_value(serde_json::json!({
        "type":"link", "text":"Label", "url":TARGET, "title":null,
    }))
    .unwrap();
    assert!(
        matches!(link, unpdf::model::InlineContent::Link { style, .. } if !style.has_styling())
    );
}

#[test]
fn headings_and_both_native_table_builders_keep_annotated_labels() {
    let document = parse(&pdf(
        "BT /F2 20 Tf 72 730 Td (Study) Tj ET BT /F1 10 Tf 72 700 Td (Some ordinary body text on the page.) Tj ET",
        &annotation("72 725 130 750", TARGET), "", ""));
    let md = markdown(&document);
    assert!(
        md.contains("# ") && md.contains(&format!("[**Study**]({TARGET})")),
        "{md}"
    );
    let content = "BT /F1 12 Tf 80 690 Td (Name) Tj ET BT /F1 12 Tf 180 690 Td (Age) Tj ET \
        BT /F1 12 Tf 80 670 Td (Alice) Tj ET BT /F1 12 Tf 180 670 Td (30) Tj ET \
        BT /F1 12 Tf 80 650 Td (Bob) Tj ET BT /F1 12 Tf 180 650 Td (28) Tj ET";
    let rules = "2 w 70 640 m 70 700 l S 170 640 m 170 700 l S 270 640 m 270 700 l S \
        70 700 m 270 700 l S 70 680 m 270 680 l S 70 660 m 270 660 l S 70 640 m 270 640 l S ";
    for rules in ["", rules] {
        let document = parse(&pdf(
            &format!("{rules}{content}"),
            &annotation("80 687 110 701", TARGET),
            "",
            "",
        ));
        assert!(document.pages[0]
            .elements
            .iter()
            .any(|b| matches!(b, unpdf::model::Block::Table(_))));
        let md = markdown(&document);
        assert!(md.contains(&format!("[Name]({TARGET})")), "{md}");
    }
}

#[test]
fn a_skewed_quad_does_not_link_text_only_inside_its_bounding_rectangle() {
    let annotations = format!("<</Subtype/Link/Rect[72 698 112 730]/QuadPoints[72 710 112 730 72 698 112 718]/A<</S/URI/URI({TARGET})>>>>");
    let document = parse(&pdf(
        "BT /F1 10 Tf 72 700 Td (A) Tj ET BT /F1 10 Tf 100 700 Td (B) Tj ET",
        &annotations,
        "",
        "",
    ));
    let md = markdown(&document);
    assert!(md.contains(&format!("[A]({TARGET}) B")), "{md}");
    assert!(!md.contains("[B]"));
}

#[test]
fn vertical_glyph_origins_locate_the_linked_character() {
    let mut parts = objects(
        "BT /F1 10 Tf 100 700 Td <00410042> Tj ET",
        &annotation("95 680 105 688", TARGET),
        "",
        "",
    );
    parts[4] = b"<</Type/Font/Subtype/Type0/BaseFont/Test/Encoding/Identity-V/ToUnicode 7 0 R/DescendantFonts[8 0 R]>>".to_vec();
    let cmap = "begincmap\n1 begincodespacerange\n<0000> <ffff>\nendcodespacerange\n2 beginbfchar\n<0041> <0041>\n<0042> <0042>\nendbfchar\nendcmap";
    parts.push(stream_object(
        &format!("<</Length {}>>", cmap.len()),
        cmap.as_bytes(),
    ));
    parts.push(b"<</Type/Font/Subtype/CIDFontType0/BaseFont/Test/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>/DW 500/DW2[880 -1000]>>".to_vec());
    let document = parse(&assemble(parts));
    assert_eq!(document.plain_text().trim(), "AB");
    let md = markdown(&document);
    assert!(md.contains(&format!("A[B]({TARGET})")), "{md}");
}

#[test]
fn shared_targets_and_final_output_are_bounded_independently() {
    let mut content = String::new();
    let mut annotations = String::new();
    for row in 0..4 {
        for column in 0..40 {
            let x = 72 + column * 12;
            let y = 700 - row * 30;
            // An unlinked `|` after each linked `X` keeps the 160 links apart: links to one
            // target separated only by spaces would come out as one.
            content.push_str(&format!(
                "BT /F1 10 Tf {x} {y} Td (X) Tj ET BT /F1 10 Tf {} {y} Td (|) Tj ET ",
                x + 8
            ));
            annotations.push_str(&format!(
                "<</Subtype/Link/Rect[{x} {} {} {}]/A<</S/URI/URI 7 0 R>>>> ",
                y - 3,
                x + 5,
                y + 9
            ));
        }
    }
    let target = format!("https://example.test/{}", "a".repeat(8000));
    let mut parts = objects(&content, &annotations, "", "");
    parts.push(format!("({target})").into_bytes());
    let bytes = assemble(parts);
    let backend = RawBackend::load_bytes(&bytes).unwrap();
    let annotations = backend.link_annotations((3, 0));
    assert_eq!(annotations.annotations.len(), 160);
    assert!(!annotations.incomplete);
    assert!(std::sync::Arc::ptr_eq(
        &annotations.annotations[0].target,
        &annotations.annotations[159].target
    ));
    let document = parse(&bytes);
    assert!(document.pages[0].link_annotations_incomplete);
    assert_eq!(
        document.plain_text(),
        parse(&pdf(&content, "", "", "")).plain_text()
    );
    let rendered = markdown(&document);
    let emitted = rendered.matches("https://example.test/").count();
    assert!(emitted > 0 && emitted < 160);
    assert!(emitted * target.len() <= 1024 * 1024);
}

#[test]
fn incomplete_state_survives_internal_retries_without_leaking_to_other_pages_or_streams() {
    let annotations = [
        "null ".repeat(4097),
        annotation("72 697 110 709", TARGET),
        String::new(),
    ];
    let content = "BT /F1 10 Tf 72 700 Td (Label) Tj ET";
    let mut parts = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R 5 0 R 7 0 R]/Count 3>>".to_vec(),
    ];
    for (i, annotations) in annotations.iter().enumerate() {
        parts.push(format!("<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Resources<</Font<</F1 9 0 R>>>>/Contents {} 0 R/Annots[{annotations}]>>", 4 + 2 * i).into_bytes());
        parts.push(stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ));
    }
    parts.push(font("Helvetica").into_bytes());
    let bytes = assemble(parts);
    let parser = unpdf::PdfParser::from_bytes(&bytes).unwrap();
    let document = parser.parse().unwrap();
    let expected = [true, false, false];
    assert_eq!(
        document
            .pages
            .iter()
            .map(|p| p.link_annotations_incomplete)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(markdown(&document).contains(&format!("[Label]({TARGET})")));
    let mut streamed = Vec::new();
    parser
        .for_each_page(unpdf::PageStreamOptions::default(), |event| {
            if let unpdf::ParseEvent::PageParsed(page) = event {
                streamed.push(page.link_annotations_incomplete);
            }
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(streamed, expected);
    let backend = RawBackend::load_bytes(&bytes).unwrap();
    let mut analyzer = unpdf::parser::LayoutAnalyzer::new(&backend);
    for (i, expected) in expected.into_iter().enumerate() {
        analyzer.extract_page_blocks(i as u32 + 1).unwrap();
        assert_eq!(analyzer.page_facts().link_annotations_incomplete, expected);
    }
}

#[test]
fn code_keeps_its_text_and_reports_that_its_links_cannot_be_represented() {
    let mut parts = objects(
        "BT /F1 10 Tf 72 700 Td (Label) Tj ET",
        &annotation("72 697 110 709", TARGET),
        "",
        "",
    );
    parts[4] = font("Courier").into_bytes();
    let document = parse(&assemble(parts));
    assert!(document.pages[0].link_annotations_incomplete);
    assert_eq!(document.plain_text().trim(), "Label");
    let md = markdown(&document);
    assert!(md.contains("```"), "{md}");
    assert!(!md.contains(TARGET));
}

#[test]
fn review_regression_rotated_labels_keep_their_original_emphasis() {
    let document = parse(&pdf(
        "BT /F2 10 Tf 0 1 -1 0 200 500 Tm (Alpha) Tj /F1 10 Tf (Beta) Tj ET",
        &annotation("190 500 210 525", TARGET),
        "",
        "",
    ));
    let md = markdown(&document);
    assert!(md.contains(&format!("[**Alpha**]({TARGET})Beta")), "{md}");
    assert_eq!(document.plain_text().trim(), "AlphaBeta");
}

#[test]
fn review_regression_character_spacing_never_splits_an_atomic_ligature() {
    for content in [
        "BT /F1 10 Tf 2 Tc 72 700 Td [<0102> 200 <03>] TJ ET",
        "BT /F1 10 Tf 2 Tc 72 700 Td <0102> Tj ET BT /F1 10 Tf 0 Tc 90 700 Td <030303> Tj ET",
    ] {
        let mut linked = objects(content, &annotation("72 697 84 709", TARGET), "", "");
        let mut plain = objects(content, "", "", "");
        let cmap = "begincmap\n1 begincodespacerange\n<00> <ff>\nendcodespacerange\n3 beginbfchar\n<01> <00660069>\n<02> <0000>\n<03> <0058>\nendbfchar\nendcmap";
        for parts in [&mut linked, &mut plain] {
            parts[4] = b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/FirstChar 1/LastChar 3/Widths[1000 0 500]/ToUnicode 7 0 R>>".to_vec();
            parts.push(stream_object(
                &format!("<</Length {}>>", cmap.len()),
                cmap.as_bytes(),
            ));
        }
        let linked = parse(&assemble(linked));
        let plain = parse(&assemble(plain));
        assert_eq!(linked.plain_text(), plain.plain_text());
        assert!(
            !linked.plain_text().contains("f i"),
            "{}",
            linked.plain_text()
        );
        let md = markdown(&linked);
        assert!(md.contains(&format!("[fi]({TARGET})")), "{md}");
    }
}

#[test]
fn known_unrepresentable_associations_keep_independently_proven_links() {
    for base in ["Courier", "UnknownFace"] {
        let label = if base == "Courier" {
            "(Label)"
        } else {
            "<c3a958>"
        };
        let content =
            format!("BT /F1 10 Tf 72 700 Td {label} Tj ET BT /F2 10 Tf 72 650 Td (Prose) Tj ET");
        let unsupported = if base == "Courier" {
            "72 697 110 709"
        } else {
            "72 697 82 709"
        };
        let annotations = format!(
            "{} {}",
            annotation(unsupported, "https://example.test/limited"),
            annotation("72 647 110 659", TARGET)
        );
        let mut linked = objects(&content, &annotations, "", "");
        let mut plain = objects(&content, "", "", "");
        for parts in [&mut linked, &mut plain] {
            parts[4] = if base == "Courier" {
                font(base)
                    .replace("/Encoding/WinAnsiEncoding", "")
                    .into_bytes()
            } else {
                fallback_font()
            };
        }
        let linked = parse(&assemble(linked));
        let plain = parse(&assemble(plain));
        assert!(linked.pages[0].link_annotations_incomplete);
        assert_eq!(linked.plain_text(), plain.plain_text());
        let md = markdown(&linked);
        assert!(
            md.contains(&format!("[**Prose**]({TARGET})")),
            "{base}: {md}"
        );
        assert!(!md.contains("https://example.test/limited"), "{md}");
    }
}
