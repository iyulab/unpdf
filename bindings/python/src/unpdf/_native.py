"""
Native library loading for unpdf.
"""

import ctypes
import os
import platform
import subprocess
import sys
from ctypes import c_bool, c_char_p, c_int, Structure, POINTER


class UnpdfResult(Structure):
    """Result structure returned by FFI functions."""
    _fields_ = [
        ("success", c_bool),
        ("data", c_char_p),
        ("error", c_char_p),
    ]


def _is_musl() -> bool:
    """Detect if the current Linux system uses musl libc."""
    try:
        with open("/etc/os-release") as f:
            if "alpine" in f.read().lower():
                return True
    except OSError:
        pass
    try:
        result = subprocess.run(
            ["ldd", "--version"], capture_output=True, text=True, timeout=5
        )
        if "musl" in (result.stdout + result.stderr).lower():
            return True
    except (OSError, subprocess.TimeoutExpired):
        pass
    return False


def _get_lib_path() -> str:
    """Get the path to the native library based on the current platform."""
    system = platform.system().lower()
    machine = platform.machine().lower()

    # Determine the library filename
    if system == "windows":
        lib_name = "unpdf.dll"
        runtime = "win-x64"
    elif system == "darwin":
        lib_name = "libunpdf.dylib"
        if machine in ("arm64", "aarch64"):
            runtime = "osx-arm64"
        else:
            runtime = "osx-x64"
    elif system == "linux":
        lib_name = "libunpdf.so"
        runtime = "linux-musl-x64" if _is_musl() else "linux-x64"
    else:
        raise OSError(f"Unsupported platform: {system}")

    # Look for the library in the package's lib directory
    package_dir = os.path.dirname(os.path.abspath(__file__))
    lib_path = os.path.join(package_dir, "lib", runtime, lib_name)

    if os.path.exists(lib_path):
        return lib_path

    # Fallback: try to find in the package directory directly
    fallback_path = os.path.join(package_dir, "lib", lib_name)
    if os.path.exists(fallback_path):
        return fallback_path

    # Fallback: try system paths
    if system == "windows":
        # Try current directory
        if os.path.exists(lib_name):
            return lib_name
    else:
        # Try LD_LIBRARY_PATH or system paths
        for path in os.environ.get("LD_LIBRARY_PATH", "").split(":"):
            full_path = os.path.join(path, lib_name)
            if os.path.exists(full_path):
                return full_path

    raise OSError(
        f"Could not find unpdf native library. "
        f"Expected at: {lib_path}"
    )


def _load_library():
    """Load the native library."""
    lib_path = _get_lib_path()

    try:
        lib = ctypes.CDLL(lib_path)
    except OSError as e:
        raise OSError(f"Failed to load unpdf library from {lib_path}: {e}")

    # Define function signatures

    # unpdf_to_markdown
    lib.unpdf_to_markdown.argtypes = [c_char_p]
    lib.unpdf_to_markdown.restype = UnpdfResult

    # unpdf_to_text
    lib.unpdf_to_text.argtypes = [c_char_p]
    lib.unpdf_to_text.restype = UnpdfResult

    # unpdf_to_json
    lib.unpdf_to_json.argtypes = [c_char_p, c_bool]
    lib.unpdf_to_json.restype = UnpdfResult

    # unpdf_get_info
    lib.unpdf_get_info.argtypes = [c_char_p]
    lib.unpdf_get_info.restype = UnpdfResult

    # unpdf_get_page_count
    lib.unpdf_get_page_count.argtypes = [c_char_p]
    lib.unpdf_get_page_count.restype = c_int

    # unpdf_is_pdf
    lib.unpdf_is_pdf.argtypes = [c_char_p]
    lib.unpdf_is_pdf.restype = c_bool

    # unpdf_free_result
    lib.unpdf_free_result.argtypes = [UnpdfResult]
    lib.unpdf_free_result.restype = None

    # unpdf_version
    lib.unpdf_version.argtypes = []
    lib.unpdf_version.restype = c_char_p

    return lib


# Global library instance
_lib = None


def get_library():
    """Get the native library instance (lazy loading)."""
    global _lib
    if _lib is None:
        _lib = _load_library()
    return _lib
