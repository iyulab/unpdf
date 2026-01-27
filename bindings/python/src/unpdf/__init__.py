"""
unpdf - Python bindings for unpdf PDF extraction library.
"""

from .unpdf import (
    to_markdown,
    to_text,
    to_json,
    get_info,
    get_page_count,
    is_pdf,
    version,
)

__all__ = [
    "to_markdown",
    "to_text",
    "to_json",
    "get_info",
    "get_page_count",
    "is_pdf",
    "version",
]
