# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**unpdf** is a high-performance Rust library for extracting PDF documents into structured Markdown, text, and JSON. It provides:
- CLI tool (`unpdf-cli` crate)
- Rust library (`unpdf` crate)
- C-ABI FFI bindings for Python and C#/.NET integration

## Build Commands

```bash
cargo build                    # Build library
cargo build --release          # Release build
cargo test                     # Run all tests
cargo clippy                   # Lint
cargo fmt                      # Format code
cargo doc --open               # Generate and view documentation

# CLI
cargo run -p unpdf-cli -- document.pdf

# FFI build
cargo build --release --features ffi
```

## Version Bump Checklist

When bumping version, **ALL** of the following files must be updated simultaneously.
**bindings/ 하위 파일을 빠뜨리지 말 것** — 누락 시 릴리스 불일치 발생.

```
Cargo.toml                           # Root library version
cli/Cargo.toml                       # CLI version (must match)
bindings/python/pyproject.toml       # Python package version
bindings/csharp/Unpdf/Unpdf.csproj   # C# package version
```

**Important**: CLI version mismatch causes "update available" message to appear even after updating. All versions must be in sync before creating a GitHub release tag.

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `lopdf` | PDF parsing (wrapped by `PdfBackend` trait) |
| `serde` + `serde_json` | JSON serialization |
| `thiserror` | Ergonomic error types |
| `rayon` | Parallel processing |
| `clap` | CLI argument parsing |
| `self_update` | GitHub release updates |

## Feature Flags

| Flag | Purpose | Default |
|------|---------|---------|
| `fast-parse` | nom-based faster PDF parsing | Yes |
| `async` | Tokio async I/O | No |
| `ffi` | C-ABI bindings | No |
