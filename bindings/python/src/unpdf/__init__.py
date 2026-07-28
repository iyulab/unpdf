"""
unpdf - Python bindings for unpdf PDF extraction library.
"""

from .unpdf import (
    ErrorKind,
    UnpdfError,
    to_markdown,
    to_text,
    to_json,
    get_info,
    get_extraction_quality,
    get_page_stats,
    get_page_count,
    is_pdf,
    version,
)

__all__ = [
    "ErrorKind",
    "UnpdfError",
    "to_markdown",
    "to_text",
    "to_json",
    "get_info",
    "get_extraction_quality",
    "get_page_stats",
    "get_page_count",
    "is_pdf",
    "version",
]
