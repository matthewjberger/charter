# atlas

**Structural context for LLMs, in seconds.**

atlas generates a `.atlas/` directory containing token-dense structural context for Rust codebases. When you're working with an LLM that's lost track of your codebase (after context compaction, or in a new session), `atlas read` dumps everything it needs to re-orient: symbol locations, struct fields, trait implementations, cross-references, and dependency graphs.

## Installation

```bash
cargo install --path .
```

## Quick Start

```bash
# In your Rust project root:
atlas              # Generate .atlas/ directory
atlas read         # Dump context to stdout (pipe to LLM or copy/paste)
```

That's it. Run `atlas` once to capture, then `atlas read` whenever you need to reload context.

## Commands

### `atlas`

Generates or updates the `.atlas/` directory. Incremental — only re-parses files that changed.

```
$ atlas

Atlas @ a3f8c2d → b7e1d4f | 3 modified, 1 added, 0 removed

  modified: src/ecs/query.rs (+2 symbols, signature changed: fn execute)
  modified: src/render/pipeline.rs (fields changed on RenderState)
  added: src/render/postprocess.rs (14 symbols)

Captured @ b7e1d4f (316 files, 89,421 lines)
  parsed: 4, cached: 312, skipped: 0
```

### `atlas read [tier]`

Dumps structural context to stdout. Three tiers control how much context:

| Tier | Files | Size | Use when |
|------|-------|------|----------|
| `quick` | overview.md | ~6k tokens | Just need orientation |
| `default` | overview + symbols + types + dependents | ~43k tokens | Normal usage |
| `full` | Everything | ~62k tokens | Deep refactoring, cross-cutting changes |

```bash
atlas read          # default tier
atlas read quick    # minimal
atlas read full     # everything
```

**Staleness detection:** If files have changed since capture, `atlas read` warns you:

```
⚠ 3 files changed since capture (a3f8c2d → b7e1d4f):
  M src/ecs/world.rs
  M src/render/pipeline.rs
  A src/render/postprocess.rs

Structural context below may be inaccurate for these files. Read them directly for current state.
```

The output includes a project-specific preamble:

```
[atlas @ a3f8c2d | 2025-01-31T14:23:07Z | 316 files | 89,421 lines]

Rust workspace with 4 crates. Primary: my-engine (lib).
Entry points: my-app (bin), 12 examples, 3 benches

Top traits by impl count:
  Component (34 impls), System (12 impls), State (6 impls)

Most-depended-on files:
  src/lib.rs (56), src/ecs/world.rs (47), src/math/vec3.rs (38)

High-churn files:
  main.rs, pipeline.rs, widgets.rs
```

### `atlas status`

Quick summary without dumping full context:

```
$ atlas status
atlas status
  files: 316
  lines: 89,421
  captured: 2025-01-31T14:23:07Z
  commit: a3f8c2d
```

### `atlas inject`

Adds recovery instructions to your `CLAUDE.md` file. After LLM context compaction, the LLM will see these instructions and know to run `atlas read` to reload structural context.

```bash
atlas inject   # Appends to CLAUDE.md (or creates it)
```

## Output Files

The `.atlas/` directory contains:

| File | Contents |
|------|----------|
| `overview.md` | Workspace structure, module tree, entry points, features |
| `symbols.md` | Complete symbol index with signatures, struct fields, enum variants |
| `types.md` | Trait definitions, impl map (trait → types), derive map |
| `refs.md` | Cross-reference index (which files use which types) |
| `dependents.md` | Inverse dependency map (what breaks if you change a file) |
| `manifest.md` | File manifest with roles, churn scores, test locations |
| `cache.bin` | Internal cache for incremental updates |
| `meta.json` | Capture metadata |

The `.atlas/` directory is auto-gitignored (it creates its own `.gitignore`).

## Workflow

**Initial setup:**
```bash
cd my-rust-project
atlas           # Generate initial capture
atlas inject    # Add recovery instructions to CLAUDE.md
```

**During development:**
```bash
atlas           # Re-run after significant changes
```

**After LLM context compaction:**
```bash
atlas read      # Reload structural context into the conversation
```

## License

Dual-licensed under MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache 2.0 ([LICENSE-APACHE](LICENSE-APACHE)).
