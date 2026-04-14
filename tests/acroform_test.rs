use std::path::Path;
use unpdf::model::{FieldType, FieldValue, FormField};
use unpdf::parse_file;
use unpdf::to_markdown;

#[test]
fn test_form_field_text() {
    let field = FormField {
        name: "FirstName".to_string(),
        field_type: FieldType::Text,
        value: Some(FieldValue::Text("John".to_string())),
        default_value: None,
    };
    assert_eq!(field.display_value(), "John");
}

#[test]
fn test_form_field_checkbox_checked() {
    let field = FormField {
        name: "Agree".to_string(),
        field_type: FieldType::Checkbox,
        value: Some(FieldValue::Boolean(true)),
        default_value: None,
    };
    assert_eq!(field.display_value(), "[x]");
}

#[test]
fn test_form_field_checkbox_unchecked() {
    let field = FormField {
        name: "Agree".to_string(),
        field_type: FieldType::Checkbox,
        value: Some(FieldValue::Boolean(false)),
        default_value: None,
    };
    assert_eq!(field.display_value(), "[ ]");
}

#[test]
fn test_form_field_no_value_uses_default() {
    let field = FormField {
        name: "Email".to_string(),
        field_type: FieldType::Text,
        value: None,
        default_value: Some(FieldValue::Text("default@example.com".to_string())),
    };
    assert_eq!(field.display_value(), "default@example.com");
}

#[test]
fn test_form_field_no_value_no_default() {
    let field = FormField {
        name: "Empty".to_string(),
        field_type: FieldType::Text,
        value: None,
        default_value: None,
    };
    assert_eq!(field.display_value(), "");
}

#[test]
fn test_form_pdf_extracts_fields() {
    let path = Path::new("test-files/forms/pdf-form-sample.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    assert!(
        !doc.form_fields.is_empty(),
        "Should extract form fields from PDF with AcroForm"
    );
}

#[test]
fn test_pdflatex_form_fields() {
    let path = Path::new("test-files/forms/pdflatex-form.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    assert!(
        !doc.form_fields.is_empty(),
        "Should extract form fields from LaTeX-generated PDF"
    );
}

#[test]
fn test_non_form_pdf_has_no_fields() {
    let path = Path::new("test-files/basic/trivial.pdf");
    if !path.exists() {
        return;
    }
    let doc = parse_file(path).unwrap();
    assert!(
        doc.form_fields.is_empty(),
        "Non-form PDF should have no form fields"
    );
}

#[test]
fn test_form_fields_in_markdown() {
    let path = Path::new("test-files/forms/pdf-form-sample.pdf");
    if !path.exists() {
        return;
    }
    let md = to_markdown(path).unwrap();
    // Form fields section should be present
    assert!(
        md.contains("Form Fields"),
        "Markdown should contain Form Fields section"
    );
}
