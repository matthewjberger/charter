# atlas Development Guide

## Overview

atlas is a fast async CLI tool that generates token-dense structural context for Rust codebases. It produces a `.atlas/` directory containing parsed symbol information, type relationships, and cross-references optimized for LLM consumption.

## Architecture

### Two-Phase Pipeline

**Phase 1 (Capture):** Parallel fan-out/join
- Walk: `ignore::WalkParallel` collects all `.rs` files
- Cache check: Match `(path, size, mtime)` or blake3 hash
- Parse: tree-sitter with thread-local parser pool
- Extract: symbols, imports, complexity, call graph, error propagation
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
  deps.rs              - dependency analysis command
  query.rs             - query engine
  session.rs           - session state tracking
  tests.rs             - test coverage mapping
  git.rs               - async git operations
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
    complexity.rs      - cyclomatic complexity metrics
    calls.rs           - call graph types
    errors.rs          - error propagation types
  output.rs            - output orchestration
  output/
    overview.rs        - workspace/module tree
    symbols.rs         - symbol index
    type_map.rs        - traits/impls/derives
    refs.rs            - cross-references
    dependents.rs      - inverse dependencies
    manifest.rs        - file manifest
    hotspots.rs        - complexity hotspots
    calls.rs           - call graph + reverse call graph
    clusters.rs        - semantic function groupings
    dataflow.rs        - type flow tracking
    errors.rs          - error propagation output
    safety.rs          - unsafe/panic/async analysis
    snippets.rs        - captured function bodies
    skipped.rs         - skipped files
    preamble.rs        - LLM preamble
  cache.rs             - cache management
  cache/
    types.rs           - CacheEntry, FileData
```

## Commands

```
atlas [path]                    - Generate/update the atlas (default)
atlas read [tier] [--since ref] - Dump context to stdout
atlas status                    - Quick summary
atlas lookup <symbol>           - Look up a single symbol
atlas query "<query>"           - Search for symbols, callers, callees, etc.
atlas deps [--crate name]       - Analyze external dependency usage
atlas tests [--file path]       - Map tests to source files
atlas session start|end|status  - Manage session state
```

Tiers: `quick` (overview only), `default` (overview + symbols + types), `full` (everything)

Query types: `callers of X`, `callees of X`, `implementors of X`, `users of X`, `errors in X`, `hotspots`, `public api`

## Key Dependencies

- `tokio` - async runtime
- `ignore` - parallel directory walking
- `tree-sitter` + `tree-sitter-rust` - AST parsing
- `blake3` - fast hashing
- `bincode` - cache serialization
- `clap` - CLI parsing
- `serde` + `serde_json` - serialization
- `chrono` - timestamps

## Output Files

### Core
- `overview.md` - workspace structure, module tree, entry points
- `symbols.md` - complete symbol index with signatures
- `types.md` - trait definitions, impl map, derive map
- `refs.md` - cross-reference index (PascalCase types)
- `dependents.md` - inverse dependency map
- `manifest.md` - file manifest with roles and churn

### Analysis
- `calls.md` - call graph + reverse call graph ("Callers" section)
- `clusters.md` - semantic function groupings by affinity
- `dataflow.md` - type producers/consumers, field access patterns
- `hotspots.md` - high-complexity functions by importance score
- `errors.md` - error propagation patterns and origins
- `safety.md` - unsafe blocks, panic points, async patterns
- `snippets.md` - captured function bodies for important code

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
