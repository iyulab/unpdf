//! PDF document structure.

use std::collections::{BTreeMap, HashMap};

use crate::error::{Error, Result};

use super::crypt::{self, EncryptionParams};
use super::stream;
use super::tokenizer::{self, dict_get, PdfDict, PdfObject, PdfStream};
use super::xref::{self, XrefEntry};

/// A parsed PDF document.
pub struct RawDocument {
    /// All loaded objects, keyed by (object_number, generation_number).
    objects: HashMap<(u32, u16), PdfObject>,
    /// The trailer dictionary (from the newest xref section).
    trailer: PdfDict,
    /// PDF version string (e.g., "1.4", "1.7").
    pub version: String,
}

impl RawDocument {
    /// Load a PDF document from bytes.
    pub fn load(data: &[u8]) -> Result<Self> {
        // 1. Parse PDF version from header: %PDF-X.Y
        let version = parse_version(data)?;

        // 2. Parse xref chain to get table + trailer
        let (xref_table, trailer) = xref::parse_xref_chain(data)?;

        // 3. Load all objects from xref entries
        let mut objects = HashMap::new();

        // First pass: load all uncompressed objects
        for (&(obj_num, gen_num), &entry) in &xref_table.entries {
            if let XrefEntry::Uncompressed(offset) = entry {
                match tokenizer::parse_object(data, offset) {
                    Ok((obj, _)) => {
                        objects.insert((obj_num, gen_num), obj);
                    }
                    Err(_) => {
                        // Skip objects that fail to parse (e.g., corrupted)
                    }
                }
            }
        }

        // Second pass: load compressed objects from ObjStm streams
        // Collect compressed entries grouped by stream object number
        let mut compressed_groups: HashMap<u32, Vec<(u32, u16, u32)>> = HashMap::new();
        for (&(obj_num, gen_num), &entry) in &xref_table.entries {
            if let XrefEntry::Compressed(stream_obj, index) = entry {
                compressed_groups
                    .entry(stream_obj)
                    .or_default()
                    .push((obj_num, gen_num, index));
            }
        }

        for (stream_obj_num, entries) in &compressed_groups {
            if let Some(stream_obj) = objects.get(&(*stream_obj_num, 0)) {
                if let Some(pdf_stream) = stream_obj.as_stream() {
                    if let Ok(extracted) = extract_objstm_objects(pdf_stream) {
                        for &(obj_num, gen_num, index) in entries {
                            if let Some(obj) = extracted.get(&(index as usize)) {
                                objects.insert((obj_num, gen_num), obj.clone());
                            }
                        }
                    }
                }
            }
        }

        let mut doc = RawDocument {
            objects,
            trailer,
            version,
        };

        // Try to decrypt if the PDF is encrypted
        if doc.is_encrypted() {
            doc.try_decrypt()?;
        }

        Ok(doc)
    }

    /// Attempt decryption with an empty user password (covers owner-password-only PDFs).
    fn try_decrypt(&mut self) -> Result<()> {
        let params = match self.encryption_params() {
            Some(p) => p,
            None => {
                return Err(Error::PdfParse(
                    "Encrypt dictionary present but could not be parsed".into(),
                ));
            }
        };

        // Only support R2-R4 for now
        if params.revision > 4 || params.revision < 2 {
            return Err(Error::Other(format!(
                "PDF encryption revision {} is not yet supported",
                params.revision
            )));
        }

        // Try empty password (most common case: owner-password-only)
        let key = crypt::authenticate_user_password(&params, b"")
            .ok_or(Error::Encrypted)?;

        // Decrypt all objects (except the Encrypt dict itself)
        let encrypt_obj_id = dict_get(&self.trailer, b"Encrypt")
            .and_then(|o| o.as_reference());
        self.decrypt_objects(&key, &params, encrypt_obj_id);

        Ok(())
    }

    /// Decrypt all string and stream objects in the document.
    fn decrypt_objects(
        &mut self,
        file_key: &[u8],
        params: &EncryptionParams,
        encrypt_obj_id: Option<(u32, u16)>,
    ) {
        let obj_ids: Vec<(u32, u16)> = self.objects.keys().cloned().collect();

        for (obj_num, gen_num) in obj_ids {
            // Skip the Encrypt dictionary object itself
            if Some((obj_num, gen_num)) == encrypt_obj_id {
                continue;
            }

            let obj_key = crypt::object_key(file_key, obj_num, gen_num, params.use_aes);

            if let Some(obj) = self.objects.get_mut(&(obj_num, gen_num)) {
                decrypt_object(obj, &obj_key, params.use_aes);
            }
        }
    }

    /// Parse encryption parameters from the trailer /Encrypt dictionary.
    fn encryption_params(&self) -> Option<EncryptionParams> {
        let encrypt_ref = dict_get(&self.trailer, b"Encrypt")?.as_reference()?;
        let encrypt_dict = self.get_dict(encrypt_ref).ok()?;

        let v = dict_get(encrypt_dict, b"V")
            .and_then(|o| o.as_i64())
            .unwrap_or(0) as u32;
        let r = dict_get(encrypt_dict, b"R")
            .and_then(|o| o.as_i64())
            .unwrap_or(0) as u32;
        let length = dict_get(encrypt_dict, b"Length")
            .and_then(|o| o.as_i64())
            .unwrap_or(40) as u32;
        let p = dict_get(encrypt_dict, b"P")
            .and_then(|o| o.as_i64())
            .unwrap_or(0) as i32;

        let o = dict_get(encrypt_dict, b"O")
            .and_then(|o| o.as_str_bytes())?
            .to_vec();
        let u = dict_get(encrypt_dict, b"U")
            .and_then(|o| o.as_str_bytes())?
            .to_vec();

        // Get file ID from trailer /ID array
        let file_id = dict_get(&self.trailer, b"ID")
            .and_then(|o| o.as_array())
            .and_then(|arr| arr.first())
            .and_then(|o| o.as_str_bytes())
            .unwrap_or(&[])
            .to_vec();

        // Detect AES usage: R4 with /StmF or /StrF = /AESV2
        let use_aes = if r >= 4 {
            let cf = dict_get(encrypt_dict, b"CF").and_then(|o| o.as_dict());
            let stmf = dict_get(encrypt_dict, b"StmF").and_then(|o| o.as_name());
            let strf = dict_get(encrypt_dict, b"StrF").and_then(|o| o.as_name());

            // Check if the named crypt filter uses AESV2
            let filter_name = stmf.or(strf);
            if let (Some(cf_dict), Some(name)) = (cf, filter_name) {
                dict_get(cf_dict, name)
                    .and_then(|o| o.as_dict())
                    .and_then(|d| dict_get(d, b"CFM"))
                    .and_then(|o| o.as_name())
                    .map(|n| n == b"AESV2")
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };

        Some(EncryptionParams {
            version: v,
            revision: r,
            key_length: length,
            owner_hash: o,
            user_hash: u,
            permissions: p,
            file_id,
            use_aes,
        })
    }

    /// Get an object by its ID (object_number, generation_number).
    pub fn get_object(&self, id: (u32, u16)) -> Option<&PdfObject> {
        self.objects.get(&id)
    }

    /// Resolve a PdfObject: if it's a Reference, follow it to the actual object.
    /// If not a reference, return the object itself.
    pub fn resolve<'a>(&'a self, obj: &'a PdfObject) -> &'a PdfObject {
        let mut current = obj;
        for _ in 0..10 {
            if let PdfObject::Reference(n, g) = current {
                if let Some(resolved) = self.objects.get(&(*n, *g)) {
                    current = resolved;
                } else {
                    return current;
                }
            } else {
                return current;
            }
        }
        current
    }

    /// Get the trailer dictionary.
    pub fn trailer(&self) -> &PdfDict {
        &self.trailer
    }

    /// Get the catalog dictionary (via trailer /Root reference).
    pub fn catalog(&self) -> Result<&PdfDict> {
        let root_ref = dict_get(&self.trailer, b"Root")
            .ok_or_else(|| Error::MissingObject("trailer /Root".into()))?;
        let root = self.resolve(root_ref);
        root.as_dict()
            .ok_or_else(|| Error::PdfParse("catalog is not a dictionary".into()))
    }

    /// Get all pages as (1-based page_number -> (obj_num, gen_num)).
    /// Traverses the page tree: Catalog -> Pages -> recursive Kids.
    pub fn pages(&self) -> BTreeMap<u32, (u32, u16)> {
        let mut result = BTreeMap::new();
        let mut page_num = 1u32;

        let catalog = match self.catalog() {
            Ok(c) => c,
            Err(_) => return result,
        };

        let pages_ref = match dict_get(catalog, b"Pages") {
            Some(r) => r,
            None => return result,
        };

        let pages_id = match pages_ref.as_reference() {
            Some(id) => id,
            None => return result,
        };

        self.collect_pages(pages_id, &mut result, &mut page_num);
        result
    }

    /// Get the number of pages.
    pub fn page_count(&self) -> u32 {
        self.pages().len() as u32
    }

    /// Get a dictionary by object ID, resolving references.
    pub fn get_dict(&self, id: (u32, u16)) -> Result<&PdfDict> {
        let obj = self
            .get_object(id)
            .ok_or_else(|| Error::MissingObject(format!("object {:?}", id)))?;
        let resolved = self.resolve(obj);
        match resolved {
            PdfObject::Dict(d) => Ok(d),
            PdfObject::Stream(s) => Ok(&s.dict),
            _ => Err(Error::PdfParse(format!(
                "object {:?} is not a dictionary",
                id
            ))),
        }
    }

    /// Check if the document is encrypted.
    pub fn is_encrypted(&self) -> bool {
        dict_get(&self.trailer, b"Encrypt").is_some()
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Recursively collect pages from the page tree.
    fn collect_pages(
        &self,
        node_id: (u32, u16),
        result: &mut BTreeMap<u32, (u32, u16)>,
        page_num: &mut u32,
    ) {
        let dict = match self.get_dict(node_id) {
            Ok(d) => d,
            Err(_) => return,
        };

        let type_name = dict_get(dict, b"Type").and_then(|o| o.as_name());

        match type_name {
            Some(b"Page") => {
                result.insert(*page_num, node_id);
                *page_num += 1;
            }
            Some(b"Pages") | None => {
                // Pages node — recurse into Kids
                if let Some(kids) = dict_get(dict, b"Kids").and_then(|o| o.as_array()) {
                    for kid in kids {
                        if let Some(kid_id) = kid.as_reference() {
                            self.collect_pages(kid_id, result, page_num);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Recursively decrypt strings and streams within a PDF object.
fn decrypt_object(obj: &mut PdfObject, key: &[u8], use_aes: bool) {
    match obj {
        PdfObject::Str(data) => {
            if use_aes {
                if let Some(decrypted) = crypt::decrypt_aes128(key, data) {
                    *data = decrypted;
                }
            } else {
                *data = crypt::decrypt_rc4(key, data);
            }
        }
        PdfObject::Stream(stream) => {
            if use_aes {
                if let Some(decrypted) = crypt::decrypt_aes128(key, &stream.raw_data) {
                    stream.raw_data = decrypted;
                }
            } else {
                stream.raw_data = crypt::decrypt_rc4(key, &stream.raw_data);
            }
        }
        PdfObject::Array(arr) => {
            for item in arr.iter_mut() {
                decrypt_object(item, key, use_aes);
            }
        }
        PdfObject::Dict(dict) => {
            for val in dict.values_mut() {
                decrypt_object(val, key, use_aes);
            }
        }
        _ => {}
    }
}

/// Parse the PDF version from the file header (`%PDF-X.Y`).
fn parse_version(data: &[u8]) -> Result<String> {
    if data.len() < 8 || &data[0..5] != b"%PDF-" {
        return Err(Error::UnknownFormat);
    }
    // Extract version string until whitespace or end
    let version_start = 5;
    let mut end = version_start;
    while end < data.len() && !data[end].is_ascii_whitespace() {
        end += 1;
    }
    let version = std::str::from_utf8(&data[version_start..end])
        .map_err(|_| Error::PdfParse("invalid version string".into()))?;
    Ok(version.to_string())
}

/// Extract objects from an ObjStm (Object Stream).
///
/// The stream contains N objects. The dictionary has:
/// - `/N`: number of objects
/// - `/First`: byte offset of the first object data (after the header pairs)
///
/// The header consists of N pairs of integers: obj_number byte_offset
/// The byte_offset is relative to `/First`.
fn extract_objstm_objects(pdf_stream: &PdfStream) -> Result<HashMap<usize, PdfObject>> {
    let n = dict_get(&pdf_stream.dict, b"N")
        .and_then(|o| o.as_i64())
        .ok_or_else(|| Error::PdfParse("ObjStm missing /N".into()))? as usize;

    let first = dict_get(&pdf_stream.dict, b"First")
        .and_then(|o| o.as_i64())
        .ok_or_else(|| Error::PdfParse("ObjStm missing /First".into()))? as usize;

    let decompressed = stream::decompress(pdf_stream)?;

    // Parse header: N pairs of (obj_number, byte_offset)
    let mut pos = 0;
    let mut offsets: Vec<(u32, usize)> = Vec::with_capacity(n);

    for _ in 0..n {
        pos = skip_ws(&decompressed, pos);
        let (obj_num, new_pos) = parse_int(&decompressed, pos)?;
        pos = skip_ws(&decompressed, new_pos);
        let (byte_offset, new_pos) = parse_int(&decompressed, pos)?;
        pos = new_pos;
        offsets.push((obj_num as u32, byte_offset as usize));
    }

    // Parse each object
    let mut result = HashMap::new();
    for (index, &(_obj_num, byte_offset)) in offsets.iter().enumerate() {
        let obj_pos = first + byte_offset;
        if obj_pos < decompressed.len() {
            if let Ok((obj, _)) = tokenizer::parse_object(&decompressed, obj_pos) {
                result.insert(index, obj);
            }
        }
    }

    Ok(result)
}

fn skip_ws(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && data[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn parse_int(data: &[u8], pos: usize) -> Result<(i64, usize)> {
    let start = pos;
    let mut p = pos;
    if p < data.len() && (data[p] == b'+' || data[p] == b'-') {
        p += 1;
    }
    while p < data.len() && data[p].is_ascii_digit() {
        p += 1;
    }
    if p == start {
        return Err(Error::PdfParse(format!(
            "expected integer at offset {}",
            pos
        )));
    }
    let s = std::str::from_utf8(&data[start..p])
        .map_err(|_| Error::PdfParse("invalid integer".into()))?;
    let val: i64 = s
        .parse()
        .map_err(|_| Error::PdfParse("invalid integer".into()))?;
    Ok((val, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_trivial_pdf() {
        let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
        let doc = RawDocument::load(&data).unwrap();
        assert!(doc.page_count() > 0);
        assert!(!doc.version.is_empty());
    }

    #[test]
    fn test_catalog_accessible() {
        let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
        let doc = RawDocument::load(&data).unwrap();
        let catalog = doc.catalog().unwrap();
        assert!(dict_get(catalog, b"Pages").is_some());
    }

    #[test]
    fn test_pages_enumeration() {
        let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
        let doc = RawDocument::load(&data).unwrap();
        let pages = doc.pages();
        assert!(!pages.is_empty());
        // Page numbers should be 1-based
        assert!(pages.contains_key(&1));
    }

    #[test]
    fn test_page_has_dict() {
        let data = std::fs::read("test-files/basic/trivial.pdf").unwrap();
        let doc = RawDocument::load(&data).unwrap();
        let pages = doc.pages();
        let first_page_id = pages[&1];
        let page_dict = doc.get_dict(first_page_id).unwrap();
        // A page dict should have /Type /Page
        let type_name = dict_get(page_dict, b"Type").and_then(|o| o.as_name());
        assert_eq!(type_name, Some(b"Page".as_slice()));
    }

    #[test]
    fn test_load_unicode_pdf() {
        let data = std::fs::read("test-files/basic/unicode-test.pdf").unwrap();
        // This PDF is encrypted. load() now attempts decryption with empty password.
        // It may succeed (decrypted) or fail (needs real password).
        match RawDocument::load(&data) {
            Ok(doc) => {
                assert!(!doc.version.is_empty());
                // If decryption succeeded, the encrypted flag is still in the trailer
                // but objects are now decrypted and usable.
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("encrypted") || msg.contains("Encrypted") || msg.contains("password") || msg.contains("supported"),
                    "Error should be about encryption: {}", msg
                );
            }
        }
    }

    #[test]
    fn test_load_outline_pdf() {
        let data = std::fs::read("test-files/basic/outline.pdf").unwrap();
        let doc = RawDocument::load(&data).unwrap();
        assert!(doc.page_count() > 0);
    }
}
