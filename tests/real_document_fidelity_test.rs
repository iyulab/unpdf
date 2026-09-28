//! Expectations on real documents, not synthetic ones.
//!
//! The rest of the suite assembles PDFs byte by byte, which pins one behaviour at a time
//! but cannot say whether a heuristic change helps or hurts a document as a whole.
//! These tests convert committed real documents and assert on what a reader would
//! check first. A fixture that is missing fails the test instead of skipping it.

use std::path::PathBuf;

use unpdf::render::{to_markdown, RenderOptions};

fn markdown_of(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let doc = unpdf::parse_file(&path)
        .unwrap_or_else(|e| panic!("fixture {} must parse: {e}", path.display()));
    to_markdown(&doc, &RenderOptions::default()).expect("markdown renders")
}

#[test]
fn latex_resume_keeps_its_section_structure() {
    let md = markdown_of("latex_resume.pdf");
    for heading in [
        "# JOHN DOE",
        "## PERSONAL PROFILE",
        "## EDUCATION",
        "## EXPERIENCE",
        "## PROJECTS",
        "## SKILLS",
        "## LANGUAGES",
    ] {
        assert!(
            md.lines().any(|l| l == heading),
            "missing heading {heading:?}\n--- markdown ---\n{md}"
        );
    }
}

#[test]
fn latex_resume_separates_runs_that_share_a_row() {
    let md = markdown_of("latex_resume.pdf");
    // A bold name and a right-aligned italic date on one row: the markers must not abut.
    assert!(
        md.contains("**University of Somewhere** *October 2022 - July 2025*"),
        "bold and italic runs on one row must stay separated\n--- markdown ---\n{md}"
    );
    assert!(
        !md.contains("***"),
        "abutting emphasis markers read as one bold-italic opener\n--- markdown ---\n{md}"
    );
    assert!(md.contains("**GitHub link:** github.com"), "{md}");
}

#[test]
fn latex_resume_keeps_words_hyphens_and_lists() {
    let md = markdown_of("latex_resume.pdf");
    assert!(
        md.contains("Some Road, Somewhere"),
        "word spaces lost\n{md}"
    );
    assert!(
        md.contains("Self-teaching, Problem-solving"),
        "hard hyphens lost\n{md}"
    );
    assert!(
        md.lines()
            .any(|l| l.starts_with("- Nam dui ligula, fringilla a")),
        "bullet list lost\n{md}"
    );
}
