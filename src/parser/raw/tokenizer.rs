//! PDF object tokenizer/parser.

use std::collections::HashMap;
use crate::error::{Error, Result};

/// A PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum PdfObject {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<PdfObject>),
    Dict(PdfDict),
    Stream(PdfStream),
    Reference(u32, u16),
}

/// A PDF dictionary.
pub type PdfDict = HashMap<Vec<u8>, PdfObject>;

/// A PDF stream object.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfStream {
    pub dict: PdfDict,
    pub raw_data: Vec<u8>,
}

impl PdfObject {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PdfObject::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PdfObject::Real(r) => Some(*r),
            PdfObject::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|f| f as f32)
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            PdfObject::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_str_bytes(&self) -> Option<&[u8]> {
        match self {
            PdfObject::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[PdfObject]> {
        match self {
            PdfObject::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&PdfDict> {
        match self {
            PdfObject::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&PdfStream> {
        match self {
            PdfObject::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<(u32, u16)> {
        match self {
            PdfObject::Reference(n, g) => Some((*n, *g)),
            _ => None,
        }
    }
}

/// Get a value from a PdfDict by key.
pub fn dict_get<'a>(dict: &'a PdfDict, key: &[u8]) -> Option<&'a PdfObject> {
    dict.get(key)
}
