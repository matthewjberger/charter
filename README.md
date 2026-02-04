<h1 align="center">charter</h1>

<p align="center">
  <a href="https://github.com/matthewjberger/charter"><img alt="github" src="https://img.shields.io/badge/github-matthewjberger/charter-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20"></a>
  <a href="https://crates.io/crates/charter"><img alt="crates.io" src="https://img.shields.io/crates/v/charter.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20"></a>
  <a href="https://github.com/matthewjberger/charter/blob/main/LICENSE-MIT"><img alt="license" src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue?style=for-the-badge&labelColor=555555" height="20"></a>
</p>

<p align="center"><strong>Structural context for LLMs, in seconds.</strong></p>

charter generates a `.charter/` directory containing token-dense structural context for Rust codebases. When you're working with an LLM that's lost track of your codebase (after context compaction, or in a new session), `charter read` dumps everything it needs to re-orient: symbol locations, call graphs, type flows, semantic clusters, and more.

## Installation

```bash
cargo install --path .
```

## Quick Start

```bash
# In your Rust project root:
charter              # Generate .charter/ directory
charter read         # Dump context to stdout (pipe to LLM or copy/paste)
```

That's it. Run `charter` once to capture, then `charter read` whenever you need to reload context.

## Commands

### `charter`

Generates or updates the `.charter/` directory. Incremental — only re-parses files that changed.

```
$ charter

Charter @ a3f8c2d → b7e1d4f | 3 modified, 1 added, 0 removed

  modified: src/ecs/query.rs (+2 symbols, signature changed: fn execute)
  modified: src/render/pipeline.rs (fields changed on RenderState)
  added: src/render/postprocess.rs (14 symbols)

Captured @ b7e1d4f (316 files, 89,421 lines)
  parsed: 4, cached: 312, skipped: 0
```

### `charter read [tier]`

Dumps structural context to stdout. Three tiers control how much context:

| Tier | Contents | Use when |
|------|----------|----------|
| `quick` | overview.md only | Just need orientation |
| `default` | overview + symbols + types + dependents | Normal usage |
| `full` | Everything | Deep refactoring, cross-cutting changes |

```bash
charter read          # default tier
charter read quick    # minimal
charter read full     # everything
```

**Options:**
- `--focus <path>` — Filter output to a specific directory or file
- `--since <ref>` — Show changes since a git ref (marks files with `[+]` added, `[~]` modified, `[-]` deleted)

```bash
charter read --focus src/pipeline    # Only show src/pipeline/**
charter read --since HEAD~5          # Highlight changes in last 5 commits
```

### `charter lookup <symbol>`

Look up a single symbol with full context:

```
$ charter lookup PipelineResult

PipelineResult [struct] defined at src/pipeline.rs
  pub struct PipelineResult {
    pub files: Vec<FileResult>,
    pub workspace: WorkspaceInfo,
    pub git_info: Option<GitInfo>,
    pub total_lines: usize,
    pub skipped: Vec<SkippedFile>,
    pub diff_summary: Option<DiffSummary>,
  }

  Derives: Debug, Default
  Referenced in 12 files:
    src/output/calls.rs, src/output/clusters.rs, src/output/dataflow.rs, src/output/dependents.rs
```

### `charter query "<query>"`

Search for symbols, relationships, and patterns:

```bash
charter query "callers of write_calls"     # What functions call write_calls?
charter query "callees of capture"         # What does capture() call?
charter query "implementors of Default"    # What types implement Default?
charter query "users of Cache"             # What files use the Cache type?
charter query "errors in pipeline.rs"      # Error propagation in a file
charter query "hotspots"                   # High-complexity functions
charter query "public api"                 # Public symbols only
```

### `charter deps [--crate <name>]`

Analyze external dependency usage:

```
$ charter deps --crate tokio

tokio (version from Cargo.toml)
  Used in 12 files, 47 imports

  Items used:
    fs::read_to_string (8 files)
    sync::Mutex (5 files)
    task::spawn (4 files)
    ...
```

### `charter tests [--file <path>]`

Map tests to source files:

```
$ charter tests --file src/cache.rs

Tests covering src/cache.rs:
  tests/cache_tests.rs
    test_cache_load
    test_cache_save
    test_cache_invalidation
```

### `charter session start|end|status`

Track what changed during a work session:

```bash
charter session start    # Mark session start
# ... do work ...
charter session status   # See what changed
charter session end      # End session tracking
```

### `charter status`

Quick summary without dumping full context:

```
$ charter status
charter status
  files: 316
  lines: 89,421
  captured: 2025-01-31T14:23:07Z
  commit: a3f8c2d
```

## Output Files

The `.charter/` directory contains structured context optimized for LLM consumption:

### Core Files

| File | Contents | Example |
|------|----------|---------|
| `overview.md` | Workspace structure, module tree, entry points, features | Crate hierarchy, bin/lib targets |
| `symbols.md` | Complete symbol index with full signatures | Every struct, enum, fn, trait with fields/variants |
| `types.md` | Trait definitions, impl map, derive map | `Default -> [Cache, Config, State]` |
| `refs.md` | Cross-reference index | `PipelineResult` used in 12 files |
| `dependents.md` | Inverse dependency map | What breaks if you change a file |

### Analysis Files

| File | Contents | Example |
|------|----------|---------|
| `calls.md` | Call graph + reverse call graph | `node_text` has 47 callers |
| `clusters.md` | Semantic function groupings | 87 parse functions work together |
| `dataflow.md` | Type flow tracking | `Cache` produced by X, consumed by Y |
| `hotspots.md` | High-complexity functions | Ranked by cyclomatic complexity + churn |
| `errors.md` | Error propagation patterns | Where errors originate, how they flow |
| `safety.md` | Unsafe blocks, panic points, async patterns | Safety-critical code locations |
| `snippets.md` | Captured function bodies | Important function implementations |
| `manifest.md` | File manifest with roles and churn | `[source]` `[test]` `[churn:high]` |

### Internal Files

| File | Purpose |
|------|---------|
| `cache.bin` | Incremental update cache |
| `meta.json` | Capture metadata (timestamp, commit, file count) |
| `FORMAT.md` | Format specification for the output files |

The `.charter/` directory is auto-gitignored.

## Output Format Examples

### symbols.md

```markdown
src/cache.rs [35 lines] [source] [churn:med]
  pub struct Cache { entries: HashMap<String, CacheEntry> }
    impl Cache:
      pub fn load(path: &Path) -> Result<Self>
      pub fn save(&self, path: &Path) -> Result<()>
      pub fn get(&self, path: &str) -> Option<&CacheEntry>
```

### calls.md — Call Map

```markdown
## Call Map

src/pipeline.rs [12 functions, 87 calls]
  capture → emit_outputs.await?, run_phase1_with_walk.await?, build_cache
  process_file → parse::parse_rust_file?, read::read_file.await?
```

### calls.md — Reverse Call Graph (Callers)

```markdown
## Callers

node_text [47 callers]
  extract_struct (src/pipeline/parse.rs:151)
  extract_enum (src/pipeline/parse.rs:202)
  extract_function (src/pipeline/parse.rs:465)
  [+44 more]
```

### clusters.md

```markdown
## Cluster 1: parse operations (87 functions)

src/pipeline/parse.rs:
  extract_struct (line 151)
  extract_enum (line 202)
  extract_function (line 465)
  ...

Internal calls: 234, External calls: 45
```

### dataflow.md

```markdown
## Type Flows

PipelineResult
  produced by: capture (src/pipeline.rs:135)
  consumed by: emit_outputs, write_calls, write_clusters [+35 more]

Cache
  produced by: build_cache (src/pipeline.rs:503)
  consumed by: process_file, quick_change_check_sync
```

### hotspots.md

```markdown
## High Importance

parse_rust_file [score: 89] (src/pipeline/parse.rs:73)
  cyclomatic: 12, lines: 156, calls: 47, public
  Called by: process_file

extract_items [score: 67] (src/pipeline/parse.rs:129)
  cyclomatic: 8, lines: 89, calls: 23
```

## Staleness Detection

If files have changed since capture, `charter read` warns you:

```
⚠ 3 files changed since capture (a3f8c2d → b7e1d4f):
  M src/ecs/world.rs
  M src/render/pipeline.rs
  A src/render/postprocess.rs

Structural context below may be inaccurate for these files. Read them directly for current state.
```

## Preamble

Every `charter read` includes a project-specific preamble:

```
[charter @ a3f8c2d | 2025-01-31T14:23:07Z | 316 files | 89,421 lines]

Rust workspace with 4 crates. Primary: my-engine (lib).
Entry points: my-app (bin), 12 examples, 3 benches

Top traits by impl count:
  Component (34 impls), System (12 impls), State (6 impls)

Most-depended-on files:
  src/lib.rs (56), src/ecs/world.rs (47), src/math/vec3.rs (38)

Top referenced types:
  Entity (89), Transform (67), Handle (45)

High-churn files:
  main.rs, pipeline.rs, widgets.rs
```

## CLAUDE.md Integration

Add this to your project's `CLAUDE.md`:

```markdown
## Codebase Context

This project uses charter for structural context. If you've lost track of the codebase:

- `charter read quick` — Orientation only (~6k tokens)
- `charter read` — Standard context (~40k tokens)
- `charter read full` — Everything (~60k tokens)

Key files in .charter/:
- `symbols.md` — All type/function signatures
- `calls.md` — Who calls what (and reverse: what calls whom)
- `clusters.md` — Semantically related functions
- `dataflow.md` — Type producers/consumers

For specific lookups:
- `charter lookup <Symbol>` — Full context for one symbol
- `charter query "callers of X"` — Find all callers
```

## Performance

| Operation | 500 files | 5000 files |
|-----------|-----------|------------|
| Cold capture | < 3s | < 15s |
| Warm (0 changes) | < 100ms | < 100ms |
| Warm (10 changes) | < 500ms | < 500ms |
| `charter read` | < 50ms | < 50ms |

## How It Works

**Phase 1 — Parallel Capture:**
- `ignore::WalkParallel` collects all `.rs` files
- Cache check: match `(path, size, mtime)` or blake3 hash
- `tree-sitter` parses each file with thread-local parser pool
- Extract: symbols, imports, complexity, call graph, error propagation
- `JoinSet` collects results in parallel

**Phase 2 — Reference Resolution:**
- Build PascalCase symbol table from Phase 1
- Match identifier locations against symbol table
- Write cross-references with no additional I/O

## Known Limitations

1. **build.rs generated code** — Files generated in `OUT_DIR` are invisible
2. **Procedural macros** — Derive expansions are tracked but internals are opaque
3. **Name-based resolution** — Method calls use name matching, not type inference
4. **External crates** — Only tracks usage patterns, not external API shapes

## License

Dual-licensed under MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache 2.0 ([LICENSE-APACHE](LICENSE-APACHE)).
