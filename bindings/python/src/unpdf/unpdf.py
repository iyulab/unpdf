"""
High-level Python API for unpdf.
"""

import ctypes
import json
import os
from dataclasses import dataclass
from enum import IntEnum
from typing import Any, Union

from ._native import (
    get_library,
    UNPDF_FLAG_ESCAPE_SPECIAL,
    UNPDF_FLAG_FRONTMATTER,
    UNPDF_FLAG_NO_ESCAPE,
    UNPDF_FLAG_PAGE_MARKERS,
    UNPDF_FLAG_REFINE,
    UNPDF_JSON_COMPACT,
    UNPDF_JSON_PRETTY,
)

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


def _take_string(lib: ctypes.CDLL, ptr: int) -> str:
    """Copy a non-null native UTF-8 string, then give the allocation back.

    An empty string is a result, not an error: only a null pointer is.
    """
    try:
        return ctypes.string_at(ptr).decode("utf-8")
    finally:
        lib.unpdf_free_string(ptr)


def _native_error(lib: ctypes.CDLL) -> "UnpdfError":
    """
    Build an :class:`UnpdfError` from the native error state.

    Reads the message and its classification together, before any further native
    call can overwrite the thread-local error slot.
    """
    message = _check_last_error(lib)
    kind = lib.unpdf_last_error_kind()
    return UnpdfError(f"unpdf error: {message}", kind)


#: Parsing options accepted by every function below, passed straight through to the
#: native library as JSON. Every key is optional; an absent key keeps unpdf's own
#: default for that setting. Known keys: ``error_mode`` (``"strict"`` |
#: ``"lenient"``), ``extract_text`` (bool — on by default; ``False`` is structure only:
#: every page is still produced with none of its content blocks),
#: ``extract_resources`` (bool — off by default; enable to
#: populate the resource inventory :func:`get_resource_ids`/:func:`get_resource_info`/
#: :func:`get_resource_data` read from), ``min_image_dimension`` (int, default 64 —
#: images below this on either axis are dropped as decorative; 0 keeps every image),
#: ``parallel`` (bool), ``password`` (str), ``suppress_low_confidence_ocr`` (bool).
ParseOptions = "dict[str, Any]"


def _parse_file(
    lib: ctypes.CDLL, source: PdfSource, options: "dict[str, Any] | None" = None
) -> ctypes.c_void_p:
    """Parse a PDF and return the document handle. Raises on failure.

    Dispatches on the type of ``source``: bytes are parsed in memory, anything
    else is treated as a filesystem path. ``options`` is JSON-encoded and passed
    to the native library; ``None`` (the default) keeps unpdf's own defaults.
    """
    options_json = json.dumps(options).encode("utf-8") if options is not None else None

    if isinstance(source, (bytes, bytearray)):
        if not source:
            raise UnpdfError("unpdf error: empty PDF data", ErrorKind.INVALID_ARGUMENT)
        buf = (ctypes.c_uint8 * len(source)).from_buffer_copy(source)
        handle = lib.unpdf_parse_bytes_with_options(buf, len(source), options_json)
    else:
        handle = lib.unpdf_parse_file_with_options(_encode_path(source), options_json)

    if not handle:
        raise _native_error(lib)
    return handle


def to_markdown(
    source: PdfSource, flags: int = 0, options: "dict[str, Any] | None" = None
) -> str:
    """
    Convert a PDF file to Markdown format.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        flags: Bitwise OR of ``UNPDF_FLAG_FRONTMATTER``,
            ``UNPDF_FLAG_PAGE_MARKERS``, ``UNPDF_FLAG_REFINE`` and
            ``UNPDF_FLAG_NO_ESCAPE`` (optional). All are importable from
            ``unpdf``. Special Markdown characters are escaped unless
            ``UNPDF_FLAG_NO_ESCAPE`` is set; ``UNPDF_FLAG_ESCAPE_SPECIAL`` is
            accepted and has no effect.
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        The extracted content as Markdown.

    Raises:
        UnpdfError: If conversion fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        result = lib.unpdf_to_markdown(handle, flags)
        if not result:
            raise _native_error(lib)
        return _take_string(lib, result)
    finally:
        lib.unpdf_free_document(handle)


def to_text(source: PdfSource, options: "dict[str, Any] | None" = None) -> str:
    """
    Convert a PDF file to plain text.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        The extracted content as plain text.

    Raises:
        UnpdfError: If conversion fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        result = lib.unpdf_to_text(handle)
        if not result:
            raise _native_error(lib)
        return _take_string(lib, result)
    finally:
        lib.unpdf_free_document(handle)


def to_json(
    source: PdfSource, pretty: bool = False, options: "dict[str, Any] | None" = None
) -> str:
    """
    Convert a PDF file to JSON format.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        pretty: If True, format JSON with indentation.
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        The extracted content as JSON string.

    Raises:
        UnpdfError: If conversion fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        fmt = UNPDF_JSON_PRETTY if pretty else UNPDF_JSON_COMPACT
        result = lib.unpdf_to_json(handle, fmt)
        if not result:
            raise _native_error(lib)
        return _take_string(lib, result)
    finally:
        lib.unpdf_free_document(handle)


def get_info(
    source: PdfSource, options: "dict[str, Any] | None" = None
) -> dict[str, Any]:
    """
    Get document metadata from a PDF file.

    Note:
        ``resource_count`` counts the extracted-resource inventory, which is
        populated only when parsing runs with resource extraction enabled — pass
        ``options={"extract_resources": True}`` (see :data:`ParseOptions`), then
        read the resources themselves with :func:`get_resource_ids` /
        :func:`get_resource_info` / :func:`get_resource_data`. It is not a count
        of images referenced by page content streams; to detect image-only
        (scanned) pages use :func:`get_page_stats` or
        :func:`get_extraction_quality` instead.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        Dictionary containing document metadata (title, author, section_count, etc.)

    Raises:
        UnpdfError: If extraction fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        info: dict[str, Any] = {}

        title = lib.unpdf_get_title(handle)
        if title:
            info["title"] = _take_string(lib, title)

        author = lib.unpdf_get_author(handle)
        if author:
            info["author"] = _take_string(lib, author)

        info["section_count"] = lib.unpdf_section_count(handle)
        info["resource_count"] = lib.unpdf_resource_count(handle)

        return info
    finally:
        lib.unpdf_free_document(handle)


def get_extraction_quality(
    source: PdfSource, options: "dict[str, Any] | None" = None
) -> dict[str, Any]:
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
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        Dictionary with ``char_count``, ``word_count``, ``replacement_char_count``,
        ``encrypted``, ``is_scan_pdf``, ``suppressed_ocr_pages``,
        ``suppressed_text_runs``, ``undecodable_content_streams``,
        ``pages_incomplete``, ``declared_page_count``,
        ``unresolved_page_nodes``, ``skipped_object_count``,
        ``unsupported_image_count``, ``ai_fallback_count``.

        ``suppressed_text_runs`` counts text runs the font decoder could not read
        and discarded — content the document had and this output does not. Any
        non-zero value means the extraction is incomplete; the count is in runs,
        not characters, because the discarded text was never decoded.

        ``undecodable_content_streams`` counts page content streams that could
        not be decoded. Lenient parsing (the default) leaves them out and keeps
        the rest of the page — an empty page when it was the page's only stream
        — so any non-zero value means content is missing. Strict parsing fails
        instead.

        ``unresolved_page_nodes`` counts unreadable page-tree *nodes*, not lost
        pages — one unreadable node can cost a whole subtree. Treat any non-zero
        value as "incomplete" and do not report it as a page count.

        ``unsupported_image_count`` counts embedded images recognized as image
        XObjects but dropped because their color space or bit depth isn't
        extractable — distinguishes "no image" from "image present but
        couldn't be extracted".

        ``ai_fallback_count`` counts images whose VLM understanding fell back to
        the non-AI result. It is only ever non-zero when AI parse options are
        configured, and extraction succeeds either way, so read it as "this many
        images are described without AI" rather than as a failure. The
        render-side refine pass reports its own fallbacks separately.

    Raises:
        UnpdfError: If parsing or retrieval fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        result = lib.unpdf_get_extraction_quality(handle)
        if not result:
            raise _native_error(lib)
        return json.loads(_take_string(lib, result))
    finally:
        lib.unpdf_free_document(handle)


def get_page_stats(
    source: PdfSource, page_number: int, options: "dict[str, Any] | None" = None
) -> dict[str, Any]:
    """
    Get statistics for a single page: what it holds and how well it was read.

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
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        Dictionary with ``page``, ``text_op_count``, ``image_op_count``,
        ``form_op_count``, ``ocr_text_suppressed``, ``suppressed_text_runs``,
        ``unreadable_fonts``, ``undecodable_content_streams``. Text and images inside the Form XObjects
        the page paints are counted in ``text_op_count`` / ``image_op_count``;
        ``form_op_count`` counts the form paints themselves. The last two are this page's share of the
        document-level totals of the same name reported by
        :func:`get_extraction_quality` — text runs the font decoder could not read
        and discarded, and content streams that could not be decoded.

        ``unreadable_fonts`` is the list of fonts behind ``suppressed_text_runs``:
        ``{"name", "reason", "runs"}`` per font and reason, in order of first
        loss (``[]`` when nothing was discarded). ``name`` is the font's
        ``/BaseFont`` (its resource name when it has none); ``reason`` is
        ``"composite_unresolved"`` (a Type0/CID font whose codes map to no text)
        or ``"binary_density"`` (a simple font whose decode was binary noise) —
        treat any other value as a generic "unreadable". The ``runs`` sum to
        ``suppressed_text_runs``.

        How the page was read: ``rotation`` (the page's ``/Rotate``, degrees
        clockwise), ``image_coverage`` (share of the page painted by images, 0 to
        1), ``rotated_text_runs`` (text not set horizontally left to right),
        ``ruled_grids`` / ``ruled_tables`` (ruling-line grids drawn, tables built
        from them), ``reading_regions`` (text regions read one after another),
        ``column_count`` (the most of them side by side at any height) and
        ``ambiguous_layout_regions`` (regions read across although their text
        looked like two columns). Thresholds are the caller's.

    Raises:
        UnpdfError: If parsing fails or the page is out of range
            (``kind == ErrorKind.PAGE_OUT_OF_RANGE``).
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        result = lib.unpdf_page_stats(handle, page_number)
        if not result:
            raise _native_error(lib)
        return json.loads(_take_string(lib, result))
    finally:
        lib.unpdf_free_document(handle)


def get_resource_ids(
    source: PdfSource, options: "dict[str, Any] | None" = None
) -> "list[str]":
    """
    List the ids of a document's extracted embedded resources (images).

    The inventory is only populated when parsing with
    ``options={"extract_resources": True}`` (see :data:`ParseOptions`) — without
    it this returns an empty list, not an error, matching :func:`get_info`'s
    ``resource_count``.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        options: Parsing options — see :data:`ParseOptions`.

    Returns:
        Resource ids in reading order, e.g. ``["page1_Im0.jpg"]`` — the same ids
        the rendered Markdown references. Pass one to
        :func:`get_resource_info` or :func:`get_resource_data`.

    Raises:
        UnpdfError: If parsing fails. Its ``kind`` says why.
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        result = lib.unpdf_get_resource_ids(handle)
        if not result:
            raise _native_error(lib)
        return json.loads(_take_string(lib, result))
    finally:
        lib.unpdf_free_document(handle)


def get_resource_info(
    source: PdfSource, resource_id: str, options: "dict[str, Any] | None" = None
) -> dict[str, Any]:
    """
    Get metadata for one extracted resource, without its binary data.

    Requires ``options={"extract_resources": True}`` — see
    :func:`get_resource_ids`.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        resource_id: A resource id from :func:`get_resource_ids`.
        options: Parsing options — see :data:`ParseOptions`.

    Returns:
        Dictionary with ``id``, ``type``, ``filename``, ``mime_type``, ``size``,
        ``width``, ``height``, ``page`` — the page the resource was collected from
        (1-based, the numbering of the page markers). Read the page from this
        field, not from the id: the id's format is not part of the contract.

    Raises:
        UnpdfError: If parsing fails or ``resource_id`` is not found
            (``kind == ErrorKind.RESOURCE_NOT_FOUND``).
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        result = lib.unpdf_get_resource_info(handle, resource_id.encode("utf-8"))
        if not result:
            raise _native_error(lib)
        return json.loads(_take_string(lib, result))
    finally:
        lib.unpdf_free_document(handle)


def get_resource_data(
    source: PdfSource, resource_id: str, options: "dict[str, Any] | None" = None
) -> bytes:
    """
    Get the binary data of one extracted resource.

    Requires ``options={"extract_resources": True}`` — see
    :func:`get_resource_ids`.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        resource_id: A resource id from :func:`get_resource_ids`.
        options: Parsing options — see :data:`ParseOptions`.

    Returns:
        The resource's raw bytes (e.g. JPEG-encoded image data).

    Raises:
        UnpdfError: If parsing fails or ``resource_id`` is not found
            (``kind == ErrorKind.RESOURCE_NOT_FOUND``).
    """
    lib = get_library()
    handle = _parse_file(lib, source, options)
    try:
        out_len = ctypes.c_size_t(0)
        result = lib.unpdf_get_resource_data(
            handle, resource_id.encode("utf-8"), ctypes.byref(out_len)
        )
        if not result:
            raise _native_error(lib)
        try:
            return ctypes.string_at(result, out_len.value)
        finally:
            lib.unpdf_free_bytes(result, out_len.value)
    finally:
        lib.unpdf_free_document(handle)


@dataclass(frozen=True)
class RenderedPage:
    """A rendered page: a PNG, its size in pixels, and what it could not show.

    ``gaps`` counts, by reason, what the renderer left out — ``text_runs``
    (text in fonts that are not embedded and have no stand-in, or are Type 3),
    ``images`` (codecs it does not decode), ``inline_images``, ``shadings`` and
    ``undecodable_content_streams``. All zero means everything was painted;
    otherwise the rest of the page still was.

    ``substituted_text_runs`` counts text painted in a standard face standing in
    for a font the PDF does not embed — readable, but not the page's own typeface.
    It is not a gap.
    """

    png: bytes
    width: int
    height: int
    gaps: "dict[str, int]"
    substituted_text_runs: int = 0


def _render(lib: ctypes.CDLL, handle: Any, page_number: int, dpi: float, region: str) -> RenderedPage:
    options = json.dumps({"dpi": dpi, "region": region}).encode("utf-8")
    out_len = ctypes.c_size_t(0)
    info = ctypes.c_void_p(None)
    result = lib.unpdf_render_page(
        handle, page_number, options, ctypes.byref(out_len), ctypes.byref(info)
    )
    if not result:
        raise _native_error(lib)
    try:
        png = ctypes.string_at(result, out_len.value)
    finally:
        lib.unpdf_free_bytes(result, out_len.value)
    report = json.loads(_take_string(lib, info.value)) if info.value else {}
    return RenderedPage(
        png=png,
        width=int(report.get("width", 0)),
        height=int(report.get("height", 0)),
        gaps=dict(report.get("gaps", {})),
        substituted_text_runs=int(report.get("substituted_text_runs", 0)),
    )


class Document:
    """A parsed PDF kept open, so its pages can be rendered — and their statistics
    read — without parsing the file again for every call.

    Use it as a context manager, or call :meth:`close`::

        with unpdf.Document("report.pdf") as doc:
            for n in pages_to_reread:
                if doc.get_page_stats(n)["image_coverage"] > 0.9:
                    png = doc.render_page(n).png
    """

    def __init__(self, source: PdfSource, options: "dict[str, Any] | None" = None) -> None:
        self._lib = get_library()
        self._handle: Any = _parse_file(self._lib, source, options)

    def render_page(self, page_number: int, dpi: float = 150.0, region: str = "crop") -> RenderedPage:
        """Render a page (1-indexed) to a PNG — painted from the content this document
        was parsed from, so it is the same page text extraction reads.

        Args:
            page_number: Page number (1-indexed).
            dpi: Resolution; a page point is ``dpi / 72`` pixels.
            region: ``"crop"`` (what a viewer shows) or ``"media"`` (the whole sheet).

        Raises:
            UnpdfError: ``kind == ErrorKind.PAGE_OUT_OF_RANGE`` for a page the
                document does not have; ``INVALID_ARGUMENT`` for options it cannot use.
        """
        return _render(self._lib, self._live(), page_number, dpi, region)

    def get_page_stats(self, page_number: int) -> "dict[str, Any]":
        """Statistics for a page — see :func:`get_page_stats`."""
        result = self._lib.unpdf_page_stats(self._live(), page_number)
        if not result:
            raise _native_error(self._lib)
        return json.loads(_take_string(self._lib, result))

    def close(self) -> None:
        """Release the document. Further calls raise ``ValueError``."""
        if self._handle:
            self._lib.unpdf_free_document(self._handle)
            self._handle = None

    def _live(self) -> Any:
        if not self._handle:
            raise ValueError("the document is closed")
        return self._handle

    def __enter__(self) -> "Document":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __del__(self) -> None:
        self.close()


def render_page(
    source: PdfSource,
    page_number: int,
    dpi: float = 150.0,
    region: str = "crop",
    options: "dict[str, Any] | None" = None,
) -> RenderedPage:
    """Render one page to a PNG. Parses the document for this call — to render
    several pages of one document, open it once with :class:`Document`.
    """
    with Document(source, options) as doc:
        return doc.render_page(page_number, dpi, region)


def get_page_count(
    source: PdfSource, options: "dict[str, Any] | None" = None
) -> int:
    """
    Get the number of pages (sections) in a PDF file.

    Args:
        source: Path to the PDF file (``str`` or ``os.PathLike``), or the
            PDF's own bytes.
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

    Returns:
        The number of pages, or -1 if it could not be parsed.

    Raises:
        TypeError: If ``source`` is neither a path-like object nor bytes. A
            wrong-typed argument is a caller bug, not an unparsable PDF, so it is
            not folded into the ``-1`` return.
    """
    lib = get_library()
    try:
        handle = _parse_file(lib, source, options)
    except UnpdfError:
        return -1
    try:
        return lib.unpdf_section_count(handle)
    finally:
        lib.unpdf_free_document(handle)


def is_pdf(source: PdfSource, options: "dict[str, Any] | None" = None) -> bool:
    """
    Check whether a PDF can be parsed, by attempting to parse it.

    For a path this answers "is the file at this path a parsable PDF"; for bytes it
    answers the same question about the bytes themselves, without touching the
    filesystem.

    Args:
        source: Path to the file (``str`` or ``os.PathLike``), or PDF bytes.
        options: Parsing options — see :data:`ParseOptions`. ``None`` (the
            default) uses unpdf's own defaults.

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
        handle = _parse_file(lib, source, options)
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
