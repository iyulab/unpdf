//! A page's running heads and feet are kept unless the caller asks for them to go, and then
//! only what stands as furniture is omitted: a footnote at the foot, a title and the body stay.

mod common;

use unpdf::render::{to_markdown, RenderOptions};
use unpdf::{parse_bytes_with_options, ParseOptions};

const BODY: [&str; 8] = [
    "The committee reviewed the proposal in detail and agreed that the schedule should hold.",
    "Members asked several questions about the budget, and the chair summarised the main points.",
    "A second reading will take place after the changes to the appendix have been circulated.",
    "The secretary noted that two of the reviewers had not yet returned their written comments.",
    "No objection was raised to the revised timetable, which now ends in the spring of next year.",
    "The working group will report back with a costed plan before the next meeting is called.",
    "Until then the existing arrangements remain in force for every department concerned here.",
    "The meeting closed with thanks to the staff who had prepared the papers for the discussion.",
];

/// A page: a running head and a running foot in 8 pt (short, clear of the body), a title in
/// 16 pt in the top band, eight lines of 11 pt body, and a footnote at the foot.
fn page() -> Vec<u8> {
    let mut content = String::new();
    content.push_str("BT /F1 8 Tf 72 775 Td (Journal of Testing, Vol. 3) Tj ET\n");
    content.push_str("BT /F1 16 Tf 72 745 Td (Minutes of the Meeting) Tj ET\n");
    for (i, line) in BODY.iter().enumerate() {
        let y = 700 - 14 * i as i32;
        content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({line}) Tj ET\n"));
    }
    content.push_str("BT /F1 8 Tf 72 52 Td (1 Source: the committee papers) Tj ET\n");
    content.push_str("BT /F1 8 Tf 72 28 Td (Journal of Testing, Vol. 3) Tj ET\n");
    common::assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
          /Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        common::stream_object(
            &format!("<</Length {}>>", content.len()),
            content.as_bytes(),
        ),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ])
}

fn markdown(options: ParseOptions) -> String {
    let doc = parse_bytes_with_options(&page(), options).unwrap();
    to_markdown(&doc, &RenderOptions::default()).unwrap()
}

#[test]
fn running_heads_and_feet_are_kept_by_default() {
    let md = markdown(ParseOptions::new());
    assert_eq!(md.matches("Journal of Testing").count(), 2, "{md}");
}

#[test]
fn the_option_omits_the_running_head_and_foot_and_nothing_else() {
    let md = markdown(ParseOptions::new().with_omit_running_heads(true));
    assert!(!md.contains("Journal of Testing"), "{md}");
    assert!(
        md.contains("Minutes of the Meeting"),
        "a title is not furniture: {md}"
    );
    assert!(
        md.contains("1 Source: the committee papers"),
        "a footnote is kept: {md}"
    );
    assert!(md.contains("The meeting closed with thanks"), "{md}");
    assert!(md.contains("The committee reviewed the proposal"), "{md}");
}
