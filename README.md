<h1 align="center">charter 🗺️🌍</h1>

<p align="center">
  <a href="https://github.com/matthewjberger/charter"><img alt="github" src="https://img.shields.io/badge/github-matthewjberger/charter-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20"></a>
  <a href="https://crates.io/crates/charter"><img alt="crates.io" src="https://img.shields.io/crates/v/charter.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20"></a>
  <a href="https://github.com/matthewjberger/charter/blob/main/LICENSE-MIT"><img alt="license" src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue?style=for-the-badge&labelColor=555555" height="20"></a>
</p>

<p align="center"><strong>Structural codebase intelligence for LLMs, via MCP.</strong></p>

<p align="center">
  <code>cargo install charter</code>
</p>

charter is an [MCP](https://modelcontextprotocol.io/) server that provides structural codebase intelligence for **Rust** and **Python** projects. It parses your codebase with tree-sitter, builds an in-memory index of symbols, call graphs, type hierarchies, and cross-references, then exposes it all through structured JSON tools.

## Installation

```bash
cargo install charter        # from crates.io
cargo install --path .       # from source
```

## Quick Start

Add charter as an MCP server in your client configuration (Claude Desktop, Claude Code, etc.):

```json
{
  "mcpServers": {
    "charter": {
      "command": "charter",
      "args": ["serve", "."]
    }
  }
}
```

The server scans on startup, holds the full parsed index in memory, and responds to queries in sub-millisecond time. Use the `rescan` tool to pick up changes without restarting.

## MCP Tools

`charter serve [path]` starts the MCP server over stdio. All tools return structured JSON.

```bash
charter serve           # serve current directory
charter serve /path     # serve a specific project
```

| Tool | Parameters | Description |
|------|-----------|-------------|
| `search_symbols` | `query`, `kind?`, `limit?` | Fuzzy/partial search across all symbols |
| `find_symbol` | `name`, `kind?` | Exact or fuzzy lookup of a specific symbol |
| `find_implementations` | `symbol` | Trait implementors, type methods, derive-generated impls |
| `find_callers` | `symbol` | All call sites with caller name, file, and line |
| `find_dependencies` | `symbol`, `direction` | Upstream/downstream/both dependencies with file:line |
| `get_module_tree` | `root?` | File paths with symbol counts |
| `get_type_hierarchy` | `symbol` | Trait inheritance, derives, supertraits, base classes |
| `summarize` | `scope?` | Architectural summary with counts and complexity hotspots |
| `get_snippet` | `name` | Captured function bodies with importance scores |
| `read_source` | `file`, `start_line`, `end_line?` | Read any source range from indexed files |
| `search_text` | `pattern`, `glob?`, `case_insensitive?`, `context_lines?`, `max_results?` | Regex text search across indexed files with context |
| `rescan` | — | Re-scan codebase and persist cache |

## Capture

Running `charter` with no subcommand generates or updates the `.charter/` directory. This is incremental — only files that changed since the last capture are re-parsed.

```
$ charter

Charter @ a3f8c2d → b7e1d4f | 3 modified, 1 added, 0 removed

  modified: src/ecs/query.rs (+2 symbols, signature changed: fn execute)
  modified: src/render/pipeline.rs (fields changed on RenderState)
  added: src/render/postprocess.rs (14 symbols)

Captured @ b7e1d4f (316 files, 89,421 lines)
  parsed: 4, cached: 312, skipped: 0
```

## Output Files

The `.charter/` directory contains structured context optimized for LLM consumption:

### Core Files

| File | Contents | Example |
|------|----------|---------|
| `overview.md` | Workspace structure, module tree, entry points, features | Crate hierarchy, Python packages, bin/lib targets |
| `symbols.md` | Complete symbol index with full signatures | Structs, enums, functions, traits, classes |
| `types.md` | Trait definitions, impl map, derive map; Python Protocols, ABCs, class hierarchy | `Default -> [Cache, Config, State]` |
| `refs.md` | Cross-reference index | `PipelineResult` used in 12 files |
| `dependents.md` | Inverse dependency map | What breaks if you change a file |

### Analysis Files

| File | Contents | Example |
|------|----------|---------|
| `calls.md` | Call graph + reverse call graph | `node_text` has 47 callers |
| `clusters.md` | Semantic function groupings | 87 parse functions work together |
| `dataflow.md` | Type flow tracking | `Cache` produced by X, consumed by Y |
| `hotspots.md` | High-complexity functions | Ranked by cyclomatic complexity + churn |
| `errors.md` | Error propagation patterns | Where errors originate (Result/Option, raise/assert), how they flow |
| `safety.md` | Unsafe blocks, panic points, async patterns; Python dangerous calls (eval, exec, subprocess, pickle) | Safety-critical code locations |
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

## Performance

| Operation | 500 files | 5000 files |
|-----------|-----------|------------|
| Cold capture | < 3s | < 15s |
| Warm (0 changes) | < 100ms | < 100ms |
| Warm (10 changes) | < 500ms | < 500ms |
| MCP query | < 1ms | < 1ms |

## How It Works

**Phase 1 — Parallel Capture:**
- `ignore::WalkParallel` collects all `.rs` and `.py` files
- Cache check: match `(path, size, mtime)` or blake3 hash
- `tree-sitter` parses each file with thread-local parser pool (Rust or Python grammar)
- Extract: symbols, imports, complexity, call graph, error propagation
- `JoinSet` collects results in parallel

**Phase 2 — Reference Resolution:**
- Build PascalCase symbol table from Phase 1
- Match identifier locations against symbol table
- Write cross-references with no additional I/O

**Index:**
- Build in-memory index from pipeline results
- HashMaps for symbols, call graphs, reverse calls, implementations, references
- MCP server queries this index directly for sub-millisecond responses

## License

Dual-licensed under MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache 2.0 ([LICENSE-APACHE](LICENSE-APACHE)).
