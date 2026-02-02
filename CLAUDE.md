# atlas Development Guide

## Overview

atlas is a fast async CLI tool that generates token-dense structural context for Rust codebases. It produces a `.atlas/` directory containing parsed symbol information, type relationships, and cross-references optimized for LLM consumption.

## Architecture

### Two-Phase Pipeline

**Phase 1 (Capture):** Parallel fan-out/join
- Walk: `ignore::WalkParallel` collects all `.rs` files
- Cache check: Match `(path, size, mtime)` or blake3 hash
- Parse: tree-sitter with thread-local parser pool
- Join: `JoinSet` collects results
- Emit: Streaming `BufWriter` per output file

**Phase 2 (References):** Fast in-memory pass
- Build complete PascalCase symbol table from Phase 1
- Match cached identifier locations against symbol table
- Write refs.md with no additional I/O

### Module Layout (2024 Edition)

No `mod.rs` files anywhere. Module roots use `foo.rs` + `foo/` directory pattern.

```
src/
  main.rs              - entry point
  cli.rs               - clap derive CLI
  detect.rs            - project/workspace detection
  pipeline.rs          - two-phase orchestrator
  pipeline/
    walk.rs            - parallel directory walking
    read.rs            - mmap + async file reading
    parse.rs           - tree-sitter Rust extraction
  extract.rs           - extraction types
  extract/
    symbols.rs         - Symbol, SymbolKind, etc.
    imports.rs         - use statement types
    attributes.rs      - derive/cfg types
  output.rs            - output orchestration
  output/
    overview.rs        - workspace/module tree
    symbols.rs         - symbol index
    type_map.rs        - traits/impls/derives
    refs.rs            - cross-references
    dependents.rs      - inverse dependencies
    manifest.rs        - file manifest
    skipped.rs         - skipped files
    preamble.rs        - LLM preamble
  cache.rs             - cache management
  cache/
    types.rs           - CacheEntry, FileData
  git.rs               - async git operations
```

## Commands

```
atlas [path]           - Generate/update the atlas (default)
atlas read [tier]      - Dump context to stdout
atlas status           - Quick summary
```

Tiers: `quick` (overview only), `default` (overview + symbols + types), `full` (everything)

## Key Dependencies

- `tokio` - async runtime
- `ignore` - parallel directory walking
- `tree-sitter` + `tree-sitter-rust` - AST parsing
- `blake3` - fast hashing
- `bincode` - cache serialization
- `clap` - CLI parsing
- `serde` + `serde_json` - serialization

## Performance Targets

- Cold capture, 500 files: < 3s
- Cold capture, 5000 files: < 15s
- Warm (0 changes): < 100ms
- Warm (10 changes): < 500ms
- read: < 50ms

## Coding Conventions

- No `unwrap()` outside tests
- No `std::fs` - use `tokio::fs`
- No sequential I/O when parallel is possible
- Write to buffers, not directly to files
- Thread-local tree-sitter parsers for performance
