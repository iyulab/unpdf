//! A form field's entries may be indirect objects (ISO 32000-1 §7.3.10): `/FT 5 0 R` with
//! object 5 holding `/Tx`. PDFium reads the field type, the name and the flags through the
//! reference (`CPDF_FormField`), and so does pdf.js (`Dict.get`). A reader that takes only a
//! direct entry skips the field — a form with its fields in this shape reads as no form.
//!
//! The PDFs are assembled in the test, so it runs from the repository alone.

use unpdf::model::{FieldType, FieldValue};
use unpdf::PdfParser;

fn assemble(objects: &[String]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \r\n");
    for offset in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \r\n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );
    out
}

/// A one-page form with a single widget field. `entries` are the field dictionary's
/// entries; `extra` are further objects, numbered from 5.
fn form(entries: &str, extra: &[&str]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".to_string(),
        format!("<< /Type /Annot /Subtype /Widget /Rect [0 0 100 20] {entries} >>"),
    ];
    objects.extend(extra.iter().map(|s| s.to_string()));
    assemble(&objects)
}

fn fields(data: &[u8]) -> Vec<(String, FieldType, Option<String>)> {
    let doc = PdfParser::from_bytes(data)
        .expect("fixture loads")
        .parse()
        .expect("fixture parses");
    doc.form_fields
        .iter()
        .map(|f| {
            let value = match &f.value {
                Some(FieldValue::Text(t)) | Some(FieldValue::Choice(t)) => Some(t.clone()),
                _ => None,
            };
            (f.name.clone(), f.field_type.clone(), value)
        })
        .collect()
}

#[test]
fn the_control_with_direct_entries_reads() {
    let data = form("/FT /Tx /T (Name) /V (Ada)", &[]);
    assert_eq!(
        fields(&data),
        vec![("Name".to_string(), FieldType::Text, Some("Ada".to_string()))]
    );
}

#[test]
fn an_indirect_field_type_is_read() {
    let data = form("/FT 5 0 R /T (Name) /V (Ada)", &["/Tx"]);
    assert_eq!(
        fields(&data),
        vec![("Name".to_string(), FieldType::Text, Some("Ada".to_string()))]
    );
}

#[test]
fn an_indirect_field_name_is_read() {
    let data = form("/FT /Tx /T 5 0 R /V (Ada)", &["(Name)"]);
    assert_eq!(
        fields(&data),
        vec![("Name".to_string(), FieldType::Text, Some("Ada".to_string()))]
    );
}

#[test]
fn indirect_flags_decide_the_button_kind() {
    // /Ff bit 17 (value 65536) makes a button a radio button; stored as an indirect integer.
    let data = form("/FT /Btn /T (Choice) /Ff 5 0 R", &["65536"]);
    let got = fields(&data);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, FieldType::RadioButton, "{got:?}");
}

#[test]
fn an_indirect_value_is_read() {
    let data = form("/FT /Tx /T (Name) /V 5 0 R", &["(Ada)"]);
    assert_eq!(
        fields(&data),
        vec![("Name".to_string(), FieldType::Text, Some("Ada".to_string()))]
    );
}
