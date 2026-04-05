use unpdf::model::{FieldType, FieldValue, FormField};

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
