"""
unpdf - Python bindings for unpdf PDF extraction library.
"""

from .unpdf import (
    UNPDF_FLAG_ESCAPE_SPECIAL,
    UNPDF_FLAG_FRONTMATTER,
    UNPDF_FLAG_PAGE_MARKERS,
    ErrorKind,
    PdfSource,
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
    "UNPDF_FLAG_ESCAPE_SPECIAL",
    "UNPDF_FLAG_FRONTMATTER",
    "UNPDF_FLAG_PAGE_MARKERS",
    "ErrorKind",
    "PdfSource",
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
