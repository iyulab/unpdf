"""
unpdf - PDF content extraction library
Python ctypes bindings
"""

import ctypes
import platform
from pathlib import Path
from typing import Optional


class UnpdfResult(ctypes.Structure):
    """Result structure returned by unpdf functions."""
    _fields_ = [
        ("success", ctypes.c_bool),
        ("data", ctypes.c_char_p),
        ("error", ctypes.c_char_p),
    ]


class UnpdfError(Exception):
    """Exception raised when an unpdf operation fails."""
    pass


class Unpdf:
    """PDF content extraction library."""

    def __init__(self, library_path: Optional[str] = None):
        """
        Initialize the unpdf library.

        Args:
            library_path: Optional path to the unpdf shared library.
                         If not provided, will search in standard locations.
        """
        if library_path:
            self._lib = ctypes.CDLL(library_path)
        else:
            self._lib = self._load_library()

        self._setup_functions()

    def _load_library(self) -> ctypes.CDLL:
        """Load the unpdf shared library."""
        system = platform.system()
        if system == "Windows":
            lib_name = "unpdf.dll"
        elif system == "Darwin":
            lib_name = "libunpdf.dylib"
        else:
            lib_name = "libunpdf.so"

        # Try common locations
        search_paths = [
            Path(__file__).parent / lib_name,
            Path.cwd() / lib_name,
            Path.cwd() / "target" / "release" / lib_name,
        ]

        for path in search_paths:
            if path.exists():
                return ctypes.CDLL(str(path))

        # Fall back to system library search
        return ctypes.CDLL(lib_name)

    def _setup_functions(self):
        """Set up function signatures."""
        # unpdf_to_markdown
        self._lib.unpdf_to_markdown.argtypes = [ctypes.c_char_p]
        self._lib.unpdf_to_markdown.restype = UnpdfResult

        # unpdf_to_text
        self._lib.unpdf_to_text.argtypes = [ctypes.c_char_p]
        self._lib.unpdf_to_text.restype = UnpdfResult

        # unpdf_to_json
        self._lib.unpdf_to_json.argtypes = [ctypes.c_char_p, ctypes.c_bool]
        self._lib.unpdf_to_json.restype = UnpdfResult

        # unpdf_get_info
        self._lib.unpdf_get_info.argtypes = [ctypes.c_char_p]
        self._lib.unpdf_get_info.restype = UnpdfResult

        # unpdf_get_page_count
        self._lib.unpdf_get_page_count.argtypes = [ctypes.c_char_p]
        self._lib.unpdf_get_page_count.restype = ctypes.c_int32

        # unpdf_is_pdf
        self._lib.unpdf_is_pdf.argtypes = [ctypes.c_char_p]
        self._lib.unpdf_is_pdf.restype = ctypes.c_bool

        # unpdf_free_result
        self._lib.unpdf_free_result.argtypes = [UnpdfResult]
        self._lib.unpdf_free_result.restype = None

        # unpdf_version
        self._lib.unpdf_version.argtypes = []
        self._lib.unpdf_version.restype = ctypes.c_char_p

    def _process_result(self, result: UnpdfResult) -> str:
        """Process a result and free memory."""
        try:
            if not result.success:
                error = result.error.decode("utf-8") if result.error else "Unknown error"
                raise UnpdfError(error)
            return result.data.decode("utf-8") if result.data else ""
        finally:
            self._lib.unpdf_free_result(result)

    @property
    def version(self) -> str:
        """Get the version of the unpdf library."""
        result = self._lib.unpdf_version()
        return result.decode("utf-8") if result else ""

    def to_markdown(self, path: str) -> str:
        """
        Convert a PDF file to Markdown.

        Args:
            path: Path to the PDF file.

        Returns:
            The Markdown content.

        Raises:
            UnpdfError: If the conversion fails.
        """
        result = self._lib.unpdf_to_markdown(path.encode("utf-8"))
        return self._process_result(result)

    def to_text(self, path: str) -> str:
        """
        Convert a PDF file to plain text.

        Args:
            path: Path to the PDF file.

        Returns:
            The text content.

        Raises:
            UnpdfError: If the conversion fails.
        """
        result = self._lib.unpdf_to_text(path.encode("utf-8"))
        return self._process_result(result)

    def to_json(self, path: str, pretty: bool = True) -> str:
        """
        Convert a PDF file to JSON.

        Args:
            path: Path to the PDF file.
            pretty: Whether to format the JSON with indentation.

        Returns:
            The JSON content.

        Raises:
            UnpdfError: If the conversion fails.
        """
        result = self._lib.unpdf_to_json(path.encode("utf-8"), pretty)
        return self._process_result(result)

    def get_info(self, path: str) -> str:
        """
        Get document information as JSON.

        Args:
            path: Path to the PDF file.

        Returns:
            Document metadata as JSON.

        Raises:
            UnpdfError: If the operation fails.
        """
        result = self._lib.unpdf_get_info(path.encode("utf-8"))
        return self._process_result(result)

    def get_page_count(self, path: str) -> int:
        """
        Get the page count of a PDF file.

        Args:
            path: Path to the PDF file.

        Returns:
            Number of pages, or -1 on error.
        """
        return self._lib.unpdf_get_page_count(path.encode("utf-8"))

    def is_pdf(self, path: str) -> bool:
        """
        Check if a file is a valid PDF.

        Args:
            path: Path to the file.

        Returns:
            True if the file is a valid PDF.
        """
        return self._lib.unpdf_is_pdf(path.encode("utf-8"))


# Convenience instance for direct usage
_default_instance: Optional[Unpdf] = None


def _get_instance() -> Unpdf:
    """Get or create the default Unpdf instance."""
    global _default_instance
    if _default_instance is None:
        _default_instance = Unpdf()
    return _default_instance


def to_markdown(path: str) -> str:
    """Convert a PDF file to Markdown."""
    return _get_instance().to_markdown(path)


def to_text(path: str) -> str:
    """Convert a PDF file to plain text."""
    return _get_instance().to_text(path)


def to_json(path: str, pretty: bool = True) -> str:
    """Convert a PDF file to JSON."""
    return _get_instance().to_json(path, pretty)


def get_info(path: str) -> str:
    """Get document information as JSON."""
    return _get_instance().get_info(path)


def get_page_count(path: str) -> int:
    """Get the page count of a PDF file."""
    return _get_instance().get_page_count(path)


def is_pdf(path: str) -> bool:
    """Check if a file is a valid PDF."""
    return _get_instance().is_pdf(path)


def version() -> str:
    """Get the version of the unpdf library."""
    return _get_instance().version


if __name__ == "__main__":
    import sys
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <pdf_file>")
        sys.exit(1)

    pdf_path = sys.argv[1]
    unpdf = Unpdf()

    print(f"unpdf version: {unpdf.version}")
    print(f"Is PDF: {unpdf.is_pdf(pdf_path)}")
    print(f"Page count: {unpdf.get_page_count(pdf_path)}")
    print("\n--- Markdown ---")
    print(unpdf.to_markdown(pdf_path)[:500] + "...")
