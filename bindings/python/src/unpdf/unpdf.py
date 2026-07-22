"""
High-level Python API for unpdf.
"""

import ctypes
import json
from typing import Any

from ._native import get_library, UNPDF_JSON_PRETTY, UNPDF_JSON_COMPACT


def _encode_path(path: str) -> bytes:
    """Encode a path string to bytes for FFI."""
    return path.encode("utf-8")


def _check_last_error(lib: ctypes.CDLL) -> str:
    """Get the last error message from the native library."""
    err = lib.unpdf_last_error()
    if err:
        return err.decode("utf-8")
    return "Unknown error"


def _parse_file(lib: ctypes.CDLL, path: str) -> ctypes.c_void_p:
    """Parse a file and return the document handle. Raises on failure."""
    handle = lib.unpdf_parse_file(_encode_path(path))
    if not handle:
        raise RuntimeError(f"unpdf error: {_check_last_error(lib)}")
    return handle


def to_markdown(path: str, flags: int = 0) -> str:
    """
    Convert a PDF file to Markdown format.

    Args:
        path: Path to the PDF file.
        flags: Bitwise OR of UNPDF_FLAG_* constants (optional).

    Returns:
        The extracted content as Markdown.

    Raises:
        RuntimeError: If conversion fails.
    """
    lib = get_library()
    handle = _parse_file(lib, path)
    try:
        result = lib.unpdf_to_markdown(handle, flags)
        if not result:
            raise RuntimeError(f"unpdf error: {_check_last_error(lib)}")
        return result.decode("utf-8")
    finally:
        lib.unpdf_free_document(handle)


def to_text(path: str) -> str:
    """
    Convert a PDF file to plain text.

    Args:
        path: Path to the PDF file.

    Returns:
        The extracted content as plain text.

    Raises:
        RuntimeError: If conversion fails.
    """
    lib = get_library()
    handle = _parse_file(lib, path)
    try:
        result = lib.unpdf_to_text(handle)
        if not result:
            raise RuntimeError(f"unpdf error: {_check_last_error(lib)}")
        return result.decode("utf-8")
    finally:
        lib.unpdf_free_document(handle)


def to_json(path: str, pretty: bool = False) -> str:
    """
    Convert a PDF file to JSON format.

    Args:
        path: Path to the PDF file.
        pretty: If True, format JSON with indentation.

    Returns:
        The extracted content as JSON string.

    Raises:
        RuntimeError: If conversion fails.
    """
    lib = get_library()
    handle = _parse_file(lib, path)
    try:
        fmt = UNPDF_JSON_PRETTY if pretty else UNPDF_JSON_COMPACT
        result = lib.unpdf_to_json(handle, fmt)
        if not result:
            raise RuntimeError(f"unpdf error: {_check_last_error(lib)}")
        return result.decode("utf-8")
    finally:
        lib.unpdf_free_document(handle)


def get_info(path: str) -> dict[str, Any]:
    """
    Get document metadata from a PDF file.

    Note:
        ``resource_count`` counts the extracted-resource inventory, which is
        populated only when parsing runs with resource extraction enabled — the
        FFI parse path keeps it off by default (since 0.4.0), so it is 0 here.
        It is not a count of images referenced by page content streams; to detect
        image-only (scanned) pages use :func:`get_page_stats` or
        :func:`get_extraction_quality` instead.

    Args:
        path: Path to the PDF file.

    Returns:
        Dictionary containing document metadata (title, author, section_count, etc.)

    Raises:
        RuntimeError: If extraction fails.
    """
    lib = get_library()
    handle = _parse_file(lib, path)
    try:
        info: dict[str, Any] = {}

        title = lib.unpdf_get_title(handle)
        if title:
            info["title"] = title.decode("utf-8")

        author = lib.unpdf_get_author(handle)
        if author:
            info["author"] = author.decode("utf-8")

        info["section_count"] = lib.unpdf_section_count(handle)
        info["resource_count"] = lib.unpdf_resource_count(handle)

        return info
    finally:
        lib.unpdf_free_document(handle)


def get_extraction_quality(path: str) -> dict[str, Any]:
    """
    Get extraction quality diagnostics for a PDF file.

    Use this to tell why extraction produced little or no text:
    ``is_scan_pdf`` identifies an image-only (scanned) document that needs OCR.
    For page-level discrimination in mixed documents use :func:`get_page_stats`.

    Args:
        path: Path to the PDF file.

    Returns:
        Dictionary with ``char_count``, ``word_count``, ``replacement_char_count``,
        ``encrypted``, ``is_scan_pdf``, ``suppressed_ocr_pages``.

    Raises:
        RuntimeError: If parsing or retrieval fails.
    """
    lib = get_library()
    handle = _parse_file(lib, path)
    try:
        result = lib.unpdf_get_extraction_quality(handle)
        if not result:
            raise RuntimeError(f"unpdf error: {_check_last_error(lib)}")
        return json.loads(result.decode("utf-8"))
    finally:
        lib.unpdf_free_document(handle)


def get_page_stats(path: str, page_number: int) -> dict[str, Any]:
    """
    Get content-stream operator statistics for a single page.

    ``text_op_count == 0`` with ``image_op_count > 0`` identifies an image-only
    (scanned) page — OCR required. Both 0 means a genuinely blank page.

    Args:
        path: Path to the PDF file.
        page_number: Page number (1-indexed).

    Returns:
        Dictionary with ``page``, ``text_op_count``, ``image_op_count``,
        ``ocr_text_suppressed``.

    Raises:
        RuntimeError: If parsing fails or the page is out of range.
    """
    lib = get_library()
    handle = _parse_file(lib, path)
    try:
        result = lib.unpdf_page_stats(handle, page_number)
        if not result:
            raise RuntimeError(f"unpdf error: {_check_last_error(lib)}")
        return json.loads(result.decode("utf-8"))
    finally:
        lib.unpdf_free_document(handle)


def get_page_count(path: str) -> int:
    """
    Get the number of pages (sections) in a PDF file.

    Args:
        path: Path to the PDF file.

    Returns:
        The number of pages, or -1 on error.
    """
    lib = get_library()
    handle = lib.unpdf_parse_file(_encode_path(path))
    if not handle:
        return -1
    try:
        return lib.unpdf_section_count(handle)
    finally:
        lib.unpdf_free_document(handle)


def is_pdf(path: str) -> bool:
    """
    Check if a file is a valid PDF by attempting to parse it.

    Args:
        path: Path to the file.

    Returns:
        True if the file can be parsed as a PDF, False otherwise.
    """
    lib = get_library()
    handle = lib.unpdf_parse_file(_encode_path(path))
    if not handle:
        return False
    lib.unpdf_free_document(handle)
    return True


def version() -> str:
    """
    Get the version of the native unpdf library.

    Returns:
        Version string.
    """
    lib = get_library()
    ver = lib.unpdf_version()
    if ver:
        return ver.decode("utf-8")
    return "unknown"
