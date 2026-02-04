# Architecture

This document describes the internal architecture of charter, a CLI tool that generates token-dense structural context for Rust codebases, optimized for LLM consumption.

## Overview

Charter analyzes Rust projects using tree-sitter parsing and produces a `.charter/` directory containing markdown files with symbols, type relationships, call graphs, and cross-references. The tool is designed for speed (parallel processing, incremental caching) and output quality (high signal-to-noise ratio for LLMs).

```
┌─────────────────────────────────────────────────────────────────┐
│                         Charter Pipeline                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │   Phase 1    │    │   Phase 2    │    │    Output    │       │
│  │   Capture    │───▶│  References  │───▶│   Writers    │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
│         │                   │                   │                │
│         ▼                   ▼                   ▼                │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │ Walk files   │    │ Build symbol │    │ overview.md  │       │
│  │ Parse AST    │    │ table        │    │ symbols.md   │       │
│  │ Extract info │    │ Match refs   │    │ calls.md     │       │
│  │ Cache check  │    │              │    │ clusters.md  │       │
│  └──────────────┘    └──────────────┘    │ dataflow.md  │       │
│                                          │ hotspots.md  │       │
│                                          │ ...          │       │
│                                          └──────────────┘       │
└─────────────────────────────────────────────────────────────────┘
```

## Module Structure

```
src/
├── main.rs           # CLI entry point, argument parsing
├── cli.rs            # clap derive CLI definitions
├── detect.rs         # Project/workspace detection
├── deps.rs           # Dependency analysis command
├── query.rs          # Query engine for symbol lookup
├── session.rs        # Session state tracking
├── tests.rs          # Test coverage mapping
├── git.rs            # Async git operations
├── pipeline.rs       # Two-phase orchestrator
├── pipeline/
│   ├── walk.rs       # Parallel directory walking
│   ├── read.rs       # Async file reading
│   └── parse.rs      # Tree-sitter Rust extraction (~2500 lines)
├── extract.rs        # Extraction type definitions
├── extract/
│   ├── symbols.rs    # Symbol, SymbolKind, InherentImpl
│   ├── imports.rs    # Use statement types
│   ├── attributes.rs # Derive/cfg types
│   ├── complexity.rs # Cyclomatic complexity metrics
│   ├── calls.rs      # Call graph types
│   ├── errors.rs     # Error propagation types
│   └── safety.rs     # Unsafe/lifetime/async info
├── output.rs         # Output orchestration, lookup, peek
└── output/
    ├── overview.rs   # Workspace structure, module tree
    ├── symbols.rs    # Symbol index with signatures
    ├── type_map.rs   # Trait definitions, impl map
    ├── refs.rs       # Cross-reference index
    ├── dependents.rs # Inverse dependency map
    ├── manifest.rs   # File manifest with roles
    ├── hotspots.rs   # Complexity hotspots
    ├── calls.rs      # Call graph + reverse call graph
    ├── clusters.rs   # Semantic function grouping
    ├── dataflow.rs   # Type flow analysis
    ├── errors.rs     # Error propagation output
    ├── safety.rs     # Unsafe/lifetime/async summary
    ├── snippets.rs   # Captured function bodies
    ├── skipped.rs    # Skipped files list
    └── preamble.rs   # LLM-optimized summary
```

### Module Responsibilities

| Module | Responsibility |
|--------|----------------|
| `pipeline` | Orchestrate two-phase capture, manage caching |
| `pipeline/parse` | Tree-sitter AST traversal, symbol extraction |
| `extract/*` | Type definitions for extracted information |
| `output/*` | Generate markdown files from PipelineResult |
| `detect` | Find Cargo.toml, workspace members, crate types |
| `query` | Parse and execute queries (callers, callees, etc.) |

## Two-Phase Pipeline

### Phase 1: Capture

Parallel fan-out/join pattern for maximum throughput:

```
┌─────────────────────────────────────────────────────────────┐
│                        Phase 1: Capture                      │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. Walk         ignore::WalkParallel collects .rs files    │
│       │                                                      │
│       ▼                                                      │
│  2. Cache Check  Match (path, size, mtime) or blake3 hash   │
│       │                                                      │
│       ├─── Hit ──▶ Return cached ParsedFile                 │
│       │                                                      │
│       ▼                                                      │
│  3. Parse        Tree-sitter with thread-local parser pool  │
│       │                                                      │
│       ▼                                                      │
│  4. Extract      Symbols, imports, complexity, calls,       │
│       │          error propagation, safety info             │
│       │                                                      │
│       ▼                                                      │
│  5. Join         tokio::JoinSet collects all FileResults    │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Phase 2: References

Fast in-memory pass after all files are parsed:

```
┌─────────────────────────────────────────────────────────────┐
│                      Phase 2: References                     │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. Build symbol table from Phase 1 (PascalCase types)      │
│                                                              │
│  2. For each file's cached identifier locations:            │
│     - Match against symbol table                            │
│     - Record (file, line) → symbol mappings                 │
│                                                              │
│  3. Write refs.md with cross-references                     │
│                                                              │
│  No additional file I/O needed                              │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Data Structures

### PipelineResult

The main output of Phase 1, consumed by all output writers:

```rust
struct PipelineResult {
    files: Vec<FileResult>,
    workspace: WorkspaceInfo,
    git_info: Option<GitInfo>,
}

struct FileResult {
    relative_path: String,
    parsed: ParsedFile,
    churn: u32,           // Git commit count
}

struct ParsedFile {
    symbols: FileSymbols,
    imports: Vec<Import>,
    call_graph: Vec<CallInfo>,
    error_info: ErrorInfo,
    safety_info: SafetyInfo,
    // ... more fields
}
```

### Symbol Types

```rust
struct Symbol {
    name: String,
    kind: SymbolKind,
    line: usize,
    visibility: Visibility,
    doc: Option<String>,
}

enum SymbolKind {
    Function { signature: String, is_async: bool, body_summary: Option<BodySummary> },
    Struct { fields: Vec<Field>, derives: Vec<String> },
    Enum { variants: Vec<Variant>, derives: Vec<String> },
    Trait { methods: Vec<TraitMethod>, supertraits: Vec<String> },
    Impl { type_name: String, trait_name: Option<String>, methods: Vec<Method> },
    Const { type_name: String, value: Option<String> },
    Static { type_name: String, is_mut: bool },
    TypeAlias { target: String },
    Macro { kind: MacroKind },
    Module { is_inline: bool },
}
```

### Call Graph

```rust
struct CallInfo {
    caller: CallerInfo,
    callees: Vec<CallEdge>,
    line: usize,
}

struct CallEdge {
    target: String,
    receiver_type: Option<String>,
    is_async: bool,
    is_try: bool,
}
```

## Output Files

| File | Purpose | Key Content |
|------|---------|-------------|
| `overview.md` | Workspace structure | Module tree, entry points, features |
| `symbols.md` | Complete symbol index | All types, functions, traits with signatures |
| `types.md` | Type relationships | Trait definitions, impl map, derive map |
| `refs.md` | Cross-references | Where each type is used |
| `calls.md` | Call graph | Who calls what, reverse call graph |
| `clusters.md` | Semantic grouping | Functions grouped by affinity |
| `dataflow.md` | Type flows | Producer/consumer relationships |
| `hotspots.md` | Complexity analysis | Functions ranked by importance score |
| `dependents.md` | Module dependencies | What depends on what |
| `errors.md` | Error propagation | Error types and flow |
| `safety.md` | Safety analysis | Unsafe blocks, panics, lifetimes |
| `manifest.md` | File listing | All files with roles and churn |
| `snippets.md` | Code snippets | Important function bodies |

## Algorithms

### Hotspot Scoring

Functions are scored to identify critical code paths:

```
score = (cyclomatic_complexity × 2)
      + (lines / 10)
      + (call_sites × 3)
      + (git_churn × 2)
      + (is_public ? 10 : 0)
```

Thresholds:
- High importance: score ≥ 30
- Medium importance: score 15-29
- Low importance: score < 15 (not shown)

### Function Clustering

Groups functions by semantic affinity using an affinity matrix:

```
Affinity Score Calculation:
┌────────────────────────────────────────────┬────────┐
│ Condition                                  │ Score  │
├────────────────────────────────────────────┼────────┤
│ Same impl type AND same file               │ +15    │
│ Same impl type AND same crate              │ +5     │
│ A calls B or B calls A                     │ +5     │
│ Same file                                  │ +5     │
│ Same crate (different file)                │ +2     │
│ Different crate                            │ -3     │
│ Per shared non-primitive type in signature │ +2     │
└────────────────────────────────────────────┴────────┘

Clustering:
- Threshold: 10 (pairs with score < 10 don't cluster)
- Max cluster size: 100 (prevents mega-clusters)
- Greedy merge: highest-scoring pairs first
```

### Type Flow Analysis

Tracks where types are produced and consumed:

```
For each function:
  1. Extract return type → mark as "produced by" this function
  2. Extract parameter types → mark as "consumed by" this function

Cross-module flows:
  - Group by module path
  - Find types that flow between modules
  - Report coupling via shared types
```

## Caching Strategy

### Cache Key

```rust
struct CacheKey {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}
```

Fast path: If (path, size, mtime) matches, return cached result.
Fallback: Compute blake3 hash for content comparison.

### Cache Storage

Binary format using bincode for fast serialization:

```
.charter/
├── cache.bin      # Serialized HashMap<PathBuf, CachedFile>
└── meta.json      # Timestamp, git hash, file counts
```

### Invalidation

```
On file change:
  1. Quick check: (size, mtime) changed → invalidate
  2. Deep check: blake3 hash changed → invalidate

On delete:
  - Remove from cache, regenerate outputs

On add:
  - Parse new file, add to cache
```

## Performance Targets

| Scenario | Target |
|----------|--------|
| Cold capture, 500 files | < 3s |
| Cold capture, 5000 files | < 15s |
| Warm (0 changes) | < 100ms |
| Warm (10 changes) | < 500ms |
| `charter read` | < 50ms |

### Parallelism

- File walking: `ignore::WalkParallel` (respects .gitignore)
- Parsing: Thread-local tree-sitter parsers (one per thread)
- I/O: Async via tokio, buffered writes
- Output: Sequential (fast enough, simpler)

## Commands

```
charter [path]              # Generate/update the charter
charter read [tier]         # Dump context to stdout
charter status              # Quick summary
charter lookup <symbol>     # Look up a single symbol
charter query "<query>"     # Search (callers, callees, etc.)
charter deps [--crate name] # Analyze dependencies
charter tests [--file path] # Map tests to source files
```

### Read Tiers

| Tier | Content |
|------|---------|
| `quick` | Overview only |
| `default` | Overview + symbols + types |
| `full` | Everything |

### Query Types

```
callers of X      # What functions call X?
callees of X      # What does X call?
implementors of X # What types implement trait X?
users of X        # What uses type X?
errors in X       # What errors can X return?
hotspots          # High-complexity functions
public api        # Public interface summary
```

## Error Handling

- No `unwrap()` outside tests
- All I/O errors are propagated via `anyhow::Result`
- Malformed Rust files are skipped (tracked in `skipped.md`)
- Parse errors don't crash; file is marked as skipped
