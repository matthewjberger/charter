# charter Development Guide

## Overview

charter is a fast async CLI tool that generates token-dense structural context for Rust and Python codebases. It produces a `.charter/` directory containing parsed symbol information, type relationships, and cross-references optimized for LLM consumption. Charter automatically detects project type (Rust, Python, or mixed) and generates appropriate output for each language.

## Architecture

### Two-Phase Pipeline

**Phase 1 (Capture):** Parallel fan-out/join
- Walk: `ignore::WalkParallel` collects all `.rs` and `.py` files
- Cache check: Match `(path, size, mtime)` or blake3 hash
- Parse: tree-sitter with thread-local parser pool (Rust or Python grammar)
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
    parse.rs           - parser dispatch and shared utilities
    parse/
      rust.rs          - tree-sitter Rust extraction
      python.rs        - tree-sitter Python extraction
  extract.rs           - extraction types
  extract/
    language.rs        - Language enum (Rust, Python)
    symbols.rs         - Symbol, SymbolKind, etc.
    imports.rs         - use statement types
    attributes.rs      - derive/cfg types
    complexity.rs      - cyclomatic complexity metrics
    calls.rs           - call graph types
    errors.rs          - error propagation types
    safety.rs          - SafetyInfo, PythonSafetyInfo
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
charter [path]                    - Generate/update the charter (default)
charter read [tier] [--since ref] - Dump context to stdout
charter status                    - Quick summary
charter lookup <symbol>           - Look up a single symbol
charter query "<query>"           - Search for symbols, callers, callees, etc.
charter deps [--crate name]       - Analyze external dependency usage
charter tests [--file path]       - Map tests to source files
charter session start|end|status  - Manage session state
```

Tiers: `quick` (overview only), `default` (overview + symbols + types), `full` (everything)

Query types: `callers of X`, `callees of X`, `implementors of X`, `users of X`, `errors in X`, `hotspots`, `public api`

## Key Dependencies

- `tokio` - async runtime
- `ignore` - parallel directory walking
- `tree-sitter` + `tree-sitter-rust` + `tree-sitter-python` - AST parsing
- `blake3` - fast hashing
- `bincode` - cache serialization
- `clap` - CLI parsing
- `serde` + `serde_json` - serialization
- `chrono` - timestamps

## Output Files

### Core
- `overview.md` - workspace structure, module tree, entry points (Rust crates + Python packages)
- `symbols.md` - complete symbol index with signatures (classes, functions, variables)
- `types.md` - trait definitions, impl map, derive map; Python Protocols, ABCs, class hierarchy
- `refs.md` - cross-reference index (PascalCase types)
- `dependents.md` - inverse dependency map
- `manifest.md` - file manifest with roles and churn

### Analysis
- `calls.md` - call graph + reverse call graph ("Callers" section)
- `clusters.md` - semantic function groupings by affinity
- `dataflow.md` - type producers/consumers, field access patterns
- `hotspots.md` - high-complexity functions by importance score
- `errors.md` - error propagation patterns and origins (Rust Result/Option, Python raise/assert)
- `safety.md` - unsafe blocks, panic points, async patterns; Python dangerous calls (eval, exec, subprocess, pickle, ctypes)
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
