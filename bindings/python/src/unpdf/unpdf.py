"""
High-level Python API for unpdf.
"""

import json
from typing import Any

from ._native import get_library, UnpdfResult


def _encode_path(path: str) -> bytes:
    """Encode a path string to bytes for FFI."""
    return path.encode("utf-8")


def _handle_result(result: UnpdfResult) -> str:
    """Handle an UnpdfResult, raising on error."""
    lib = get_library()

    try:
        if result.success:
            if result.data:
                return result.data.decode("utf-8")
            return ""
        else:
            error_msg = "Unknown error"
            if result.error:
                error_msg = result.error.decode("utf-8")
            raise RuntimeError(f"unpdf error: {error_msg}")
    finally:
        lib.unpdf_free_result(result)


def to_markdown(path: str) -> str:
    """
    Convert a PDF file to Markdown format.

    Args:
        path: Path to the PDF file.

    Returns:
        The extracted content as Markdown.

    Raises:
        RuntimeError: If conversion fails.
    """
    lib = get_library()
    result = lib.unpdf_to_markdown(_encode_path(path))
    return _handle_result(result)


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
    result = lib.unpdf_to_text(_encode_path(path))
    return _handle_result(result)


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
    result = lib.unpdf_to_json(_encode_path(path), pretty)
    return _handle_result(result)


def get_info(path: str) -> dict[str, Any]:
    """
    Get document metadata from a PDF file.

    Args:
        path: Path to the PDF file.

    Returns:
        Dictionary containing document metadata (title, author, page_count, etc.)

    Raises:
        RuntimeError: If extraction fails.
    """
    lib = get_library()
    result = lib.unpdf_get_info(_encode_path(path))
    json_str = _handle_result(result)
    return json.loads(json_str)


def get_page_count(path: str) -> int:
    """
    Get the number of pages in a PDF file.

    Args:
        path: Path to the PDF file.

    Returns:
        The number of pages, or -1 on error.
    """
    lib = get_library()
    return lib.unpdf_get_page_count(_encode_path(path))


def is_pdf(path: str) -> bool:
    """
    Check if a file is a valid PDF.

    Args:
        path: Path to the file.

    Returns:
        True if the file is a valid PDF, False otherwise.
    """
    lib = get_library()
    return lib.unpdf_is_pdf(_encode_path(path))


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
