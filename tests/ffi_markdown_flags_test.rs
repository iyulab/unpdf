//! Every markdown flag must reach the renderer, from both markdown entry points.
//!
//! The flag bitmask is the only way a C-ABI caller — and through it the Python and C#
//! packages — can ask for a render option. A bit that is defined but never read is a
//! promise the library does not keep, and nothing about the call reports that: the caller
//! gets a successful render that ignored what it asked for.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::CStr;
use std::os::raw::c_char;

use common::{mixed_pdf, text_pdf};
use unpdf::ffi::{
    unpdf_free_document, unpdf_free_string, unpdf_page_to_markdown, unpdf_parse_bytes,
    unpdf_to_markdown, UNPDF_FLAG_ESCAPE_SPECIAL, UNPDF_FLAG_FRONTMATTER, UNPDF_FLAG_PAGE_MARKERS,
};

unsafe fn take_string(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    let s = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
    unpdf_free_string(ptr);
    s
}

unsafe fn markdown(bytes: &[u8], flags: u32) -> String {
    let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
    assert!(!doc.is_null());
    let out = take_string(unpdf_to_markdown(doc, flags));
    unpdf_free_document(doc);
    out
}

#[test]
fn page_markers_flag_marks_page_boundaries() {
    let bytes = mixed_pdf();
    unsafe {
        assert!(
            !markdown(&bytes, 0).contains("<!-- page "),
            "page markers must stay off unless asked for"
        );

        let marked = markdown(&bytes, UNPDF_FLAG_PAGE_MARKERS);
        assert!(
            marked.contains("<!-- page 1 -->") && marked.contains("<!-- page 2 -->"),
            "each page boundary must be marked, got {marked:?}"
        );
    }
}

/// The single-page entry point reads the same bitmask, and read it in its own copy of the
/// decoding code — which is how a flag comes to work through one call and not the other.
#[test]
fn page_markers_flag_reaches_the_single_page_entry_point() {
    let bytes = mixed_pdf();
    unsafe {
        let doc = unpdf_parse_bytes(bytes.as_ptr(), bytes.len());
        assert!(!doc.is_null());

        let plain = take_string(unpdf_page_to_markdown(doc, 1, 0));
        assert!(!plain.contains("<!-- page "));

        let marked = take_string(unpdf_page_to_markdown(doc, 1, UNPDF_FLAG_PAGE_MARKERS));
        assert!(marked.contains("<!-- page 1 -->"), "got {marked:?}");

        unpdf_free_document(doc);
    }
}

/// Flags are independent bits: asking for one must not turn on another, and combining
/// them must give both.
#[test]
fn flags_do_not_interfere_with_each_other() {
    let bytes = text_pdf();
    unsafe {
        let markers_only = markdown(&bytes, UNPDF_FLAG_PAGE_MARKERS);
        assert!(markers_only.contains("<!-- page 1 -->"));
        assert!(
            !markers_only.starts_with("---"),
            "frontmatter was not asked for, got {markers_only:?}"
        );

        let both = markdown(&bytes, UNPDF_FLAG_FRONTMATTER | UNPDF_FLAG_PAGE_MARKERS);
        assert!(both.starts_with("---"), "got {both:?}");
        assert!(both.contains("<!-- page 1 -->"), "got {both:?}");
    }
}

/// Bit 4 named a paragraph-spacing option that never reached the renderer. It is retired
/// rather than reused, so a caller still passing it gets exactly what it always got.
#[test]
fn the_retired_bit_is_inert_and_not_reused() {
    let bytes = text_pdf();
    const RETIRED: u32 = 4;
    unsafe {
        assert_eq!(markdown(&bytes, RETIRED), markdown(&bytes, 0));
    }

    assert_eq!(UNPDF_FLAG_FRONTMATTER, 1);
    assert_eq!(UNPDF_FLAG_ESCAPE_SPECIAL, 2);
    assert_eq!(
        UNPDF_FLAG_PAGE_MARKERS, 8,
        "flag values are part of the C ABI: existing bits keep their meaning"
    );
}
