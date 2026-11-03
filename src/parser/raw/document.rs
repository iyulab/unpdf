//! PDF document structure.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::{Error, Result};

use super::crypt::{self, EncryptionParams};
use super::stream;
use super::tokenizer::{self, dict_get, PdfDict, PdfObject, PdfStream};
use super::xref::{self, XrefEntry, XrefTable};

/// A parsed PDF document.
pub struct RawDocument {
    /// All loaded objects, keyed by (object_number, generation_number).
    objects: HashMap<(u32, u16), PdfObject>,
    /// The trailer dictionary (from the newest xref section).
    trailer: PdfDict,
    /// PDF version string (e.g., "1.4", "1.7").
    pub version: String,
    /// Objects the xref table pointed at that could not be loaded.
    skipped_objects: usize,
    /// Whether the cross-reference table was unusable and rebuilt from the file's objects.
    xref_reconstructed: bool,
}

/// Whether every uncompressed entry of `table` points at its own `num gen obj` header.
fn points_at_its_objects(data: &[u8], table: &XrefTable) -> bool {
    table
        .entries
        .iter()
        .all(|(&(num, gen), entry)| match *entry {
            XrefEntry::Uncompressed(offset) => xref::object_header_at(data, offset, num, gen),
            XrefEntry::Compressed(..) => true,
        })
}

/// `chain`'s trailer with what `found` adds: the chain's entries win where both have one.
fn merge_trailers(mut chain: PdfDict, found: PdfDict) -> PdfDict {
    for (key, value) in found {
        chain.entry(key).or_insert(value);
    }
    chain
}

/// Result of walking the page tree: the pages found, and what the walk had to drop.
#[derive(Debug, Default, Clone)]
pub struct PageTreeScan {
    /// 1-based page number → object id.
    pub pages: BTreeMap<u32, (u32, u16)>,

    /// Page-tree nodes the walk could not use: an unresolvable reference, a kid that
    /// is not a reference, a node that is neither `/Page` nor `/Pages`, or a missing
    /// catalog / root `Pages` entry.
    ///
    /// This is **not** a count of lost pages — an unusable intermediate `Pages` node
    /// drops its entire subtree, so one unresolved node can cost many pages. Treat any
    /// non-zero value as "the page set is incomplete" and nothing more.
    pub unresolved_nodes: usize,
}

impl RawDocument {
    /// Load a PDF document from bytes, decrypting with the empty user password only.
    pub fn load(data: &[u8]) -> Result<Self> {
        Self::load_with_password(data, None)
    }

    /// Load a PDF document from bytes, offering `password` to an encrypted one.
    ///
    /// The empty password is always tried first: a document encrypted with an owner password
    /// only opens with it, and that is the common case. A caller-supplied password is tried
    /// when the empty one does not authenticate, and its failure is reported as
    /// [`Error::InvalidPassword`] -- distinct from [`Error::Encrypted`], which means no
    /// password was offered at all. The two send the caller in different directions: correct
    /// the password, or go and get one.
    pub fn load_with_password(data: &[u8], password: Option<&str>) -> Result<Self> {
        // 1. Parse PDF version from header: %PDF-X.Y
        let version = parse_version(data)?;

        // 2. Parse xref chain to get table + trailer. A table that cannot be read, or that
        //    points where its objects are not, is rebuilt from the objects in the file.
        let (xref_table, trailer, xref_reconstructed) = match xref::parse_xref_chain(data) {
            Ok((table, trailer)) if points_at_its_objects(data, &table) => (table, trailer, false),
            Ok((table, trailer)) => match xref::reconstruct(data) {
                // The chain's trailer still names the catalog, the encryption, the ID.
                Ok((rebuilt, found)) => (rebuilt, merge_trailers(trailer, found), true),
                Err(_) => (table, trailer, false),
            },
            Err(error) => match xref::reconstruct(data) {
                Ok((rebuilt, trailer)) => (rebuilt, trailer, true),
                Err(_) => return Err(error),
            },
        };

        // 3. Load all objects from xref entries
        let mut objects = HashMap::new();
        let mut skipped_objects = 0usize;

        // First pass: load all uncompressed objects
        for (&(obj_num, gen_num), &entry) in &xref_table.entries {
            if let XrefEntry::Uncompressed(offset) = entry {
                match tokenizer::parse_object(data, offset) {
                    Ok((obj, _)) => {
                        objects.insert((obj_num, gen_num), obj);
                    }
                    Err(_) => {
                        // Skip objects that fail to parse (e.g., corrupted), but count
                        // them: a caller has no other way to learn the file was lossy.
                        skipped_objects += 1;
                    }
                }
            }
        }

        // Collect compressed xref entries for later ObjStm extraction
        let mut compressed_groups: HashMap<u32, Vec<(u32, u16, u32)>> = HashMap::new();
        for (&(obj_num, gen_num), &entry) in &xref_table.entries {
            if let XrefEntry::Compressed(stream_obj, index) = entry {
                compressed_groups
                    .entry(stream_obj)
                    .or_default()
                    .push((obj_num, gen_num, index));
            }
        }

        let mut doc = RawDocument {
            objects,
            trailer,
            version,
            skipped_objects,
            xref_reconstructed,
        };

        // Decrypt before ObjStm extraction: ObjStm streams are encrypted and must
        // be decrypted before their compressed content can be decompressed and parsed.
        if doc.is_encrypted() {
            doc.try_decrypt(password)?;
        }

        // Second pass: extract compressed objects from ObjStm streams (now decrypted).
        // An unusable ObjStm loses every object it carried, so count the whole group —
        // that is the difference between "one object skipped" and "a chapter missing".
        for (stream_obj_num, entries) in &compressed_groups {
            let extracted = doc
                .objects
                .get(&(*stream_obj_num, 0))
                .and_then(|obj| obj.as_stream())
                .and_then(|pdf_stream| extract_objstm_objects(pdf_stream).ok());

            let Some(extracted) = extracted else {
                doc.skipped_objects += entries.len();
                continue;
            };

            for &(obj_num, gen_num, index) in entries {
                match extracted.get(&(index as usize)) {
                    Some(obj) => {
                        doc.objects.insert((obj_num, gen_num), obj.clone());
                    }
                    None => doc.skipped_objects += 1,
                }
            }
        }

        doc.inline_filter_references();
        Ok(doc)
    }

    /// Replace the indirect references in every stream's `/Filter` and `/DecodeParms` (the
    /// entry, or the entries of its array) with the objects they name. The filter chain is
    /// applied by a function that sees only the stream, so a predictor written as
    /// `/DecodeParms 8 0 R` -- what many producers do -- would otherwise never be applied.
    fn inline_filter_references(&mut self) {
        const KEYS: [&[u8]; 2] = [b"Filter", b"DecodeParms"];
        let is_reference = |obj: &PdfObject| matches!(obj, PdfObject::Reference(..));
        let needs_inlining = |obj: &PdfObject| match obj {
            PdfObject::Stream(stream) => KEYS.iter().any(|key| match stream.dict.get(*key) {
                Some(PdfObject::Array(items)) => items.iter().any(is_reference),
                Some(other) => is_reference(other),
                None => false,
            }),
            _ => false,
        };
        let ids: Vec<(u32, u16)> = self
            .objects
            .iter()
            .filter(|(_, obj)| needs_inlining(obj))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let mut inlined: Vec<(&[u8], PdfObject)> = Vec::new();
            if let Some(PdfObject::Stream(stream)) = self.objects.get(&id) {
                for key in KEYS {
                    let Some(value) = stream.dict.get(key) else {
                        continue;
                    };
                    inlined.push((
                        key,
                        match self.resolve(value) {
                            PdfObject::Array(items) => PdfObject::Array(
                                items
                                    .iter()
                                    .map(|item| self.resolve(item).clone())
                                    .collect(),
                            ),
                            other => other.clone(),
                        },
                    ));
                }
            }
            if let Some(PdfObject::Stream(stream)) = self.objects.get_mut(&id) {
                for (key, value) in inlined {
                    stream.dict.insert(key.to_vec(), value);
                }
            }
        }
    }

    /// Attempt decryption with the empty user password, then with `password` if given.
    fn try_decrypt(&mut self, password: Option<&str>) -> Result<()> {
        let params = match self.encryption_params() {
            Some(p) => p,
            None => {
                return Err(Error::PdfParse(
                    "Encrypt dictionary present but could not be parsed".into(),
                ));
            }
        };

        // R2-R4 (RC4, AES-128) and R5-R6 (AES-256). Anything else is reported as
        // `UnsupportedVersion` rather than the catch-all: this is exactly the case that
        // discriminant names, and a caller meeting such a document otherwise cannot tell "this
        // build does not do that" apart from any other failure.
        if !(2..=6).contains(&params.revision) {
            return Err(Error::UnsupportedVersion(format!(
                "encryption revision {}",
                params.revision
            )));
        }

        // Try the empty password first (most common case: owner-password-only), then the one
        // the caller supplied. Order does not change the outcome -- whichever authenticates
        // yields the same file key for the same document -- but it keeps every document that
        // opened before opening exactly as it did.
        let authenticate = |password: &[u8]| {
            if params.revision >= 5 {
                crypt::authenticate_aes256(&params, password)
            } else {
                crypt::authenticate_user_password(&params, password)
            }
        };
        let key = match authenticate(b"") {
            Some(key) => key,
            None => match password.filter(|p| !p.is_empty()) {
                // A password was offered and neither it nor the empty one worked. Saying
                // `Encrypted` here would tell the caller to find a password they already gave.
                Some(supplied) => {
                    authenticate(supplied.as_bytes()).ok_or(Error::InvalidPassword)?
                }
                None => return Err(Error::Encrypted),
            },
        };

        // Decrypt all objects (except the Encrypt dict itself)
        let encrypt_obj_id = dict_get(&self.trailer, b"Encrypt").and_then(|o| o.as_reference());
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

            let obj_key = crypt::object_key(file_key, obj_num, gen_num, params.cipher);

            if let Some(obj) = self.objects.get_mut(&(obj_num, gen_num)) {
                decrypt_object(obj, &obj_key, params.cipher);
            }
        }
    }

    /// Parse encryption parameters from the trailer /Encrypt dictionary.
    fn encryption_params(&self) -> Option<EncryptionParams> {
        // A reference to the dictionary, or the dictionary itself written into the trailer
        // (MuPDF writes it there).
        let encrypt_dict = match dict_get(&self.trailer, b"Encrypt")? {
            PdfObject::Dict(dict) => dict,
            other => self.get_dict(other.as_reference()?).ok()?,
        };

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

        // /EncryptMetadata defaults to true when absent (PDF spec)
        let encrypt_metadata = dict_get(encrypt_dict, b"EncryptMetadata")
            .map(|o| !matches!(o, crate::parser::raw::tokenizer::PdfObject::Bool(false)))
            .unwrap_or(true);

        // The cipher: the crypt filter `/StmF` (or `/StrF`) names, for R4 and up.
        let cipher_of = |cfm: &[u8]| match cfm {
            b"AESV3" => crypt::Cipher::Aes256,
            b"AESV2" => crypt::Cipher::Aes128,
            _ => crypt::Cipher::Rc4,
        };
        let cipher = if r >= 4 {
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
                    .map(cipher_of)
                    .unwrap_or(crypt::Cipher::Rc4)
            } else if r >= 5 {
                // R5-R6 define no cipher but AES-256.
                crypt::Cipher::Aes256
            } else {
                crypt::Cipher::Rc4
            }
        } else {
            crypt::Cipher::Rc4
        };
        let bytes_of = |key: &[u8]| {
            dict_get(encrypt_dict, key)
                .and_then(|o| o.as_str_bytes())
                .map(<[u8]>::to_vec)
                .unwrap_or_default()
        };

        Some(EncryptionParams {
            version: v,
            revision: r,
            key_length: length,
            owner_hash: o,
            user_hash: u,
            permissions: p,
            file_id,
            cipher,
            owner_key: bytes_of(b"OE"),
            user_key: bytes_of(b"UE"),
            encrypt_metadata,
        })
    }

    /// Get an object by its ID (object_number, generation_number).
    pub fn get_object(&self, id: (u32, u16)) -> Option<&PdfObject> {
        self.objects.get(&id)
    }

    /// The name `dict` holds under `key`, following an indirect reference to it
    /// (ISO 32000-1 §7.3.10: a name entry may be an indirect object, as PDFium's
    /// `GetNameFor` and pdf.js's `Dict.get` read it).
    pub fn dict_name<'a>(&'a self, dict: &'a PdfDict, key: &[u8]) -> Option<&'a [u8]> {
        self.resolve(dict.get(key)?).as_name()
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
    /// Traverses the page tree: Catalog -> Pages -> Kids.
    pub fn pages(&self) -> BTreeMap<u32, (u32, u16)> {
        self.scan_page_tree().pages
    }

    /// Get the number of pages.
    pub fn page_count(&self) -> u32 {
        self.pages().len() as u32
    }

    /// Page count as declared by the root `Pages` node (`/Count`), when it is readable.
    ///
    /// Independent of [`Self::pages`], which reports what the walk could actually reach.
    /// The two disagreeing means the file is damaged — see [`PageTreeScan`].
    pub fn declared_page_count(&self) -> Option<u32> {
        let root = self
            .catalog()
            .ok()
            .and_then(|c| dict_get(c, b"Pages"))
            .and_then(|r| r.as_reference())?;
        let count = dict_get(self.get_dict(root).ok()?, b"Count").and_then(|o| o.as_i64())?;
        u32::try_from(count).ok()
    }

    /// Number of objects the xref table pointed at that could not be loaded.
    pub fn skipped_object_count(&self) -> usize {
        self.skipped_objects
    }

    /// Whether the cross-reference table could not be used and was rebuilt by scanning the
    /// file for its objects.
    pub fn xref_reconstructed(&self) -> bool {
        self.xref_reconstructed
    }

    /// Walk the page tree, reporting both the pages reached and what had to be dropped.
    ///
    /// Depth-first over an explicit stack rather than by recursion: the page tree of a
    /// damaged (or hostile) file can nest arbitrarily deep or point back at itself, and
    /// neither may cost the caller its stack. A `visited` set makes cycles terminate.
    pub fn scan_page_tree(&self) -> PageTreeScan {
        let mut scan = PageTreeScan::default();

        // No usable catalog or root `Pages` loses every page at once. Report it as one
        // unusable node so callers see "incomplete", not "this document has no pages".
        let Some(root) = self
            .catalog()
            .ok()
            .and_then(|c| dict_get(c, b"Pages"))
            .and_then(|r| r.as_reference())
        else {
            scan.unresolved_nodes += 1;
            return scan;
        };

        let mut page_num = 1u32;
        let mut visited: HashSet<(u32, u16)> = HashSet::new();
        let mut stack = vec![root];

        while let Some(node_id) = stack.pop() {
            // Already walked: a cyclic or diamond page tree. Not a loss — just done.
            if !visited.insert(node_id) {
                continue;
            }

            let Ok(dict) = self.get_dict(node_id) else {
                scan.unresolved_nodes += 1;
                continue;
            };

            match dict_get(dict, b"Type").and_then(|o| o.as_name()) {
                Some(b"Page") => {
                    scan.pages.insert(page_num, node_id);
                    page_num += 1;
                }
                // `/Type` is optional on intermediate nodes in practice — treat absent
                // as a `Pages` node and recurse into Kids.
                Some(b"Pages") | None => {
                    if let Some(kids) = dict_get(dict, b"Kids").and_then(|o| o.as_array()) {
                        // Push in reverse so the leftmost kid pops first (document order).
                        for kid in kids.iter().rev() {
                            match kid.as_reference() {
                                Some(kid_id) => stack.push(kid_id),
                                None => scan.unresolved_nodes += 1,
                            }
                        }
                    }
                }
                // Neither a page nor a page-tree node: the subtree below it is unreachable.
                Some(_) => scan.unresolved_nodes += 1,
            }
        }

        scan
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
}

/// Recursively decrypt strings and streams within a PDF object.
fn decrypt_object(obj: &mut PdfObject, key: &[u8], cipher: crypt::Cipher) {
    let decrypt = |data: &[u8]| match cipher {
        crypt::Cipher::Rc4 => Some(crypt::decrypt_rc4(key, data)),
        crypt::Cipher::Aes128 | crypt::Cipher::Aes256 => crypt::decrypt_aes(key, data),
    };
    match obj {
        PdfObject::Str(data) => {
            if let Some(decrypted) = decrypt(data) {
                *data = decrypted;
            }
        }
        PdfObject::Stream(stream) => {
            if let Some(decrypted) = decrypt(&stream.raw_data) {
                stream.raw_data = decrypted;
            }
        }
        PdfObject::Array(arr) => {
            for item in arr.iter_mut() {
                decrypt_object(item, key, cipher);
            }
        }
        PdfObject::Dict(dict) => {
            for val in dict.values_mut() {
                decrypt_object(val, key, cipher);
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

/// Extract objects from an ObjStm (Object Stream), keyed by their index in it.
fn extract_objstm_objects(pdf_stream: &PdfStream) -> Result<HashMap<usize, PdfObject>> {
    Ok(objstm_contents(pdf_stream)?
        .into_iter()
        .enumerate()
        .filter_map(|(index, (_, obj))| obj.map(|obj| (index, obj)))
        .collect())
}

/// What an ObjStm (Object Stream) holds, in order: each object's number, and the object when
/// it parses.
///
/// The stream contains N objects. The dictionary has:
/// - `/N`: number of objects
/// - `/First`: byte offset of the first object data (after the header pairs)
///
/// The header consists of N pairs of integers: obj_number byte_offset
/// The byte_offset is relative to `/First`.
pub(super) fn objstm_contents(pdf_stream: &PdfStream) -> Result<Vec<(u32, Option<PdfObject>)>> {
    let n = dict_get(&pdf_stream.dict, b"N")
        .and_then(|o| o.as_i64())
        .ok_or_else(|| Error::PdfParse("ObjStm missing /N".into()))? as usize;

    let first = dict_get(&pdf_stream.dict, b"First")
        .and_then(|o| o.as_i64())
        .ok_or_else(|| Error::PdfParse("ObjStm missing /First".into()))? as usize;

    let decompressed = stream::decompress(pdf_stream)?;

    // Parse header: N pairs of (obj_number, byte_offset)
    let mut pos = 0;
    let mut offsets: Vec<(u32, usize)> = Vec::with_capacity(n.min(decompressed.len()));

    for _ in 0..n {
        pos = skip_ws(&decompressed, pos);
        let (obj_num, new_pos) = parse_int(&decompressed, pos)?;
        pos = skip_ws(&decompressed, new_pos);
        let (byte_offset, new_pos) = parse_int(&decompressed, pos)?;
        pos = new_pos;
        offsets.push((obj_num as u32, byte_offset as usize));
    }

    Ok(offsets
        .into_iter()
        .map(|(obj_num, byte_offset)| {
            let obj_pos = first + byte_offset;
            let obj = (obj_pos < decompressed.len())
                .then(|| tokenizer::parse_object(&decompressed, obj_pos).ok())
                .flatten()
                .map(|(obj, _)| obj);
            (obj_num, obj)
        })
        .collect())
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
    // Assembled in the test rather than read from disk -- see that module's docs.
    use crate::parser::test_pdf::{one_page_pdf, pdf, undecryptable_pdf};

    /// `ParseOptions::with_password` existed, was threaded through every options struct and the
    /// C ABI, and stopped one call short of the decryption it names: `RawDocument::load` had no
    /// password parameter and `try_decrypt` passed `b""`. A caller could not tell a wrong
    /// password from a missing one, because the one they gave was never tried (cycle-147).
    #[test]
    fn a_supplied_password_reaches_decryption_and_its_failure_is_named() {
        let encrypted = undecryptable_pdf();

        // `RawDocument` is not `Debug`, so no `unwrap_err`.
        let Err(without) = RawDocument::load_with_password(&encrypted, None) else {
            panic!("an encrypted document must not load without a password");
        };
        assert!(
            matches!(without, Error::Encrypted),
            "no password offered: the caller has to go and get one -- got {without}"
        );

        let Err(with) = RawDocument::load_with_password(&encrypted, Some("secret")) else {
            panic!("this fixture authenticates with no password at all");
        };
        assert!(
            matches!(with, Error::InvalidPassword),
            "a password was offered and did not work: saying Encrypted would send the caller \
             after a password they already gave -- got {with}"
        );
    }

    /// An empty string is not a password. Treating it as one would report InvalidPassword for
    /// a caller who supplied nothing, which is the same conflation in the other direction.
    #[test]
    fn an_empty_password_is_the_same_as_none() {
        let encrypted = undecryptable_pdf();

        let Err(err) = RawDocument::load_with_password(&encrypted, Some("")) else {
            panic!("this fixture authenticates with no password at all");
        };
        assert!(matches!(err, Error::Encrypted), "got {err}");
    }

    /// The plain `load` keeps its old behaviour exactly: empty password only.
    #[test]
    fn load_without_a_password_still_reports_encrypted() {
        let Err(err) = RawDocument::load(&undecryptable_pdf()) else {
            panic!("this fixture authenticates with no password at all");
        };
        assert!(matches!(err, Error::Encrypted), "got {err}");
    }

    #[test]
    fn a_minimal_document_reports_its_version_and_page_count() {
        let doc = RawDocument::load(&one_page_pdf()).expect("a well-formed PDF loads");
        assert_eq!(doc.page_count(), 1);
        assert_eq!(
            doc.version, "1.4",
            "the header version is read, not guessed"
        );
    }

    #[test]
    fn the_catalog_is_reachable_and_points_at_the_page_tree() {
        let doc = RawDocument::load(&one_page_pdf()).unwrap();
        let catalog = doc.catalog().expect("the trailer /Root resolves");
        assert!(
            dict_get(catalog, b"Pages").is_some(),
            "a catalog without /Pages would make page enumeration silently empty"
        );
    }

    #[test]
    fn pages_are_enumerated_from_one() {
        let doc = RawDocument::load(&one_page_pdf()).unwrap();
        let pages = doc.pages();
        assert_eq!(pages.len(), 1);
        assert!(
            pages.contains_key(&1),
            "page numbering is 1-indexed across this crate API"
        );
    }

    #[test]
    fn an_enumerated_page_resolves_to_a_page_dictionary() {
        let doc = RawDocument::load(&one_page_pdf()).unwrap();
        let first = doc.pages()[&1];
        let page_dict = doc.get_dict(first).expect("the id from pages() resolves");
        assert_eq!(
            dict_get(page_dict, b"Type").and_then(|o| o.as_name()),
            Some(b"Page".as_slice()),
            "pages() must not hand back the page tree node"
        );
    }

    #[test]
    fn a_two_page_tree_is_walked_in_order() {
        // One page proves almost nothing about a tree walk: a stub returning the first Kid
        // would pass every test above.
        let doc = RawDocument::load(&pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R 4 0 R]/Count 2>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>".to_vec(),
            ],
            1,
        ))
        .unwrap();

        assert_eq!(doc.page_count(), 2);
        let pages = doc.pages();
        assert_eq!(pages.len(), 2);
        // The second Kid must be page 2, not a second entry for page 1.
        assert_ne!(pages[&1], pages[&2]);
        let second = doc.get_dict(pages[&2]).unwrap();
        assert!(
            format!("{:?}", dict_get(second, b"MediaBox")).contains("200"),
            "page 2 should be the second Kid, not the first one again"
        );
    }

    #[test]
    fn an_encryption_revision_this_build_cannot_do_is_refused_by_kind() {
        // The standard security handler stops at revision 6. A later one is refused, and what
        // matters to a caller is that the refusal is distinguishable, since the catch-all kind
        // would make "this build does not implement that" look like any other failure.
        let bytes = pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_vec(),
                b"<</Filter/Standard/V 5/R 7/Length 256/P -1/O<00>/U<00>>>".to_vec(),
            ],
            1,
        );
        // The trailer built above carries no /Encrypt, so point it at object 4.
        let encrypted = String::from_utf8(bytes)
            .unwrap()
            .replace("/Root 1 0 R>>", "/Root 1 0 R/Encrypt 4 0 R>>")
            .into_bytes();

        let Err(err) = RawDocument::load(&encrypted) else {
            panic!("revision 7 does not exist, so loading must not succeed");
        };
        assert_eq!(
            err.kind(),
            crate::error::ErrorKind::UnsupportedVersion,
            "got {err:?} -- a caller cannot branch on this if it arrives as the catch-all"
        );
    }

    #[test]
    fn a_document_that_needs_a_password_is_refused_as_encrypted() {
        // RC4, revision 3. /U is what the empty user password would have to reproduce, and
        // zeros are not that -- so opening without a password must fail, and must say why:
        // "needs a password" is the one failure here a caller can act on.
        let zeros = "00".repeat(32);
        let bytes = pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_vec(),
                format!("<</Filter/Standard/V 2/R 3/Length 128/P -4/O<{zeros}>/U<{zeros}>>>")
                    .into_bytes(),
            ],
            1,
        );
        let id = "00".repeat(16);
        let encrypted = String::from_utf8(bytes)
            .unwrap()
            .replace(
                "/Root 1 0 R>>",
                &format!("/Root 1 0 R/Encrypt 4 0 R/ID[<{id}><{id}>]>>"),
            )
            .into_bytes();

        let Err(err) = RawDocument::load(&encrypted) else {
            panic!("the empty password does not authenticate, so loading must not succeed");
        };
        assert_eq!(
            err.kind(),
            crate::error::ErrorKind::Encrypted,
            "got {err:?} -- a password-protected file must not look like a damaged one"
        );
    }

    #[test]
    fn a_document_outline_is_read_rather_than_ignored() {
        // The test this replaces was named for outlines and asserted only that the page count
        // was above zero -- it would have passed against a build that ignored /Outlines
        // entirely.
        let doc = RawDocument::load(&pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R/Outlines 4 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_vec(),
                b"<</Type/Outlines/First 5 0 R/Last 5 0 R/Count 1>>".to_vec(),
                b"<</Title(Chapter One)/Parent 4 0 R/Dest[3 0 R /Fit]>>".to_vec(),
            ],
            1,
        ))
        .unwrap();

        let catalog = doc.catalog().unwrap();
        let outlines = dict_get(catalog, b"Outlines")
            .and_then(|o| o.as_reference())
            .expect("/Outlines is a reference in the catalog");
        let root = doc.get_dict(outlines).expect("/Outlines resolves");
        let first = dict_get(root, b"First")
            .and_then(|o| o.as_reference())
            .expect("an outline root has a first child");
        let item = doc
            .get_dict(first)
            .expect("the first outline item resolves");
        assert_eq!(
            dict_get(item, b"Title").and_then(|o| o.as_str_bytes()),
            Some(b"Chapter One".as_slice()),
            "the outline item title should survive parsing"
        );
    }
}
