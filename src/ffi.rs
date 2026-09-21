//! C-ABI Foreign Function Interface for unpdf.
//!
//! This module provides C-compatible bindings for using unpdf from other languages
//! such as C, C++, C#, Python, and any language with C FFI support.
//!
//! # Memory Management
//!
//! All strings returned by this library must be freed using `unpdf_free_string`.
//! All document handles must be freed using `unpdf_free_document`.
//!
//! # Error Handling
//!
//! Functions that can fail return a null pointer on error. Use `unpdf_last_error`
//! to retrieve the error message and `unpdf_last_error_kind` to classify it without
//! parsing that message.

use std::ffi::{c_char, c_int};
use std::ptr;

use unparser_shared::ffi::{self, invalid_argument, FfiError, LastErrorSlot};

use crate::error::ErrorKind;
use crate::model::Document;
use crate::parser::{ErrorMode, ParseOptions};
use crate::render::{JsonFormat, PageMarkerStyle, PageSelection, RenderOptions};

// Thread-local storage for the last error message and its classification. Declared
// here rather than in `unparser-shared` — see that crate's `ffi` module docs for why the slot
// must live in the consuming crate.
thread_local! {
    static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() };
}

unparser_shared::export_last_error_abi!(LAST_ERROR, unpdf_last_error, unpdf_last_error_kind);

/// `unpdf_last_error_kind` value when no error is recorded on this thread.
pub const UNPDF_ERROR_NONE: c_int = unparser_shared::kind::NONE;

// Values 1..=17 are [`ErrorKind`] discriminants — core failure reasons.
// Values 100+ are FFI-boundary reasons with no core `Error` counterpart.

/// An argument was null or not valid UTF-8.
pub const UNPDF_ERROR_INVALID_ARGUMENT: c_int = unparser_shared::kind::INVALID_ARGUMENT;
/// A panic was caught at the FFI boundary.
pub const UNPDF_ERROR_PANIC: c_int = unparser_shared::kind::PANIC;
/// The produced output contains an interior NUL byte and cannot cross the C ABI.
pub const UNPDF_ERROR_INVALID_OUTPUT: c_int = unparser_shared::kind::INVALID_OUTPUT;

/// Classify a core error and render its message, for return from a closure.
fn ffi_err(e: crate::Error) -> FfiError {
    (e.kind() as c_int, e.to_string())
}

/// Classify a JSON serialization failure — producing output is rendering.
fn json_err(e: serde_json::Error) -> FfiError {
    (ErrorKind::Render as c_int, e.to_string())
}

unparser_shared::export_handle! {
    /// Opaque handle to a parsed document.
    handle UnpdfDocument { inner: Document },

    /// Free a document handle.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid pointer returned by `unpdf_parse_file`, `unpdf_parse_bytes`,
    ///   `unpdf_parse_file_with_options`, or `unpdf_parse_bytes_with_options`.
    /// - After calling this function, the handle is invalid and must not be used.
    free unpdf_free_document,
}

/// Flags for markdown rendering.
pub const UNPDF_FLAG_FRONTMATTER: u32 = 1;
pub const UNPDF_FLAG_ESCAPE_SPECIAL: u32 = 2;
/// Bit `4` is retired. It named a paragraph-spacing option that never reached the
/// renderer, so setting it did nothing. Retired bits are not reused: a caller still
/// passing it gets the default rendering, which is what it always produced.
pub const UNPDF_FLAG_PAGE_MARKERS: u32 = 8;
pub const UNPDF_FLAG_REFINE: u32 = 16;

/// Build render options from the flag bitmask.
///
/// Both markdown entry points take the same bitmask, and reading it in two places is how
/// a flag comes to be honoured by one of them and ignored by the other.
fn render_options_from_flags(flags: u32) -> RenderOptions {
    let mut options = RenderOptions::new();
    if flags & UNPDF_FLAG_FRONTMATTER != 0 {
        options.include_frontmatter = true;
    }
    if flags & UNPDF_FLAG_ESCAPE_SPECIAL != 0 {
        options.escape_special_chars = true;
    }
    if flags & UNPDF_FLAG_PAGE_MARKERS != 0 {
        options.page_markers = PageMarkerStyle::Comment;
    }
    #[cfg(feature = "refine")]
    if flags & UNPDF_FLAG_REFINE != 0 {
        options = options.with_refine();
    }
    options
}

/// Deserializable mirror of [`RenderOptions`] for `unpdf_to_markdown_with_options`.
///
/// The flag bitmask above reaches four of `RenderOptions`' settings; this reaches all of
/// the ones a C-ABI caller can meaningfully set, and is the only way to pass the AI refine
/// pass its credentials. Same contract as [`FfiParseOptions`]: every field is optional and
/// an absent one keeps `RenderOptions::default()`'s value.
///
/// # Schema
///
/// ```json
/// {
///   "image_path_prefix": string,
///   "table_fallback": "markdown" | "html" | "ascii",
///   "max_heading_level": number,
///   "include_frontmatter": bool,
///   "preserve_line_breaks": bool,
///   "escape_special_chars": bool,
///   "cleanup_preset": "minimal" | "standard" | "aggressive",
///   "refine": bool,
///   "line_width": number,
///   "page_markers": "none" | "comment",
///   "pages": "all" | { "range": { "from": number, "to": number } } | { "pages": [number, ...] },
///   "ai_refine": {
///     "base_url": string,
///     "api_key": string,
///     "model": string,
///     "instructions": string
///   }
/// }
/// ```
///
/// `ai_refine`'s three credential fields are all required when the object is present —
/// the same rule [`FfiParseOptions`] applies to its own `ai_*` fields.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct FfiRenderOptions {
    image_path_prefix: Option<String>,
    table_fallback: Option<FfiTableFallback>,
    max_heading_level: Option<u8>,
    include_frontmatter: Option<bool>,
    preserve_line_breaks: Option<bool>,
    escape_special_chars: Option<bool>,
    cleanup_preset: Option<FfiCleanupPreset>,
    #[cfg(feature = "refine")]
    refine: Option<bool>,
    line_width: Option<u32>,
    page_markers: Option<FfiPageMarkerStyle>,
    pages: Option<FfiPageSelection>,
    #[cfg(feature = "ai")]
    ai_refine: Option<FfiAiRefine>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FfiTableFallback {
    Markdown,
    Html,
    Ascii,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FfiCleanupPreset {
    Minimal,
    Standard,
    Aggressive,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FfiPageMarkerStyle {
    None,
    Comment,
}

#[cfg(feature = "ai")]
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct FfiAiRefine {
    base_url: String,
    api_key: String,
    model: String,
    instructions: Option<String>,
}

impl TryFrom<FfiRenderOptions> for RenderOptions {
    type Error = String;

    fn try_from(ffi: FfiRenderOptions) -> Result<Self, Self::Error> {
        let mut options = RenderOptions::default();
        if let Some(prefix) = ffi.image_path_prefix {
            options.image_path_prefix = prefix;
        }
        if let Some(fallback) = ffi.table_fallback {
            options.table_fallback = match fallback {
                FfiTableFallback::Markdown => crate::TableFallback::Markdown,
                FfiTableFallback::Html => crate::TableFallback::Html,
                FfiTableFallback::Ascii => crate::TableFallback::Ascii,
            };
        }
        if let Some(level) = ffi.max_heading_level {
            if !(1..=6).contains(&level) {
                return Err(format!("max_heading_level must be 1-6, got {level}"));
            }
            options.max_heading_level = level;
        }
        if let Some(v) = ffi.include_frontmatter {
            options.include_frontmatter = v;
        }
        if let Some(v) = ffi.preserve_line_breaks {
            options.preserve_line_breaks = v;
        }
        if let Some(v) = ffi.escape_special_chars {
            options.escape_special_chars = v;
        }
        if let Some(preset) = ffi.cleanup_preset {
            options = options.with_cleanup_preset(match preset {
                FfiCleanupPreset::Minimal => crate::CleanupPreset::Minimal,
                FfiCleanupPreset::Standard => crate::CleanupPreset::Standard,
                FfiCleanupPreset::Aggressive => crate::CleanupPreset::Aggressive,
            });
        }
        #[cfg(feature = "refine")]
        if ffi.refine == Some(true) {
            options = options.with_refine();
        }
        if let Some(v) = ffi.line_width {
            options.line_width = v;
        }
        if let Some(style) = ffi.page_markers {
            options.page_markers = match style {
                FfiPageMarkerStyle::None => PageMarkerStyle::None,
                FfiPageMarkerStyle::Comment => PageMarkerStyle::Comment,
            };
        }
        if let Some(pages) = ffi.pages {
            options.page_selection = match pages {
                FfiPageSelection::All => PageSelection::All,
                FfiPageSelection::Range { from, to } => PageSelection::Range(from..=to),
                FfiPageSelection::Pages(list) => PageSelection::Pages(list),
            };
        }
        #[cfg(feature = "ai")]
        if let Some(ai) = ffi.ai_refine {
            options =
                options.with_ai_refine(crate::AiConfig::new(ai.base_url, ai.api_key, ai.model));
            if let Some(instructions) = ai.instructions {
                if let Some(opts) = options.ai_refine.as_mut() {
                    opts.instructions = Some(instructions);
                }
            }
        }
        Ok(options)
    }
}

/// Resolve the `options_json` argument for the markdown entry points.
/// A null pointer means "use defaults" — it is not an error.
///
/// # Safety
/// `ptr` must be null or a valid null-terminated UTF-8 string.
unsafe fn render_options_from_json(ptr: *const c_char) -> Result<RenderOptions, FfiError> {
    if ptr.is_null() {
        return Ok(RenderOptions::default());
    }
    let json = unparser_shared::with_c_str!(ptr)?;
    let ffi = serde_json::from_str::<FfiRenderOptions>(json)
        .map_err(|e| invalid_argument(format!("invalid options_json: {e}")))?;
    RenderOptions::try_from(ffi).map_err(|e| invalid_argument(format!("invalid options_json: {e}")))
}

/// JSON format options.
pub const UNPDF_JSON_PRETTY: c_int = 0;
pub const UNPDF_JSON_COMPACT: c_int = 1;

/// Get the version of the library.
///
/// # Safety
///
/// Returns a static string that must not be freed.
#[no_mangle]
pub extern "C" fn unpdf_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Parse a document from a file path.
///
/// # Safety
///
/// - `path` must be a valid null-terminated UTF-8 string.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned handle must be freed with `unpdf_free_document`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_parse_file(path: *const c_char) -> *mut UnpdfDocument {
    LAST_ERROR.with(|slot| slot.clear());

    let result: Result<*mut UnpdfDocument, FfiError> = ffi::catch(|| {
        let path_str = unparser_shared::with_c_str!(path)?;

        crate::parse_file(path_str)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(ffi_err)
    });

    match result {
        Ok(doc) => doc,
        Err(error) => {
            LAST_ERROR.with(|slot| slot.set_error(&error));
            ptr::null_mut()
        }
    }
}

/// Parse a document from a byte buffer.
///
/// # Safety
///
/// - `data` must be a valid pointer to a byte buffer of at least `len` bytes.
/// - Returns null on error. Use `unpdf_last_error` to get the error message.
/// - The returned handle must be freed with `unpdf_free_document`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_parse_bytes(data: *const u8, len: usize) -> *mut UnpdfDocument {
    LAST_ERROR.with(|slot| slot.clear());

    if data.is_null() {
        LAST_ERROR.with(|slot| slot.set_error(&invalid_argument("data is null")));
        return ptr::null_mut();
    }

    let result: Result<*mut UnpdfDocument, FfiError> = ffi::catch(|| {
        let bytes = std::slice::from_raw_parts(data, len);

        crate::parse_bytes(bytes)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(ffi_err)
    });

    match result {
        Ok(doc) => doc,
        Err(error) => {
            LAST_ERROR.with(|slot| slot.set_error(&error));
            ptr::null_mut()
        }
    }
}

/// Deserializable mirror of [`ParseOptions`] for the `_with_options` C ABI entry points.
///
/// Every field is optional; an absent field keeps `ParseOptions::default()`'s value. Kept
/// separate from `ParseOptions` itself so the core Rust API's shape is not dictated by the
/// FFI JSON wire format — the same separation `unpdf-wasm`'s own `ParseOptions` wrapper
/// already keeps from the core type.
///
/// # Schema
///
/// ```json
/// {
///   "error_mode": "strict" | "lenient",
///   "extract_text": bool,
///   "extract_resources": bool,
///   "min_image_dimension": number,
///   "parallel": bool,
///   "pages": "all" | { "range": { "from": number, "to": number } } | { "pages": [number, ...] },
///   "password": string,
///   "suppress_low_confidence_ocr": bool,
///   "ai_base_url": string,
///   "ai_api_key": string,
///   "ai_model": string,
///   "ai_image_scope": "all" | "low_confidence_pages_only"
/// }
/// ```
///
/// The three `ai_*` credential fields go together: supplying only some of them is
/// rejected rather than silently ignored, so a typo cannot look like a model that
/// found nothing to say. Supplying none of them leaves AI disabled, which is the
/// default. `ai_image_scope` without them is likewise rejected.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct FfiParseOptions {
    error_mode: Option<FfiErrorMode>,
    extract_text: Option<bool>,
    extract_resources: Option<bool>,
    min_image_dimension: Option<u32>,
    parallel: Option<bool>,
    pages: Option<FfiPageSelection>,
    password: Option<String>,
    suppress_low_confidence_ocr: Option<bool>,
    #[cfg(feature = "ai")]
    ai_base_url: Option<String>,
    #[cfg(feature = "ai")]
    ai_api_key: Option<String>,
    #[cfg(feature = "ai")]
    ai_model: Option<String>,
    #[cfg(feature = "ai")]
    ai_image_scope: Option<FfiImageScope>,
}

#[cfg(feature = "ai")]
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FfiImageScope {
    All,
    LowConfidencePagesOnly,
}

#[cfg(feature = "ai")]
impl From<FfiImageScope> for crate::ImageScope {
    fn from(scope: FfiImageScope) -> Self {
        match scope {
            FfiImageScope::All => crate::ImageScope::All,
            FfiImageScope::LowConfidencePagesOnly => crate::ImageScope::LowConfidencePagesOnly,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FfiErrorMode {
    Strict,
    Lenient,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FfiPageSelection {
    All,
    Range { from: u32, to: u32 },
    Pages(Vec<u32>),
}

impl TryFrom<FfiParseOptions> for ParseOptions {
    type Error = String;

    fn try_from(ffi: FfiParseOptions) -> Result<Self, Self::Error> {
        let mut options = ParseOptions::default();
        if let Some(mode) = ffi.error_mode {
            options.error_mode = match mode {
                FfiErrorMode::Strict => ErrorMode::Strict,
                FfiErrorMode::Lenient => ErrorMode::Lenient,
            };
        }
        if let Some(v) = ffi.extract_text {
            options.extract_text = v;
        }
        if let Some(v) = ffi.extract_resources {
            options.extract_resources = v;
        }
        if let Some(v) = ffi.min_image_dimension {
            options.min_image_dimension = v;
        }
        if let Some(v) = ffi.parallel {
            options.parallel = v;
        }
        if let Some(pages) = ffi.pages {
            options.pages = match pages {
                FfiPageSelection::All => PageSelection::All,
                FfiPageSelection::Range { from, to } => PageSelection::Range(from..=to),
                FfiPageSelection::Pages(list) => PageSelection::Pages(list),
            };
        }
        if ffi.password.is_some() {
            options.password = ffi.password;
        }
        if let Some(v) = ffi.suppress_low_confidence_ocr {
            options.suppress_low_confidence_ocr = v;
        }

        #[cfg(feature = "ai")]
        {
            match (ffi.ai_base_url, ffi.ai_api_key, ffi.ai_model) {
                (None, None, None) => {
                    if ffi.ai_image_scope.is_some() {
                        return Err(
                            "ai_image_scope requires ai_base_url, ai_api_key and ai_model".into(),
                        );
                    }
                }
                (Some(base_url), Some(api_key), Some(model)) => {
                    let mut config = crate::AiConfig::new(base_url, api_key, model);
                    if let Some(scope) = ffi.ai_image_scope {
                        config.image_scope = scope.into();
                    }
                    options.ai = Some(config);
                }
                (base_url, api_key, model) => {
                    let missing: Vec<&str> = [
                        ("ai_base_url", base_url.is_none()),
                        ("ai_api_key", api_key.is_none()),
                        ("ai_model", model.is_none()),
                    ]
                    .into_iter()
                    .filter_map(|(field, absent)| absent.then_some(field))
                    .collect();
                    return Err(format!(
                        "incomplete AI configuration — also required: {}",
                        missing.join(", ")
                    ));
                }
            }
        }

        Ok(options)
    }
}

/// Resolve the `options_json` argument shared by the `_with_options` entry points below.
/// A null pointer means "use defaults" — it is not an error.
///
/// # Safety
/// `ptr` must be null or a valid null-terminated UTF-8 string.
unsafe fn parse_options_from_json(ptr: *const c_char) -> Result<ParseOptions, FfiError> {
    if ptr.is_null() {
        return Ok(ParseOptions::default());
    }
    let json = unparser_shared::with_c_str!(ptr)?;
    let ffi = serde_json::from_str::<FfiParseOptions>(json)
        .map_err(|e| invalid_argument(format!("invalid options_json: {e}")))?;
    ParseOptions::try_from(ffi).map_err(|e| invalid_argument(format!("invalid options_json: {e}")))
}

/// Parse a document from a file path, with options.
///
/// # Safety
///
/// - `path` must be a valid null-terminated UTF-8 string.
/// - `options_json` may be null (equivalent to default options) or a valid null-terminated
///   UTF-8 JSON string matching [`FfiParseOptions`]'s schema (see that type's docs).
/// - Returns null on error, including malformed `options_json`. Use `unpdf_last_error`.
/// - The returned handle must be freed with `unpdf_free_document`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_parse_file_with_options(
    path: *const c_char,
    options_json: *const c_char,
) -> *mut UnpdfDocument {
    LAST_ERROR.with(|slot| slot.clear());

    let result: Result<*mut UnpdfDocument, FfiError> = ffi::catch(|| {
        let path_str = unparser_shared::with_c_str!(path)?;
        let options = parse_options_from_json(options_json)?;

        crate::parse_file_with_options(path_str, options)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(ffi_err)
    });

    match result {
        Ok(doc) => doc,
        Err(error) => {
            LAST_ERROR.with(|slot| slot.set_error(&error));
            ptr::null_mut()
        }
    }
}

/// Parse a document from a byte buffer, with options.
///
/// # Safety
///
/// - `data` must be a valid pointer to a byte buffer of at least `len` bytes.
/// - `options_json` may be null (equivalent to default options) or a valid null-terminated
///   UTF-8 JSON string matching [`FfiParseOptions`]'s schema (see that type's docs).
/// - Returns null on error, including malformed `options_json`. Use `unpdf_last_error`.
/// - The returned handle must be freed with `unpdf_free_document`.
#[no_mangle]
pub unsafe extern "C" fn unpdf_parse_bytes_with_options(
    data: *const u8,
    len: usize,
    options_json: *const c_char,
) -> *mut UnpdfDocument {
    LAST_ERROR.with(|slot| slot.clear());

    if data.is_null() {
        LAST_ERROR.with(|slot| slot.set_error(&invalid_argument("data is null")));
        return ptr::null_mut();
    }

    let result: Result<*mut UnpdfDocument, FfiError> = ffi::catch(|| {
        let bytes = std::slice::from_raw_parts(data, len);
        let options = parse_options_from_json(options_json)?;

        crate::parse_bytes_with_options(bytes, options)
            .map(|doc| Box::into_raw(Box::new(UnpdfDocument { inner: doc })))
            .map_err(ffi_err)
    });

    match result {
        Ok(doc) => doc,
        Err(error) => {
            LAST_ERROR.with(|slot| slot.set_error(&error));
            ptr::null_mut()
        }
    }
}

unparser_shared::export_string_getter!(
    /// Convert a document to Markdown.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `flags` is a bitwise OR of `UNPDF_FLAG_*` constants.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_to_markdown(doc: UnpdfDocument, flags: u32),
    {
        let document = &(*doc).inner;
        let options = render_options_from_flags(flags);
        crate::render::to_markdown(document, &options).map_err(ffi_err)
    }
);

unparser_shared::export_string_getter!(
    /// Convert a document to Markdown, with options.
    ///
    /// The counterpart to `unpdf_to_markdown`'s flag bitmask, which reaches only four
    /// settings and cannot carry the AI refine pass's credentials. `unpdf_to_markdown`
    /// keeps working unchanged; this is the surface for everything the bitmask cannot
    /// express.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `options_json` may be null (equivalent to default options) or a valid
    ///   null-terminated UTF-8 JSON string matching [`FfiRenderOptions`]'s schema
    ///   (see that type's docs).
    /// - Returns null on error, including malformed `options_json`. Use `unpdf_last_error`.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_to_markdown_with_options(doc: UnpdfDocument, options_json: *const c_char),
    {
        let document = &(*doc).inner;
        let options = render_options_from_json(options_json)?;
        crate::render::to_markdown(document, &options).map_err(ffi_err)
    }
);

unparser_shared::export_string_getter!(
    /// Convert a document to plain text.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_to_text(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        let options = RenderOptions::default();
        crate::render::to_text(document, &options).map_err(ffi_err)
    }
);

unparser_shared::export_string_getter!(
    /// Convert a document to JSON.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `format` is one of `UNPDF_JSON_PRETTY` or `UNPDF_JSON_COMPACT`.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_to_json(doc: UnpdfDocument, format: c_int),
    {
        let document = &(*doc).inner;
        let json_format = if format == UNPDF_JSON_COMPACT {
            JsonFormat::Compact
        } else {
            JsonFormat::Pretty
        };
        crate::render::to_json(document, json_format).map_err(ffi_err)
    }
);

unparser_shared::export_string_getter!(
    /// Get the plain text content of a document.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns null on error.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_plain_text(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        Ok(document.plain_text())
    }
);

unparser_shared::export_count_getter!(
    /// Get the number of sections (pages) in a document.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns -1 on error.
    LAST_ERROR,
    unpdf_section_count(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        Ok(document.pages.len() as c_int)
    }
);

unparser_shared::export_count_getter!(
    /// Get the number of extracted resources (images) in a document.
    ///
    /// Semantics: counts entries in the document's resource inventory, which is
    /// populated only when parsing runs with `extract_resources` enabled. The FFI
    /// parse entry points use default options where resource extraction is **off**
    /// (since 0.4.0, to bound peak memory), so this returns `0` for every document
    /// parsed through `unpdf_parse_file` / `unpdf_parse_bytes`. It is **not** a
    /// count of images referenced by page content streams — for detecting
    /// image-only (scanned) pages use `unpdf_page_stats` or
    /// `unpdf_get_extraction_quality` instead.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns -1 on error.
    LAST_ERROR,
    unpdf_resource_count(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        Ok(document.resources.len() as c_int)
    }
);

unparser_shared::export_optional_string_getter!(
    /// Get the document title.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns null if no title is set — with `unpdf_last_error_kind` left at
    ///   `UNPDF_ERROR_NONE`, since an absent title is not a failure. A null return paired
    ///   with a non-zero kind means the title could not be produced (for instance
    ///   `UNPDF_ERROR_INVALID_OUTPUT` when it holds an interior NUL byte).
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_get_title(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        Ok(document.metadata.title.clone())
    }
);

unparser_shared::export_optional_string_getter!(
    /// Get the document author.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns null if no author is set — with `unpdf_last_error_kind` left at
    ///   `UNPDF_ERROR_NONE`, since an absent author is not a failure. A null return paired
    ///   with a non-zero kind means the author could not be produced (for instance
    ///   `UNPDF_ERROR_INVALID_OUTPUT` when it holds an interior NUL byte).
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_get_author(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        Ok(document.metadata.author.clone())
    }
);

unparser_shared::export_string_getter!(
    /// Get all resource IDs as a JSON array.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_get_resource_ids(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        let ids: Vec<&String> = document.resources.keys().collect();
        serde_json::to_string(&ids).map_err(json_err)
    }
);

unparser_shared::export_string_getter!(
    /// Get extraction quality diagnostics as a JSON object.
    ///
    /// Fields are those of `ExtractionQuality`: `char_count`, `word_count`,
    /// `replacement_char_count`, `encrypted`, `is_scan_pdf`, `suppressed_ocr_pages`,
    /// `suppressed_text_runs`, `undecodable_content_streams`, `pages_incomplete`,
    /// `declared_page_count`, `unresolved_page_nodes`, `skipped_object_count`,
    /// `unsupported_image_count`, `ai_fallback_count`. `is_scan_pdf` is `true` when sampled
    /// pages draw images with no text-showing operators — the document-level
    /// "scanned document, OCR required" signal. For page-level discrimination
    /// (mixed documents) use `unpdf_page_stats`.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_get_extraction_quality(doc: UnpdfDocument),
    {
        let document = &(*doc).inner;
        serde_json::to_string(&document.extraction_quality).map_err(json_err)
    }
);

unparser_shared::export_string_getter!(
    /// Get per-page content-stream operator statistics as a JSON object.
    ///
    /// Returns `{"page":N,"text_op_count":N,"image_op_count":N,"ocr_text_suppressed":bool,
    /// "suppressed_text_runs":N,"undecodable_content_streams":N}`.
    ///
    /// - `suppressed_text_runs` / `undecodable_content_streams`: this page's share of the
    ///   document-level counts of the same name in `unpdf_get_extraction_quality`.
    /// - `text_op_count`: number of text-showing operators (`Tj`/`TJ`/`'`/`"`).
    /// - `image_op_count`: number of XObject `Do` invocations (mostly images;
    ///   may include form XObjects).
    /// - Both `0` → genuinely blank page. `text_op_count == 0` with
    ///   `image_op_count > 0` → image-only (scanned) page, OCR required.
    /// - Note: a *searchable* scan (page image plus an invisible OCR text layer)
    ///   reports `text_op_count > 0` — combine with `ocr_text_suppressed` to
    ///   detect scans whose OCR layer was dropped as unreadable.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `page_number` is 1-indexed.
    /// - Returns null if the page is out of range or on error.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_page_stats(doc: UnpdfDocument, page_number: c_int),
    {
        let document = &(*doc).inner;
        let page = document
            .pages
            .iter()
            .find(|p| p.number == page_number as u32)
            .ok_or_else(|| {
                (
                    ErrorKind::PageOutOfRange as c_int,
                    format!(
                        "page {} out of range (document has {} pages)",
                        page_number,
                        document.pages.len()
                    ),
                )
            })?;
        serde_json::to_string(&serde_json::json!({
            "page": page.number,
            "text_op_count": page.text_op_count,
            "image_op_count": page.image_op_count,
            "ocr_text_suppressed": page.ocr_text_suppressed,
            "suppressed_text_runs": page.suppressed_text_runs,
            "undecodable_content_streams": page.undecodable_content_streams,
        }))
        .map_err(json_err)
    }
);

unparser_shared::export_string_getter!(
    /// Get resource metadata as JSON (without binary data).
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `resource_id` must be a valid null-terminated UTF-8 string.
    /// - Returns null if resource not found or on error.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_get_resource_info(doc: UnpdfDocument, resource_id: *const c_char),
    {
        let id_str = unparser_shared::with_c_str!(resource_id)?;

        let document = &(*doc).inner;

        match document.resources.get(id_str) {
            Some(resource) => {
                let info = serde_json::json!({
                    "id": id_str,
                    "type": resource.resource_type,
                    "filename": resource.filename,
                    "mime_type": resource.mime_type,
                    "size": resource.size(),
                    "width": resource.width,
                    "height": resource.height,
                });
                serde_json::to_string(&info).map_err(json_err)
            }
            None => Err((
                ErrorKind::ResourceNotFound as c_int,
                format!("resource not found: {}", id_str),
            )),
        }
    }
);

unparser_shared::export_bytes_getter!(
    /// Get resource binary data.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `resource_id` must be a valid null-terminated UTF-8 string.
    /// - `out_len` must be a valid pointer to receive the data length.
    /// - Returns null if resource not found or on error.
    /// - The returned pointer must be freed with `unpdf_free_bytes`.
    LAST_ERROR,
    unpdf_get_resource_data(doc: UnpdfDocument, resource_id, out out_len),
    {
        let id_str = unparser_shared::ffi::c_str_utf8(resource_id)?;

        let document = &(*doc).inner;

        match document.resources.get(id_str) {
            Some(resource) => Ok(resource.data.clone()),
            None => Err((
                ErrorKind::ResourceNotFound as c_int,
                format!("resource not found: {}", id_str),
            )),
        }
    }
);

unparser_shared::export_string_getter!(
    /// Convert a single page to Markdown.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `page_num` is 1-indexed.
    /// - `flags` is a bitwise OR of `UNPDF_FLAG_*` constants.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_page_to_markdown(doc: UnpdfDocument, page_num: c_int, flags: u32),
    {
        let document = &(*doc).inner;
        let page = document.get_page(page_num as u32).ok_or_else(|| {
            (
                ErrorKind::PageOutOfRange as c_int,
                format!(
                    "page {} out of range (document has {} pages)",
                    page_num,
                    document.page_count()
                ),
            )
        })?;

        let options = render_options_from_flags(flags);

        // Create a single-page document for rendering
        let mut single_page_doc = Document::new();
        single_page_doc.add_page(page.clone());

        crate::render::to_markdown(&single_page_doc, &options).map_err(ffi_err)
    }
);

unparser_shared::export_string_getter!(
    /// Get the plain text of a single page.
    ///
    /// # Safety
    ///
    /// - `doc` must be a valid document handle.
    /// - `page_num` is 1-indexed.
    /// - Returns null on error. Use `unpdf_last_error` to get the error message.
    /// - The returned string must be freed with `unpdf_free_string`.
    LAST_ERROR,
    unpdf_page_to_text(doc: UnpdfDocument, page_num: c_int),
    {
        let document = &(*doc).inner;
        let page = document.get_page(page_num as u32).ok_or_else(|| {
            (
                ErrorKind::PageOutOfRange as c_int,
                format!(
                    "page {} out of range (document has {} pages)",
                    page_num,
                    document.page_count()
                ),
            )
        })?;

        Ok::<String, FfiError>(page.plain_text())
    }
);

unparser_shared::export_free_string!(
    /// Free a string allocated by this library.
    ///
    /// # Safety
    ///
    /// - `s` must be a pointer returned by an unpdf function, or null.
    /// - After calling this function, the pointer is invalid and must not be used.
    unpdf_free_string
);

unparser_shared::export_free_bytes!(
    /// Free binary data allocated by `unpdf_get_resource_data`.
    ///
    /// # Safety
    ///
    /// - `data` must be a pointer returned by `unpdf_get_resource_data`, or null.
    /// - `len` must be the length returned by `unpdf_get_resource_data`.
    /// - After calling this function, the pointer is invalid and must not be used.
    unpdf_free_bytes
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn test_version() {
        let version = unpdf_version();
        assert!(!version.is_null());
        let version_str = unsafe { CStr::from_ptr(version) }.to_str().unwrap();
        assert!(!version_str.is_empty());
    }

    #[test]
    fn test_parse_null_path() {
        let doc = unsafe { unpdf_parse_file(ptr::null()) };
        assert!(doc.is_null());

        let error = unpdf_last_error();
        assert!(!error.is_null());
    }

    #[test]
    fn test_parse_invalid_path() {
        let path = CString::new("nonexistent.pdf").unwrap();
        let doc = unsafe { unpdf_parse_file(path.as_ptr()) };
        assert!(doc.is_null());

        let error = unpdf_last_error();
        assert!(!error.is_null());
    }

    #[test]
    fn test_null_document_operations() {
        let md = unsafe { unpdf_to_markdown(ptr::null(), 0) };
        assert!(md.is_null());

        let text = unsafe { unpdf_to_text(ptr::null()) };
        assert!(text.is_null());

        let json = unsafe { unpdf_to_json(ptr::null(), 0) };
        assert!(json.is_null());

        let count = unsafe { unpdf_section_count(ptr::null()) };
        assert_eq!(count, -1);

        let res_count = unsafe { unpdf_resource_count(ptr::null()) };
        assert_eq!(res_count, -1);
    }

    #[test]
    fn test_page_null_document() {
        let md = unsafe { unpdf_page_to_markdown(ptr::null(), 1, 0) };
        assert!(md.is_null());

        let text = unsafe { unpdf_page_to_text(ptr::null(), 1) };
        assert!(text.is_null());
    }

    /// A null return does not always mean failure: an absent title is not an error.
    /// The kind channel is what lets a caller tell the two apart.
    #[test]
    fn test_absent_metadata_is_not_reported_as_a_failure() {
        let doc = Box::into_raw(Box::new(UnpdfDocument {
            inner: Document::new(),
        }));

        assert!(unsafe { unpdf_get_title(doc) }.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_NONE);

        assert!(unsafe { unpdf_get_author(doc) }.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_NONE);

        unsafe { unpdf_free_document(doc) };
    }

    /// The counterpart: metadata that *exists* but cannot cross the ABI must not be
    /// reported as absent. Both cases return null, so the kind is the only thing that
    /// separates "there is nothing" from "we could not give it to you".
    #[test]
    fn test_unrepresentable_metadata_is_not_reported_as_absent() {
        let mut document = Document::new();
        document.metadata.title = Some("has\0interior nul".to_string());
        document.metadata.author = Some("also\0bad".to_string());
        let doc = Box::into_raw(Box::new(UnpdfDocument { inner: document }));

        assert!(unsafe { unpdf_get_title(doc) }.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_INVALID_OUTPUT);

        assert!(unsafe { unpdf_get_author(doc) }.is_null());
        assert_eq!(unpdf_last_error_kind(), UNPDF_ERROR_INVALID_OUTPUT);

        unsafe { unpdf_free_document(doc) };
    }

    /// `out_len` is written only once the call has reached the point of producing a
    /// buffer. A rejected argument leaves the caller's variable alone, so a caller that
    /// seeded it can tell "not attempted" from "attempted and produced nothing".
    #[test]
    fn test_rejected_arguments_leave_out_len_untouched() {
        let doc = Box::into_raw(Box::new(UnpdfDocument {
            inner: Document::new(),
        }));
        let id = CString::new("image1").unwrap();
        const SEEDED: usize = 0xDEAD;

        let mut out_len: usize = SEEDED;
        assert!(
            unsafe { unpdf_get_resource_data(ptr::null(), id.as_ptr(), &mut out_len) }.is_null()
        );
        assert_eq!(out_len, SEEDED, "a null document must not write out_len");

        assert!(unsafe { unpdf_get_resource_data(doc, ptr::null(), &mut out_len) }.is_null());
        assert_eq!(out_len, SEEDED, "a null resource_id must not write out_len");

        // A resource that is merely absent *is* looked up, so the length is zeroed.
        assert!(unsafe { unpdf_get_resource_data(doc, id.as_ptr(), &mut out_len) }.is_null());
        assert_eq!(out_len, 0, "a lookup that failed reports zero length");
        assert_eq!(
            unpdf_last_error_kind(),
            ErrorKind::ResourceNotFound as c_int
        );

        unsafe { unpdf_free_document(doc) };
    }

    #[test]
    fn test_free_null() {
        // Should not crash
        unsafe {
            unpdf_free_document(ptr::null_mut());
            unpdf_free_string(ptr::null_mut());
        }
    }
}
