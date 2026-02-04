mod parse;
mod read;
mod walk;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheEntry, FileData};
use crate::detect::{WorkspaceInfo, detect_workspace};
use crate::git::{GitInfo, get_churn_data, get_git_info};
use crate::output;

const MAX_FILE_SIZE: u64 = 1024 * 1024;

fn should_skip_file(size: u64) -> bool {
    size > MAX_FILE_SIZE
}

fn is_binary_content(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    memchr::memchr(0, &data[..check_len]).is_some()
}

fn count_lines(data: &[u8]) -> usize {
    memchr::memchr_iter(b'\n', data).count()
        + if data.last() != Some(&b'\n') && !data.is_empty() {
            1
        } else {
            0
        }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn count_total_symbols(parsed: &parse::ParsedFile) -> usize {
    let top_level = parsed.symbols.symbols.len();
    let macros = parsed.symbols.macros.len();
    let impl_methods: usize = parsed
        .symbols
        .inherent_impls
        .iter()
        .map(|imp| imp.methods.len())
        .sum();
    top_level + macros + impl_methods
}

pub(crate) fn is_pascal_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };

    if !first.is_ascii_uppercase() {
        return false;
    }

    let has_lowercase = name.chars().any(|c| c.is_ascii_lowercase());
    let all_valid = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');

    has_lowercase && all_valid
}

pub use parse::{CapturedBody, ParsedFile};

const SEMAPHORE_PERMITS: usize = 256;

pub struct PipelineResult {
    pub files: Vec<FileResult>,
    pub workspace: WorkspaceInfo,
    pub git_info: Option<GitInfo>,
    pub total_lines: usize,
    pub skipped: Vec<SkippedFile>,
    pub diff_summary: Option<DiffSummary>,
}

#[derive(Debug, Default)]
pub struct DiffSummary {
    pub old_commit: Option<String>,
    pub new_commit: Option<String>,
    pub added: Vec<AddedFile>,
    pub removed: Vec<RemovedFile>,
    pub modified: Vec<ModifiedFile>,
}

#[derive(Debug)]
pub struct AddedFile {
    pub path: String,
    pub symbol_count: usize,
}

#[derive(Debug)]
pub struct RemovedFile {
    pub path: String,
    pub symbol_count: usize,
}

#[derive(Debug)]
pub struct ModifiedFile {
    pub path: String,
    pub symbols_added: usize,
    pub symbols_removed: usize,
    pub signature_changes: Vec<String>,
    pub field_changes: Vec<String>,
}

#[derive(Debug)]
pub struct FileResult {
    pub path: PathBuf,
    pub relative_path: String,
    pub hash: String,
    pub size: u64,
    pub lines: usize,
    pub parsed: ParsedFile,
    pub from_cache: bool,
}

#[derive(Debug)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: String,
}

pub async fn capture(root: &Path) -> Result<()> {
    let atlas_dir = root.join(".atlas");
    tokio::fs::create_dir_all(&atlas_dir).await?;

    let gitignore_path = atlas_dir.join(".gitignore");
    if !gitignore_path.exists() {
        tokio::fs::write(&gitignore_path, "*\n").await?;
    }

    let cache_path = atlas_dir.join("cache.bin");
    let meta_path = atlas_dir.join("meta.json");

    let (cache, walk_result, old_meta) = tokio::join!(
        Cache::load(&cache_path),
        walk::walk_directory(root),
        load_old_meta(&meta_path)
    );
    let cache = cache.unwrap_or_default();
    let walk_result = walk_result?;
    let old_commit = old_meta.and_then(|m| m.git_commit);

    if !cache.entries.is_empty() {
        if let Some(change_count) = quick_change_check_sync(root, &walk_result.files, &cache) {
            if change_count == 0 {
                let git_info = get_git_info(root).await.ok();
                println!();
                if let Some(git) = &git_info {
                    println!(
                        "Up to date @ {} ({} files)",
                        git.commit_short,
                        walk_result.files.len()
                    );
                } else {
                    println!("Up to date ({} files)", walk_result.files.len());
                }
                return Ok(());
            }
        }
    }

    let workspace = detect_workspace(root).await?;
    let (git_info, churn_data) = tokio::join!(get_git_info(root), get_churn_data(root));
    let git_info = git_info.ok();
    let churn_data = churn_data.unwrap_or_default();

    let mut result =
        run_phase1_with_walk(root, &workspace, &cache, git_info.as_ref(), walk_result).await?;

    result.diff_summary = Some(build_diff_summary(
        &result.files,
        &cache,
        old_commit,
        git_info.as_ref().map(|g| g.commit_short.clone()),
    ));

    let symbol_table = build_symbol_table(&result.files);
    let references = run_phase2(&result.files, &symbol_table);

    emit_outputs(root, &result, &references, &churn_data).await?;

    let new_cache = build_cache(&result.files);
    new_cache.save(&cache_path).await?;

    print_summary(&result);

    Ok(())
}

async fn load_old_meta(path: &Path) -> Option<Meta> {
    let content = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&content).ok()
}

fn quick_change_check_sync(root: &Path, files: &[PathBuf], cache: &Cache) -> Option<usize> {
    if files.len() != cache.entries.len() {
        return None;
    }

    let mut changed = 0;

    for path in files {
        let relative_path = path
            .strip_prefix(root)
            .map(normalize_path)
            .unwrap_or_else(|_| normalize_path(path));

        let cached = cache.get(&relative_path)?;

        let Ok(metadata) = std::fs::metadata(path) else {
            return None;
        };

        let size = metadata.len();
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if cached.mtime != mtime || cached.size != size {
            changed += 1;
        }
    }

    Some(changed)
}

async fn run_phase1_with_walk(
    root: &Path,
    workspace: &WorkspaceInfo,
    cache: &Cache,
    git_info: Option<&GitInfo>,
    walk_result: walk::WalkResult,
) -> Result<PipelineResult> {
    let pb = ProgressBar::new(walk_result.files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .expect("valid template")
            .progress_chars("#>-"),
    );

    let semaphore = Arc::new(Semaphore::new(SEMAPHORE_PERMITS));
    let files = Arc::new(Mutex::new(Vec::new()));
    let skipped = Arc::new(Mutex::new(Vec::new()));

    let mut join_set = JoinSet::new();

    for file_path in walk_result.files {
        let semaphore = Arc::clone(&semaphore);
        let files = Arc::clone(&files);
        let skipped = Arc::clone(&skipped);
        let cache = cache.clone();
        let root = root.to_path_buf();
        let pb = pb.clone();

        join_set.spawn(async move {
            let _permit = semaphore.acquire().await;

            match process_file(&file_path, &root, &cache).await {
                Ok(Some(result)) => {
                    files.lock().await.push(result);
                }
                Ok(None) => {}
                Err(e) => {
                    skipped.lock().await.push(SkippedFile {
                        path: file_path,
                        reason: e.to_string(),
                    });
                }
            }

            pb.inc(1);
        });
    }

    while join_set.join_next().await.is_some() {}

    pb.finish_with_message("Phase 1 complete");

    let mut files = Arc::try_unwrap(files)
        .expect("all tasks completed")
        .into_inner();

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let total_lines: usize = files.iter().map(|f| f.lines).sum();
    let skipped = Arc::try_unwrap(skipped)
        .expect("all tasks completed")
        .into_inner();

    Ok(PipelineResult {
        files,
        workspace: workspace.clone(),
        git_info: git_info.cloned(),
        total_lines,
        skipped,
        diff_summary: None,
    })
}

async fn process_file(path: &Path, root: &Path, cache: &Cache) -> Result<Option<FileResult>> {
    let metadata = tokio::fs::metadata(path).await?;
    let size = metadata.len();

    if should_skip_file(size) {
        return Ok(None);
    }

    let relative_path = path
        .strip_prefix(root)
        .map(normalize_path)
        .unwrap_or_else(|_| normalize_path(path));

    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if let Some(cached) = cache.get(&relative_path) {
        if cached.mtime == mtime && cached.size == size {
            return Ok(Some(FileResult {
                path: path.to_path_buf(),
                relative_path,
                hash: cached.hash.clone(),
                size,
                lines: cached.lines,
                parsed: cached.data.parsed.clone(),
                from_cache: true,
            }));
        }
    }

    let content = read::read_file(path, size).await?;

    if is_binary_content(&content) {
        return Ok(None);
    }

    let hash = blake3::hash(&content).to_hex().to_string();

    if let Some(cached) = cache.get(&relative_path) {
        if cached.hash == hash {
            return Ok(Some(FileResult {
                path: path.to_path_buf(),
                relative_path,
                hash,
                size,
                lines: cached.lines,
                parsed: cached.data.parsed.clone(),
                from_cache: true,
            }));
        }
    }

    let lines = count_lines(&content);

    let content_string = String::from_utf8_lossy(&content).into_owned();
    let relative_path_clone = relative_path.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        parse::parse_rust_file(&content_string, &relative_path_clone)
    })
    .await??;

    Ok(Some(FileResult {
        path: path.to_path_buf(),
        relative_path,
        hash,
        size,
        lines,
        parsed,
        from_cache: false,
    }))
}

fn build_symbol_table(files: &[FileResult]) -> HashMap<String, (String, usize)> {
    let mut table = HashMap::new();

    for file in files {
        for symbol in &file.parsed.symbols.symbols {
            if is_pascal_case(&symbol.name) {
                table.insert(
                    symbol.name.clone(),
                    (file.relative_path.clone(), symbol.line),
                );
            }
        }
    }

    table
}

fn run_phase2(
    files: &[FileResult],
    symbol_table: &HashMap<String, (String, usize)>,
) -> HashMap<String, Vec<(String, usize)>> {
    let mut references: HashMap<String, Vec<(String, usize)>> = HashMap::new();

    for file in files {
        for (name, line) in &file.parsed.identifier_locations {
            if !is_pascal_case(name) {
                continue;
            }

            if let Some((def_file, def_line)) = symbol_table.get(name) {
                if def_file == &file.relative_path && *def_line == *line {
                    continue;
                }

                if def_file == &file.relative_path {
                    continue;
                }

                references
                    .entry(name.clone())
                    .or_default()
                    .push((file.relative_path.clone(), *line));
            }
        }
    }

    for refs in references.values_mut() {
        refs.sort();
        refs.dedup();
    }

    references
}

async fn emit_outputs(
    root: &Path,
    result: &PipelineResult,
    references: &HashMap<String, Vec<(String, usize)>>,
    churn_data: &HashMap<PathBuf, u32>,
) -> Result<()> {
    let atlas_dir = root.join(".atlas");

    let stamp = format_stamp(result);

    output::overview::write_overview(&atlas_dir, result, &stamp).await?;
    output::symbols::write_symbols(&atlas_dir, result, churn_data, &stamp).await?;
    output::type_map::write_types(&atlas_dir, result, &stamp).await?;
    output::refs::write_refs(&atlas_dir, references, &stamp).await?;
    output::dependents::write_dependents(&atlas_dir, result, &stamp).await?;
    output::manifest::write_manifest(&atlas_dir, result, churn_data, &stamp).await?;
    output::hotspots::write_hotspots(&atlas_dir, result, churn_data, &stamp).await?;
    output::calls::write_calls(&atlas_dir, result, &stamp).await?;
    output::errors::write_errors(&atlas_dir, result, &stamp).await?;
    output::snippets::write_snippets(&atlas_dir, result, &stamp).await?;
    output::safety::write_safety(&atlas_dir, result, &stamp).await?;
    output::clusters::write_clusters(&atlas_dir, result, &stamp).await?;
    output::dataflow::write_dataflow(&atlas_dir, result, &stamp).await?;

    if !result.skipped.is_empty() {
        output::skipped::write_skipped(&atlas_dir, &result.skipped, &stamp).await?;
    }

    write_meta(&atlas_dir, result).await?;
    write_format_md(&atlas_dir).await?;

    Ok(())
}

fn format_stamp(result: &PipelineResult) -> String {
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    match &result.git_info {
        Some(git) => format!(
            "[atlas @ {} | {} | {} files | {} lines]",
            git.commit_short,
            timestamp,
            result.files.len(),
            result.total_lines
        ),
        None => format!(
            "[atlas | {} | {} files | {} lines | no git]",
            timestamp,
            result.files.len(),
            result.total_lines
        ),
    }
}

fn build_cache(files: &[FileResult]) -> Cache {
    let mut cache = Cache::default();

    for file in files {
        let mtime = std::fs::metadata(&file.path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        cache.entries.insert(
            file.relative_path.clone(),
            CacheEntry {
                hash: file.hash.clone(),
                mtime,
                size: file.size,
                lines: file.lines,
                data: FileData {
                    parsed: file.parsed.clone(),
                },
            },
        );
    }

    cache
}

fn build_diff_summary(
    files: &[FileResult],
    old_cache: &Cache,
    old_commit: Option<String>,
    new_commit: Option<String>,
) -> DiffSummary {
    use crate::extract::symbols::SymbolKind;
    use std::collections::HashSet;

    let mut summary = DiffSummary {
        old_commit,
        new_commit,
        added: Vec::new(),
        removed: Vec::new(),
        modified: Vec::new(),
    };

    let new_paths: HashSet<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    let old_paths: HashSet<&str> = old_cache.entries.keys().map(|s| s.as_str()).collect();

    for path in old_paths.difference(&new_paths) {
        if let Some(cached) = old_cache.get(path) {
            summary.removed.push(RemovedFile {
                path: path.to_string(),
                symbol_count: count_total_symbols(&cached.data.parsed),
            });
        }
    }

    for file in files {
        let path = &file.relative_path;

        if !old_cache.entries.contains_key(path) {
            summary.added.push(AddedFile {
                path: path.clone(),
                symbol_count: count_total_symbols(&file.parsed),
            });
            continue;
        }

        if file.from_cache {
            continue;
        }

        let cached = match old_cache.get(path) {
            Some(c) => c,
            None => continue,
        };

        if cached.hash == file.hash {
            continue;
        }

        let old_symbols = &cached.data.parsed.symbols.symbols;
        let new_symbols = &file.parsed.symbols.symbols;

        let old_names: HashSet<&str> = old_symbols.iter().map(|s| s.name.as_str()).collect();
        let new_names: HashSet<&str> = new_symbols.iter().map(|s| s.name.as_str()).collect();

        let symbols_added = new_names.difference(&old_names).count();
        let symbols_removed = old_names.difference(&new_names).count();

        let mut signature_changes = Vec::new();
        let mut field_changes = Vec::new();

        for new_sym in new_symbols {
            if let Some(old_sym) = old_symbols.iter().find(|s| s.name == new_sym.name) {
                match (&old_sym.kind, &new_sym.kind) {
                    (
                        SymbolKind::Function {
                            signature: old_sig, ..
                        },
                        SymbolKind::Function {
                            signature: new_sig, ..
                        },
                    ) => {
                        if old_sig != new_sig {
                            signature_changes.push(format!("fn {}", new_sym.name));
                        }
                    }
                    (
                        SymbolKind::Struct { fields: old_fields },
                        SymbolKind::Struct { fields: new_fields },
                    ) => {
                        if old_fields.len() != new_fields.len() {
                            field_changes.push(new_sym.name.clone());
                        } else {
                            let fields_differ =
                                old_fields.iter().zip(new_fields.iter()).any(|(old, new)| {
                                    old.name != new.name || old.field_type != new.field_type
                                });
                            if fields_differ {
                                field_changes.push(new_sym.name.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        summary.modified.push(ModifiedFile {
            path: path.clone(),
            symbols_added,
            symbols_removed,
            signature_changes,
            field_changes,
        });
    }

    summary.added.sort_by(|a, b| a.path.cmp(&b.path));
    summary.removed.sort_by(|a, b| a.path.cmp(&b.path));
    summary.modified.sort_by(|a, b| a.path.cmp(&b.path));

    summary
}

fn print_summary(result: &PipelineResult) {
    let cached_count = result.files.iter().filter(|f| f.from_cache).count();
    let parsed_count = result.files.len() - cached_count;

    println!();

    if let Some(diff) = &result.diff_summary {
        let has_changes =
            !diff.added.is_empty() || !diff.removed.is_empty() || !diff.modified.is_empty();

        if has_changes {
            match (&diff.old_commit, &diff.new_commit) {
                (Some(old), Some(new)) if old != new => {
                    println!(
                        "Atlas @ {} → {} | {} modified, {} added, {} removed",
                        old,
                        new,
                        diff.modified.len(),
                        diff.added.len(),
                        diff.removed.len()
                    );
                }
                (None, Some(new)) => {
                    println!(
                        "Atlas @ {} | {} modified, {} added, {} removed",
                        new,
                        diff.modified.len(),
                        diff.added.len(),
                        diff.removed.len()
                    );
                }
                _ => {
                    println!(
                        "Atlas | {} modified, {} added, {} removed",
                        diff.modified.len(),
                        diff.added.len(),
                        diff.removed.len()
                    );
                }
            }
            println!();

            for modified in &diff.modified {
                let mut details = Vec::new();
                if modified.symbols_added > 0 {
                    details.push(format!("+{} symbols", modified.symbols_added));
                }
                if modified.symbols_removed > 0 {
                    details.push(format!("-{} symbols", modified.symbols_removed));
                }
                for sig in &modified.signature_changes {
                    details.push(format!("signature changed: {}", sig));
                }
                for field in &modified.field_changes {
                    details.push(format!("fields changed on {}", field));
                }

                let detail_str = if details.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", details.join(", "))
                };
                println!("  modified: {}{}", modified.path, detail_str);
            }

            for added in &diff.added {
                println!("  added: {} ({} symbols)", added.path, added.symbol_count);
            }

            for removed in &diff.removed {
                println!(
                    "  removed: {} ({} symbols)",
                    removed.path, removed.symbol_count
                );
            }

            println!();
        }
    }

    if let Some(git) = &result.git_info {
        println!(
            "Captured @ {} ({} files, {} lines)",
            git.commit_short,
            result.files.len(),
            result.total_lines
        );
    } else {
        println!(
            "Captured ({} files, {} lines)",
            result.files.len(),
            result.total_lines
        );
    }
    println!(
        "  parsed: {}, cached: {}, skipped: {}",
        parsed_count,
        cached_count,
        result.skipped.len()
    );
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    timestamp: String,
    git_commit: Option<String>,
    files: usize,
    lines: usize,
}

async fn write_meta(atlas_dir: &Path, result: &PipelineResult) -> Result<()> {
    let meta = Meta {
        timestamp: chrono::Utc::now().to_rfc3339(),
        git_commit: result.git_info.as_ref().map(|g| g.commit_short.clone()),
        files: result.files.len(),
        lines: result.total_lines,
    };

    let content = serde_json::to_string_pretty(&meta)?;
    tokio::fs::write(atlas_dir.join("meta.json"), content).await?;
    Ok(())
}

async fn write_format_md(atlas_dir: &Path) -> Result<()> {
    let content = r#"# .atlas/ Format Specification

This directory contains generated structural context for Rust codebases.

## Files

- `overview.md` — workspace structure, module tree with doc comments, entry points, feature/cfg
- `symbols.md` — complete symbol index with full signatures, struct fields, enum variants
- `types.md` — trait definitions, impl map, derive map
- `refs.md` — cross-reference index (PascalCase types only)
- `dependents.md` — inverse dependency map
- `manifest.md` — file manifest with roles, churn, test locations
- `hotspots.md` — high-complexity functions ranked by importance score
- `calls.md` — call graph with hot paths and function relationships
- `errors.md` — error propagation patterns, origins, and public API surface
- `snippets.md` — captured function bodies for high/medium importance functions
- `skipped.md` — files skipped during capture (if any)
- `cache.bin` — internal cache for incremental updates
- `meta.json` — capture metadata

## Commit Stamp

Every output file starts with a commit stamp:
```
[atlas @ a3f8c2d | 2025-01-31T14:23:07Z | 487 files | 74,891 lines]
```

Use `git diff <commit>..HEAD --name-only` to assess freshness.

## Complexity Scoring (hotspots.md)

Importance score = (cyclomatic * 2) + (lines / 10) + (call_sites * 3) + (churn * 2) + (public ? 10 : 0)
- High: >= 30 (critical paths requiring review)
- Medium: 15-29 (worth understanding)
- Low: < 15 (not shown)

## Known Limitations

1. **build.rs generated code**: Files generated in `OUT_DIR` are invisible to static analysis.
   Types defined in build scripts won't appear in the symbol index.

2. **Name-based references**: `refs.md` uses name matching, not semantic analysis.
   Identically-named local types may produce false positives.

3. **Macro-generated code**: Procedural macro expansions are not analyzed.
   Derive implementations are tracked, but their internals are opaque.

4. **Call graph resolution**: Method calls use name matching, not type inference.
   Receiver types are best-effort inferred.

## Tiers

- `quick` — `overview.md` only
- `default` — `overview.md` + `symbols.md` + `types.md`
- `full` — all files
"#;

    tokio::fs::write(atlas_dir.join("FORMAT.md"), content).await?;
    Ok(())
}
