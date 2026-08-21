"""Pytest harness for deterministic local unpdf binding verification."""

from __future__ import annotations

import os
import platform
import sys
from pathlib import Path


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def _python_src_dir() -> Path:
    return _repo_root() / "bindings" / "python" / "src"


def _native_library_name() -> str:
    system = platform.system()
    if system == "Windows":
        return "unpdf.dll"
    if system == "Darwin":
        return "libunpdf.dylib"
    return "libunpdf.so"


def _built_native_library_path() -> Path:
    return _repo_root() / "target" / "release" / _native_library_name()


def _configure_python_path() -> None:
    # Only prepend the source tree to sys.path for local development where
    # the caller has also built the native library at target/release. In
    # CI's test-python job, the installed wheel bundles the native library
    # under its own package dir and must not be shadowed by a bare source
    # tree (which has no `lib/` populated in a fresh checkout).
    if not _built_native_library_path().exists():
        return
    python_src = str(_python_src_dir())
    if python_src not in sys.path:
        sys.path.insert(0, python_src)


def _configure_native_library_path() -> None:
    if os.environ.get("UNPDF_LIB_PATH"):
        return

    built_library = _built_native_library_path()
    if built_library.exists():
        os.environ["UNPDF_LIB_PATH"] = str(built_library)


def pytest_configure() -> None:
    _configure_python_path()
    _configure_native_library_path()


def pytest_report_header() -> str:
    configured_path = os.environ.get("UNPDF_LIB_PATH")
    if configured_path:
        return f"UNPDF_LIB_PATH={configured_path}"
    return f"UNPDF_LIB_PATH not set (expected build output: {_built_native_library_path()})"
