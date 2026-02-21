"""Native library loading for unpdf."""

import ctypes
import platform
import os
import subprocess
from pathlib import Path

# Library filename by platform
_LIB_NAMES = {
    "Windows": "unpdf.dll",
    "Linux": "libunpdf.so",
    "Darwin": "libunpdf.dylib",
}


def _is_musl() -> bool:
    """Detect if the current Linux system uses musl libc."""
    # Check if /etc/os-release indicates Alpine
    try:
        osrelease = Path("/etc/os-release").read_text()
        if "alpine" in osrelease.lower():
            return True
    except OSError:
        pass
    # Check ldd version output (musl ldd identifies itself)
    try:
        result = subprocess.run(
            ["ldd", "--version"], capture_output=True, text=True, timeout=5
        )
        output = result.stdout + result.stderr
        if "musl" in output.lower():
            return True
    except (OSError, subprocess.TimeoutExpired):
        pass
    return False


def _get_linux_runtime_id(machine: str) -> str:
    """Get the runtime ID for Linux, detecting musl vs glibc."""
    if machine == "x86_64":
        return "linux-musl-x64" if _is_musl() else "linux-x64"
    raise OSError(f"Unsupported Linux architecture: {machine}")


# Runtime identifier
_RUNTIME_IDS = {
    ("Windows", "AMD64"): "win-x64",
    ("Windows", "x86_64"): "win-x64",
    ("Darwin", "x86_64"): "osx-x64",
    ("Darwin", "arm64"): "osx-arm64",
}


def _get_lib_path() -> Path:
    """Get the path to the native library."""
    system = platform.system()
    machine = platform.machine()

    lib_name = _LIB_NAMES.get(system)
    if not lib_name:
        raise OSError(f"Unsupported platform: {system}")

    # Check UNPDF_LIB_PATH environment variable first
    env_path = os.environ.get("UNPDF_LIB_PATH")
    if env_path:
        p = Path(env_path)
        if p.exists():
            return p

    if system == "Linux":
        runtime_id = _get_linux_runtime_id(machine)
    else:
        runtime_id = _RUNTIME_IDS.get((system, machine))
        if not runtime_id:
            raise OSError(f"Unsupported architecture: {system}/{machine}")

    # Look for the library in the package
    package_dir = Path(__file__).parent
    lib_path = package_dir / "lib" / runtime_id / lib_name

    if lib_path.exists():
        return lib_path

    # Fallback: look in package root
    lib_path = package_dir / "lib" / lib_name
    if lib_path.exists():
        return lib_path

    # Fallback: look in current directory
    lib_path = Path(lib_name)
    if lib_path.exists():
        return lib_path

    # Fallback: system library path
    return Path(lib_name)


def _load_library() -> ctypes.CDLL:
    """Load the native unpdf library."""
    lib_path = _get_lib_path()

    try:
        if platform.system() == "Windows":
            # On Windows, use LoadLibraryEx with LOAD_WITH_ALTERED_SEARCH_PATH
            return ctypes.CDLL(str(lib_path), winmode=0)
        else:
            return ctypes.CDLL(str(lib_path))
    except OSError as e:
        raise OSError(
            f"Failed to load unpdf native library from {lib_path}: {e}\n"
            f"Make sure the library is installed for your platform ({platform.system()}/{platform.machine()})."
        ) from e


# Load the library eagerly
_lib = _load_library()

# Define function signatures
_lib.unpdf_version.argtypes = []
_lib.unpdf_version.restype = ctypes.c_char_p

_lib.unpdf_last_error.argtypes = []
_lib.unpdf_last_error.restype = ctypes.c_char_p

_lib.unpdf_parse_file.argtypes = [ctypes.c_char_p]
_lib.unpdf_parse_file.restype = ctypes.c_void_p

_lib.unpdf_parse_bytes.argtypes = [ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t]
_lib.unpdf_parse_bytes.restype = ctypes.c_void_p

_lib.unpdf_free_document.argtypes = [ctypes.c_void_p]
_lib.unpdf_free_document.restype = None

_lib.unpdf_to_markdown.argtypes = [ctypes.c_void_p, ctypes.c_int]
_lib.unpdf_to_markdown.restype = ctypes.c_char_p

_lib.unpdf_to_text.argtypes = [ctypes.c_void_p]
_lib.unpdf_to_text.restype = ctypes.c_char_p

_lib.unpdf_to_json.argtypes = [ctypes.c_void_p, ctypes.c_int]
_lib.unpdf_to_json.restype = ctypes.c_char_p

_lib.unpdf_plain_text.argtypes = [ctypes.c_void_p]
_lib.unpdf_plain_text.restype = ctypes.c_char_p

_lib.unpdf_section_count.argtypes = [ctypes.c_void_p]
_lib.unpdf_section_count.restype = ctypes.c_int

_lib.unpdf_resource_count.argtypes = [ctypes.c_void_p]
_lib.unpdf_resource_count.restype = ctypes.c_int

_lib.unpdf_get_title.argtypes = [ctypes.c_void_p]
_lib.unpdf_get_title.restype = ctypes.c_char_p

_lib.unpdf_get_author.argtypes = [ctypes.c_void_p]
_lib.unpdf_get_author.restype = ctypes.c_char_p

_lib.unpdf_free_string.argtypes = [ctypes.c_char_p]
_lib.unpdf_free_string.restype = None

_lib.unpdf_get_resource_ids.argtypes = [ctypes.c_void_p]
_lib.unpdf_get_resource_ids.restype = ctypes.c_char_p

_lib.unpdf_get_resource_info.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
_lib.unpdf_get_resource_info.restype = ctypes.c_char_p

_lib.unpdf_get_resource_data.argtypes = [
    ctypes.c_void_p,
    ctypes.c_char_p,
    ctypes.POINTER(ctypes.c_size_t),
]
_lib.unpdf_get_resource_data.restype = ctypes.POINTER(ctypes.c_uint8)

_lib.unpdf_free_bytes.argtypes = [ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t]
_lib.unpdf_free_bytes.restype = None

# Export constants
UNPDF_FLAG_FRONTMATTER = 1
UNPDF_FLAG_ESCAPE_SPECIAL = 2
UNPDF_FLAG_PARAGRAPH_SPACING = 4

UNPDF_JSON_PRETTY = 0
UNPDF_JSON_COMPACT = 1


def get_library():
    """Get the loaded native library."""
    return _lib
