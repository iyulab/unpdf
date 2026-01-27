/**
 * unpdf - PDF content extraction library
 * C API Header
 *
 * This header provides C bindings for the unpdf Rust library.
 */

#ifndef UNPDF_H
#define UNPDF_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Result structure returned by unpdf functions.
 */
typedef struct {
    /** Whether the operation succeeded. */
    bool success;
    /** The result data (null if failed). Must be freed with unpdf_free_string. */
    char* data;
    /** Error message (null if succeeded). Must be freed with unpdf_free_string. */
    char* error;
} UnpdfResult;

/**
 * Convert a PDF file to Markdown.
 *
 * @param path Path to the PDF file (UTF-8 encoded, null-terminated).
 * @return Result containing the Markdown content or error message.
 *         Must be freed with unpdf_free_result.
 */
UnpdfResult unpdf_to_markdown(const char* path);

/**
 * Convert a PDF file to plain text.
 *
 * @param path Path to the PDF file (UTF-8 encoded, null-terminated).
 * @return Result containing the text content or error message.
 *         Must be freed with unpdf_free_result.
 */
UnpdfResult unpdf_to_text(const char* path);

/**
 * Convert a PDF file to JSON.
 *
 * @param path Path to the PDF file (UTF-8 encoded, null-terminated).
 * @param pretty Whether to format the JSON with indentation.
 * @return Result containing the JSON content or error message.
 *         Must be freed with unpdf_free_result.
 */
UnpdfResult unpdf_to_json(const char* path, bool pretty);

/**
 * Get document information as JSON.
 *
 * @param path Path to the PDF file (UTF-8 encoded, null-terminated).
 * @return Result containing document metadata as JSON or error message.
 *         Must be freed with unpdf_free_result.
 */
UnpdfResult unpdf_get_info(const char* path);

/**
 * Get the page count of a PDF file.
 *
 * @param path Path to the PDF file (UTF-8 encoded, null-terminated).
 * @return Number of pages, or -1 on error.
 */
int32_t unpdf_get_page_count(const char* path);

/**
 * Check if a file is a valid PDF.
 *
 * @param path Path to the file (UTF-8 encoded, null-terminated).
 * @return true if the file is a valid PDF, false otherwise.
 */
bool unpdf_is_pdf(const char* path);

/**
 * Free a result returned by any unpdf function.
 *
 * @param result The result to free.
 */
void unpdf_free_result(UnpdfResult result);

/**
 * Free a string allocated by unpdf.
 *
 * @param ptr The string to free.
 */
void unpdf_free_string(char* ptr);

/**
 * Get the version of the unpdf library.
 *
 * @return Version string (statically allocated, do not free).
 */
const char* unpdf_version(void);

#ifdef __cplusplus
}
#endif

#endif /* UNPDF_H */
