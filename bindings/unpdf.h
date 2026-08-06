/**
 * unpdf - PDF content extraction library
 * C API Header
 *
 * Mirrors the C-ABI surface exported from src/ffi.rs (build with
 * `cargo build --release --features ffi`). The API is handle-based:
 * parse once into an UnpdfDocument*, query it, then free it.
 *
 * Memory rules:
 *  - Every char* returned by a function documented as "must be freed" is
 *    owned by the caller and released with unpdf_free_string().
 *  - Byte buffers from unpdf_get_resource_data() are released with
 *    unpdf_free_bytes().
 *  - Document handles are released with unpdf_free_document().
 *  - unpdf_version() / unpdf_last_error() return borrowed pointers —
 *    do not free them.
 */

#ifndef UNPDF_H
#define UNPDF_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque handle to a parsed document. */
typedef struct UnpdfDocument UnpdfDocument;

/* Flags for unpdf_to_markdown / unpdf_page_to_markdown. */
#define UNPDF_FLAG_FRONTMATTER       1u
#define UNPDF_FLAG_ESCAPE_SPECIAL    2u
/* Bit 4 is retired: it named a paragraph-spacing option that never reached the
   renderer. Retired bits are not reused; passing it yields the default rendering. */
#define UNPDF_FLAG_PAGE_MARKERS      8u

/* Format selector for unpdf_to_json. */
#define UNPDF_JSON_PRETTY  0
#define UNPDF_JSON_COMPACT 1

/**
 * Classification returned by unpdf_last_error_kind(), so callers can branch on
 * why a call failed without parsing the error message.
 *
 * Values 1..17 mirror the library's own failure reasons one-to-one; values 100+
 * are raised at the FFI boundary and have no library-side counterpart. These
 * numbers are part of the ABI: a new reason takes the next free number and
 * existing ones are never renumbered, so treat an unrecognised value as a
 * generic failure rather than an error.
 */
typedef enum UnpdfErrorKind {
    UNPDF_ERROR_NONE                = 0,   /* last call succeeded */
    UNPDF_ERROR_OTHER               = 1,   /* no more specific classification */
    UNPDF_ERROR_IO                  = 2,
    UNPDF_ERROR_UNKNOWN_FORMAT      = 3,   /* not a valid PDF */
    UNPDF_ERROR_UNSUPPORTED_VERSION = 4,
    UNPDF_ERROR_PDF_PARSE           = 5,
    UNPDF_ERROR_ENCRYPTED           = 6,   /* password required */
    UNPDF_ERROR_INVALID_PASSWORD    = 7,
    UNPDF_ERROR_CORRUPTED           = 8,   /* malformed structure */
    UNPDF_ERROR_MISSING_OBJECT      = 9,
    UNPDF_ERROR_FONT_DECODE         = 10,
    UNPDF_ERROR_IMAGE_EXTRACT       = 11,
    UNPDF_ERROR_RENDER              = 12,
    UNPDF_ERROR_TEXT_EXTRACT        = 13,
    UNPDF_ERROR_PAGE_OUT_OF_RANGE   = 14,
    UNPDF_ERROR_INVALID_PAGE_RANGE  = 15,
    UNPDF_ERROR_RESOURCE_NOT_FOUND  = 16,
    UNPDF_ERROR_ENCODING            = 17,

    UNPDF_ERROR_INVALID_ARGUMENT    = 100, /* null argument, or not valid UTF-8 */
    UNPDF_ERROR_PANIC               = 101, /* panic caught at the FFI boundary */
    UNPDF_ERROR_INVALID_OUTPUT      = 102  /* output holds an interior NUL byte */
} UnpdfErrorKind;

/**
 * Get the library version.
 * @return Statically allocated version string — do not free.
 */
const char* unpdf_version(void);

/**
 * Get the last error message for the calling thread.
 * @return Borrowed pointer valid until the next unpdf call on this thread,
 *         or NULL if no error is recorded. Do not free.
 */
const char* unpdf_last_error(void);

/**
 * Classify the last error for the calling thread.
 *
 * Written in lockstep with unpdf_last_error(): every call that records a message
 * records a kind, and a successful call clears both. Use this instead of matching
 * on message text — for example to tell UNPDF_ERROR_ENCRYPTED (ask for a password)
 * apart from UNPDF_ERROR_CORRUPTED (the file is damaged) when both surface to the
 * user as "extraction failed".
 *
 * @return An UnpdfErrorKind value, or UNPDF_ERROR_NONE if the last call succeeded.
 */
int unpdf_last_error_kind(void);

/**
 * Parse a document from a file path.
 * @param path UTF-8, null-terminated path.
 * @return Document handle, or NULL on error (see unpdf_last_error).
 *         Must be freed with unpdf_free_document.
 */
UnpdfDocument* unpdf_parse_file(const char* path);

/**
 * Parse a document from a byte buffer.
 * @param data Pointer to at least `len` bytes.
 * @param len  Buffer length in bytes.
 * @return Document handle, or NULL on error. Must be freed with
 *         unpdf_free_document.
 */
UnpdfDocument* unpdf_parse_bytes(const uint8_t* data, size_t len);

/** Free a document handle. Safe to call with NULL. */
void unpdf_free_document(UnpdfDocument* doc);

/**
 * Convert the document to Markdown.
 * @param flags Bitwise OR of UNPDF_FLAG_* values.
 * @return Markdown string (must be freed with unpdf_free_string), or NULL.
 */
char* unpdf_to_markdown(const UnpdfDocument* doc, uint32_t flags);

/** Convert the document to plain text. Free with unpdf_free_string. */
char* unpdf_to_text(const UnpdfDocument* doc);

/**
 * Convert the document to JSON.
 * @param format UNPDF_JSON_PRETTY or UNPDF_JSON_COMPACT.
 * @return JSON string (must be freed with unpdf_free_string), or NULL.
 */
char* unpdf_to_json(const UnpdfDocument* doc, int format);

/** Get the plain text content. Free with unpdf_free_string. */
char* unpdf_plain_text(const UnpdfDocument* doc);

/** Number of sections (pages), or -1 on error. */
int unpdf_section_count(const UnpdfDocument* doc);

/**
 * Number of extracted resources (images), or -1 on error.
 *
 * Counts the resource inventory, which is populated only when parsing runs
 * with resource extraction enabled. The FFI parse entry points use default
 * options where resource extraction is OFF (since 0.4.0), so this returns 0
 * for documents parsed through this API. It is NOT a count of images
 * referenced by page content streams — use unpdf_page_stats for that.
 */
int unpdf_resource_count(const UnpdfDocument* doc);

/** Document title, or NULL if absent. Free with unpdf_free_string. */
char* unpdf_get_title(const UnpdfDocument* doc);

/** Document author, or NULL if absent. Free with unpdf_free_string. */
char* unpdf_get_author(const UnpdfDocument* doc);

/**
 * Extraction quality diagnostics as a JSON object.
 *
 * Fields: char_count, word_count, replacement_char_count, encrypted,
 * is_scan_pdf, suppressed_ocr_pages, pages_incomplete, declared_page_count,
 * unresolved_page_nodes, skipped_object_count. `is_scan_pdf` is true when
 * sampled pages draw images with no text-showing operators — the document-level
 * "scanned document, OCR required" signal. For page-level discrimination
 * (mixed documents) use unpdf_page_stats.
 *
 * pages_incomplete is true when the document was damaged and pages are missing
 * from the output — parsing "succeeded" over a short page set, which is
 * otherwise indistinguishable from a genuinely shorter document. Compare
 * declared_page_count (the document's own /Count, null when unreadable) with
 * unpdf_section_count. unresolved_page_nodes counts unreadable page-tree
 * *nodes*, not lost pages: one unusable node drops its whole subtree, so treat
 * any non-zero value as "incomplete" and nothing more. skipped_object_count is
 * a damage indicator only — most skipped objects cost no page.
 *
 * Fields are only ever added here, never removed or renamed; a consumer that
 * parses this JSON should ignore unknown fields.
 *
 * @return JSON string (must be freed with unpdf_free_string), or NULL.
 */
char* unpdf_get_extraction_quality(const UnpdfDocument* doc);

/**
 * Per-page content-stream operator statistics as a JSON object:
 * {"page":N,"text_op_count":N,"image_op_count":N,"ocr_text_suppressed":bool}
 *
 * text_op_count counts text-showing operators (Tj/TJ/'/"); image_op_count
 * counts XObject Do invocations (mostly images; may include form XObjects).
 * Both 0 -> genuinely blank page. text_op_count == 0 with image_op_count > 0
 * -> image-only (scanned) page, OCR required.
 *
 * Note: a *searchable* scan (page image plus an invisible OCR text layer)
 * reports text_op_count > 0 — combine with ocr_text_suppressed to detect
 * scans whose OCR layer was dropped as unreadable.
 *
 * @param page_number 1-indexed page number.
 * @return JSON string (must be freed with unpdf_free_string), or NULL if the
 *         page is out of range.
 */
char* unpdf_page_stats(const UnpdfDocument* doc, int page_number);

/**
 * All resource IDs as a JSON array of strings.
 * @return JSON string (must be freed with unpdf_free_string), or NULL.
 */
char* unpdf_get_resource_ids(const UnpdfDocument* doc);

/**
 * Resource metadata as JSON (without binary data).
 * @param resource_id UTF-8, null-terminated resource ID.
 * @return JSON string (must be freed with unpdf_free_string), or NULL if the
 *         resource is not found.
 */
char* unpdf_get_resource_info(const UnpdfDocument* doc, const char* resource_id);

/**
 * Resource binary data.
 * @param resource_id UTF-8, null-terminated resource ID.
 * @param out_len Receives the buffer length in bytes.
 * @return Byte buffer (must be freed with unpdf_free_bytes), or NULL.
 */
uint8_t* unpdf_get_resource_data(const UnpdfDocument* doc,
                                 const char* resource_id,
                                 size_t* out_len);

/**
 * Convert a single page to Markdown.
 * @param page_num 1-indexed page number.
 * @param flags Bitwise OR of UNPDF_FLAG_* values.
 * @return Markdown string (must be freed with unpdf_free_string), or NULL.
 */
char* unpdf_page_to_markdown(const UnpdfDocument* doc, int page_num, uint32_t flags);

/**
 * Get plain text of a single page.
 * @param page_num 1-indexed page number.
 * @return Text string (must be freed with unpdf_free_string), or NULL.
 */
char* unpdf_page_to_text(const UnpdfDocument* doc, int page_num);

/** Free a string allocated by the library. Safe to call with NULL. */
void unpdf_free_string(char* s);

/** Free a byte buffer allocated by the library. */
void unpdf_free_bytes(uint8_t* data, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* UNPDF_H */
