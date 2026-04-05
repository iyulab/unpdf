use serde::{Deserialize, Serialize};

/// A form field extracted from PDF AcroForm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    /// Fully qualified field name (dot-separated hierarchy).
    pub name: String,
    /// Field type.
    pub field_type: FieldType,
    /// Current value (/V).
    pub value: Option<FieldValue>,
    /// Default value (/DV).
    pub default_value: Option<FieldValue>,
}

/// AcroForm field type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Text,
    Checkbox,
    RadioButton,
    Dropdown,
    ListBox,
    PushButton,
    Signature,
}

/// Field value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldValue {
    Text(String),
    Boolean(bool),
    Choice(String),
    Choices(Vec<String>),
}

impl FormField {
    /// Get the display representation of this field's value.
    pub fn display_value(&self) -> String {
        let val = self.value.as_ref().or(self.default_value.as_ref());
        match val {
            Some(FieldValue::Text(s)) => s.clone(),
            Some(FieldValue::Boolean(true)) => "[x]".to_string(),
            Some(FieldValue::Boolean(false)) => "[ ]".to_string(),
            Some(FieldValue::Choice(s)) => s.clone(),
            Some(FieldValue::Choices(v)) => v.join(", "),
            None => String::new(),
        }
    }
}
