"""
High-level Python API for unpdf.
"""

import ctypes
import json
import os
from enum import IntEnum
from typing import Any, Union

from ._native import get_library, UNPDF_JSON_PRETTY, UNPDF_JSON_COMPACT

#: What every function in this module accepts as its PDF: a filesystem path
#: (``str``, or anything implementing ``os.PathLike`` such as ``pathlib.Path``)
#: or the PDF's own bytes. The two are told apart by type, so there is no
#: ambiguity — ``str`` is always a path, ``bytes`` is always content.
PdfSource = Union[str, "os.PathLike[str]", bytes, bytearray]


class ErrorKind(IntEnum):
    """
    Why an unpdf call failed, so callers can branch on the reason instead of
    matching on message text.

    Values 1–17 mirror the library's own failure reasons one-to-one; values 100+
    are raised at the FFI boundary and have no library-side counterpart. The
    numbers are part of the native ABI: a new reason takes the next free number
    and existing ones are never renumbered, so an unrecognised value should be
    treated as a generic failure rather than as an error.
    """

    NONE = 0
    OTHER = 1
    IO = 2
    UNKNOWN_FORMAT = 3
    UNSUPPORTED_VERSION = 4
    PDF_PARSE = 5
    ENCRYPTED = 6
    INVALID_PASSWORD = 7
    CORRUPTED = 8
    MISSING_OBJECT = 9
    FONT_DECODE = 10
    IMAGE_EXTRACT = 11
    RENDER = 12
    TEXT_EXTRACT = 13
    PAGE_OUT_OF_RANGE = 14
    INVALID_PAGE_RANGE = 15
    RESOURCE_NOT_FOUND = 16
    ENCODING = 17

    INVALID_ARGUMENT = 100
    PANIC = 101
    INVALID_OUTPUT = 102


class UnpdfError(RuntimeError):
    """
    An unpdf call failed.

    Subclasses :class:`RuntimeError`, which is what this package raised before
    error classification existed, so ``except RuntimeError`` keeps working.

    Attributes:
        kind: An :class:`ErrorKind`, or the raw integer if the native library
            reports a reason this build does not know about.
    """

    def __init__(self, message: str, kind: int = ErrorKind.OTHER) -> None:
        super().__init__(message)
        try:
            self.kind: int = ErrorKind(kind)
        except ValueError:
            self.kind = kind


def _encode_path(path: "str | os.PathLike[str]") -> bytes:
    """Encode a filesystem path for the FFI boundary.

    Goes through :func:`os.fspath`, so ``pathlib.Path`` and any other path-like
    object works, not only ``str``.
    """
    return os.fspath(path).encode("utf-8")


def _check_last_error(lib: ctypes.CDLL) -> str:
    """Get the last error message from the native library."""
    err = lib.unpdf_last_error()
    if err:
        return err.decode("utf-8")
    return "Unknown error"


def _native_error(lib: ctypes.CDLL) -> "UnpdfError":
    """
    Build an :class:`UnpdfError` from the native error state.

    Reads the message and its classification together, before any further native
    call can overwrite the thread-local error slot.
    """
    message = _check_last_error(lib)
    kind = lib.unpdf_last_error_kind()
    return UnpdfError(f"unpdf error: {message}", kind)


def _parse_file(lib: ctypes.CDLL, source: PdfSource) -> ctypes.c_void_p:
    """Parse a PDF and return the document handle. Raises on failure.

    Dispatches on the type of ``source``: bytes are parsed in memory, anything
    else is treated as a filesystem path.
    """
    if isinstance(source, (bytes, bytearray)):
        if not source:
            raise UnpdfError("unpdf error: empty PDF data", ErrorKind.INVALID_ARGUMENT)
        buf = (ctypes.c_uint8 * len(source)).from_buffer_copy(source)
        handle = lib.unpdf_parse_bytes(buf, len(source))
    else:
        handle = lib.unpdf_parse_file(_encode_path(source))

    if not handle:
        raise _native_error(lib)
    return handle


def to_markdown(source: PdfSource, flags: int = 0) -> str:
    """
    Convert a PDF file to Markdown format.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        flags: Bitwise OR of UNPDF_FLAG_* constants (optional).

    Returns:
        The extracted content as Markdown.

    Raises:
        UnpdfError: If conversion fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source)
    try:
        result = lib.unpdf_to_markdown(handle, flags)
        if not result:
            raise _native_error(lib)
        return result.decode("utf-8")
    finally:
        lib.unpdf_free_document(handle)


def to_text(source: PdfSource) -> str:
    """
    Convert a PDF file to plain text.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.

    Returns:
        The extracted content as plain text.

    Raises:
        UnpdfError: If conversion fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source)
    try:
        result = lib.unpdf_to_text(handle)
        if not result:
            raise _native_error(lib)
        return result.decode("utf-8")
    finally:
        lib.unpdf_free_document(handle)


def to_json(source: PdfSource, pretty: bool = False) -> str:
    """
    Convert a PDF file to JSON format.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        pretty: If True, format JSON with indentation.

    Returns:
        The extracted content as JSON string.

    Raises:
        UnpdfError: If conversion fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source)
    try:
        fmt = UNPDF_JSON_PRETTY if pretty else UNPDF_JSON_COMPACT
        result = lib.unpdf_to_json(handle, fmt)
        if not result:
            raise _native_error(lib)
        return result.decode("utf-8")
    finally:
        lib.unpdf_free_document(handle)


def get_info(source: PdfSource) -> dict[str, Any]:
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
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.

    Returns:
        Dictionary containing document metadata (title, author, section_count, etc.)

    Raises:
        UnpdfError: If extraction fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source)
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


def get_extraction_quality(source: PdfSource) -> dict[str, Any]:
    """
    Get extraction quality diagnostics for a PDF file.

    Use this to tell why extraction produced little or no text:
    ``is_scan_pdf`` identifies an image-only (scanned) document that needs OCR.
    For page-level discrimination in mixed documents use :func:`get_page_stats`.

    ``pages_incomplete`` is the one to check before indexing or archiving: it is
    ``True`` when the document was damaged and some pages never reached the output,
    even though extraction "succeeded". A page that silently never arrived is
    otherwise indistinguishable from a page that never existed.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.

    Returns:
        Dictionary with ``char_count``, ``word_count``, ``replacement_char_count``,
        ``encrypted``, ``is_scan_pdf``, ``suppressed_ocr_pages``,
        ``pages_incomplete``, ``declared_page_count``, ``unresolved_page_nodes``,
        ``skipped_object_count``.

        ``unresolved_page_nodes`` counts unreadable page-tree *nodes*, not lost
        pages — one unreadable node can cost a whole subtree. Treat any non-zero
        value as "incomplete" and do not report it as a page count.

    Raises:
        UnpdfError: If parsing or retrieval fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source)
    try:
        result = lib.unpdf_get_extraction_quality(handle)
        if not result:
            raise _native_error(lib)
        return json.loads(result.decode("utf-8"))
    finally:
        lib.unpdf_free_document(handle)


def get_page_stats(source: PdfSource, page_number: int) -> dict[str, Any]:
    """
    Get content-stream operator statistics for a single page.

    ``text_op_count == 0`` with ``image_op_count > 0`` identifies an image-only
    (scanned) page — OCR required. Both 0 means a genuinely blank page.

    Note:
        A *searchable* scan (page image plus an invisible OCR text layer) reports
        ``text_op_count > 0`` — combine the check with ``ocr_text_suppressed``,
        which flags pages whose unreadable OCR layer was dropped.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        page_number: Page number (1-indexed).

    Returns:
        Dictionary with ``page``, ``text_op_count``, ``image_op_count``,
        ``ocr_text_suppressed``.

    Raises:
        UnpdfError: If parsing fails or the page is out of range
            (``kind == ErrorKind.PAGE_OUT_OF_RANGE``).
    """
    lib = get_library()
    handle = _parse_file(lib, source)
    try:
        result = lib.unpdf_page_stats(handle, page_number)
        if not result:
            raise _native_error(lib)
        return json.loads(result.decode("utf-8"))
    finally:
        lib.unpdf_free_document(handle)


def get_page_count(source: PdfSource) -> int:
    """
    Get the number of pages (sections) in a PDF file.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.

    Returns:
        The number of pages, or -1 if it could not be parsed.

    Raises:
        TypeError: If ``source`` is neither a path-like object nor bytes. A
            wrong-typed argument is a caller bug, not an unparsable PDF, so it is
            not folded into the ``-1`` return.
    """
    lib = get_library()
    try:
        handle = _parse_file(lib, source)
    except UnpdfError:
        return -1
    try:
        return lib.unpdf_section_count(handle)
    finally:
        lib.unpdf_free_document(handle)


def is_pdf(source: PdfSource) -> bool:
    """
    Check whether a PDF can be parsed, by attempting to parse it.

    For a path this answers "is the file at this path a parsable PDF"; for bytes it
    answers the same question about the bytes themselves, without touching the
    filesystem.

    Args:
        source: Path to the file (``str`` or ``os.PathLike``), or PDF bytes.

    Returns:
        True if it can be parsed as a PDF, False otherwise — including when the
        path does not exist or the data is not a PDF.

    Raises:
        TypeError: If ``source`` is neither a path-like object nor bytes. A
            wrong-typed argument is a caller bug, not an unparsable PDF, so it is
            not folded into the ``False`` return.
    """
    lib = get_library()
    try:
        handle = _parse_file(lib, source)
    except UnpdfError:
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
