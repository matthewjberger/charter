pub mod calls;
pub mod clusters;
pub mod dataflow;
pub mod dependents;
pub mod errors;
pub mod hotspots;
pub mod manifest;
pub mod overview;
pub mod preamble;
pub mod refs;
pub mod safety;
pub mod skipped;
pub mod snippets;
pub mod symbols;
pub mod type_map;

use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use tokio::fs;

use crate::cli::Tier;
use crate::git::get_git_info;

pub struct DiffContext {
    pub since_ref: String,
    pub changed_files: HashSet<String>,
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
}

impl DiffContext {
    pub fn get_marker(&self, path: &str) -> &'static str {
        let normalized = path.replace('\\', "/");
        if self
            .added
            .iter()
            .any(|p| normalized.ends_with(p) || p.ends_with(&normalized))
        {
            "[+] "
        } else if self
            .modified
            .iter()
            .any(|p| normalized.ends_with(p) || p.ends_with(&normalized))
        {
            "[~] "
        } else if self
            .deleted
            .iter()
            .any(|p| normalized.ends_with(p) || p.ends_with(&normalized))
        {
            "[-] "
        } else {
            ""
        }
    }

    pub fn is_changed(&self, path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        self.changed_files
            .iter()
            .any(|p| normalized.ends_with(p) || p.ends_with(&normalized))
    }
}

pub(crate) fn format_qualifiers(is_async: bool, is_unsafe: bool, is_const: bool) -> String {
    let mut parts = Vec::new();
    if is_const {
        parts.push("const");
    }
    if is_async {
        parts.push("async");
    }
    if is_unsafe {
        parts.push("unsafe");
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(" ") + " "
    }
}

pub(crate) fn file_role(path: &Path) -> &'static str {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if file_name == "Cargo.toml" || file_name == "Cargo.lock" {
        return "[build]";
    }

    if file_name == "build.rs" {
        return "[build]";
    }

    if file_name.ends_with("_test.rs") || file_name.ends_with("_tests.rs") {
        return "[test]";
    }

    let path_str = path.to_string_lossy();
    if path_str.contains("/tests/") || path_str.contains("\\tests\\") {
        return "[test]";
    }

    if path_str.contains("/benches/") || path_str.contains("\\benches\\") {
        return "[bench]";
    }

    if path_str.contains("/examples/") || path_str.contains("\\examples\\") {
        return "[example]";
    }

    match extension {
        "rs" => "[source]",
        "md" => "[docs]",
        "toml" => "[config]",
        "json" => "[config]",
        "yaml" | "yml" => "[config]",
        _ => "[other]",
    }
}

pub(crate) fn churn_label(count: u32, high_threshold: u32, med_threshold: u32) -> &'static str {
    if count >= high_threshold {
        "[churn:high]"
    } else if count >= med_threshold {
        "[churn:med]"
    } else {
        "[stable]"
    }
}

pub async fn lookup(root: &Path, symbol: &str) -> Result<()> {
    let charter_dir = root.join(".charter");

    if !charter_dir.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    let symbols_content = fs::read_to_string(charter_dir.join("symbols.md"))
        .await
        .unwrap_or_default();
    let types_content = fs::read_to_string(charter_dir.join("types.md"))
        .await
        .unwrap_or_default();
    let refs_content = fs::read_to_string(charter_dir.join("refs.md"))
        .await
        .unwrap_or_default();
    let dependents_content = fs::read_to_string(charter_dir.join("dependents.md"))
        .await
        .unwrap_or_default();

    let mut results = LookupResult::default();

    find_symbol_definition(&symbols_content, symbol, &mut results);
    find_trait_definition(&types_content, symbol, &mut results);
    find_type_info(&types_content, symbol, &mut results);
    find_references(&refs_content, symbol, &mut results);

    if !results.found && (results.ref_count > 0 || !results.derives.is_empty()) {
        results.found = true;
        results.name = symbol.to_string();
        if !results.derives.is_empty() || !results.implements.is_empty() {
            results.kind = "type".to_string();
        } else {
            results.kind = "external".to_string();
        }
    }

    let defined_at = results.defined_at.clone();
    find_dependents(&dependents_content, &defined_at, &mut results);

    if results.found {
        print_lookup_result(&results);
    } else {
        let suggestions = find_similar_symbols(&symbols_content, symbol);
        if suggestions.is_empty() {
            println!("⚠ Symbol '{}' not found. No similar symbols found.", symbol);
        } else {
            println!("⚠ Symbol '{}' not found. Did you mean:", symbol);
            for (name, kind, location) in suggestions.iter().take(5) {
                println!("  {} [{}] — {}", name, kind, location);
            }
        }
    }

    Ok(())
}

#[derive(Default)]
struct LookupResult {
    found: bool,
    name: String,
    kind: String,
    defined_at: String,
    definition_lines: Vec<String>,
    impl_methods: Vec<String>,
    derives: Vec<String>,
    implements: Vec<String>,
    implementors: Vec<String>,
    ref_count: usize,
    ref_locations: Vec<String>,
    ref_total_files: usize,
    dependent_count: usize,
}

fn find_symbol_definition(content: &str, symbol: &str, results: &mut LookupResult) {
    let mut current_file = String::new();
    let mut in_target_symbol = false;
    let mut brace_depth = 0;
    let mut capture_until_next_symbol = false;

    for line in content.lines() {
        if line.starts_with('[') || line.is_empty() && !in_target_symbol {
            continue;
        }

        let is_file_header = !line.starts_with(' ')
            && !line.is_empty()
            && (line.contains(".rs [") || line.contains(".rs:"));

        if is_file_header {
            current_file = line.split_whitespace().next().unwrap_or("").to_string();
            in_target_symbol = false;
            capture_until_next_symbol = false;
            continue;
        }

        if line.starts_with("  ") && !line.starts_with("    ") {
            let trimmed = line.trim();

            let is_symbol_def = is_symbol_definition_line(trimmed, symbol);

            if is_symbol_def {
                results.found = true;
                results.name = symbol.to_string();
                results.defined_at = current_file.clone();

                if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
                    results.kind = "struct".to_string();
                } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
                    results.kind = "enum".to_string();
                } else if trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ") {
                    results.kind = "trait".to_string();
                } else if trimmed.contains("fn ") {
                    results.kind = "fn".to_string();
                } else if trimmed.starts_with("pub const ") || trimmed.starts_with("const ") {
                    results.kind = "const".to_string();
                } else if trimmed.starts_with("pub type ") || trimmed.starts_with("type ") {
                    results.kind = "type".to_string();
                } else {
                    results.kind = "symbol".to_string();
                }

                results.definition_lines.push(trimmed.to_string());
                in_target_symbol = true;
                brace_depth =
                    trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;
                capture_until_next_symbol = brace_depth > 0;
                continue;
            }

            if in_target_symbol && !capture_until_next_symbol {
                in_target_symbol = false;
            }
        }

        if in_target_symbol && line.starts_with("    ") {
            let trimmed = line.trim();

            if trimmed.starts_with("impl") && trimmed.contains(':') {
                continue;
            }

            if capture_until_next_symbol {
                results.definition_lines.push(format!("  {}", trimmed));
                brace_depth += trimmed.matches('{').count() as i32;
                brace_depth -= trimmed.matches('}').count() as i32;
                if brace_depth <= 0 {
                    capture_until_next_symbol = false;
                }
            }
        }

        if in_target_symbol && line.starts_with("      ") {
            let trimmed = line.trim();
            if trimmed.contains("fn ") {
                results.impl_methods.push(trimmed.to_string());
            }
        }
    }
}

fn is_symbol_definition_line(line: &str, symbol: &str) -> bool {
    let patterns = [
        format!("pub struct {} ", symbol),
        format!("pub struct {}<", symbol),
        format!("pub struct {} {{", symbol),
        format!("struct {} ", symbol),
        format!("struct {}<", symbol),
        format!("struct {} {{", symbol),
        format!("pub enum {} ", symbol),
        format!("pub enum {}<", symbol),
        format!("pub enum {} {{", symbol),
        format!("enum {} ", symbol),
        format!("enum {}<", symbol),
        format!("enum {} {{", symbol),
        format!("pub trait {} ", symbol),
        format!("pub trait {}<", symbol),
        format!("pub trait {}:", symbol),
        format!("trait {} ", symbol),
        format!("trait {}<", symbol),
        format!("trait {}:", symbol),
        format!("pub fn {}(", symbol),
        format!("pub fn {}<", symbol),
        format!("fn {}(", symbol),
        format!("fn {}<", symbol),
        format!("pub async fn {}(", symbol),
        format!("async fn {}(", symbol),
        format!("pub const {}", symbol),
        format!("const {}", symbol),
        format!("pub type {} ", symbol),
        format!("pub type {}<", symbol),
        format!("type {} ", symbol),
        format!("type {}<", symbol),
    ];

    for pattern in &patterns {
        if line.contains(pattern.as_str()) {
            return true;
        }
    }

    false
}

fn find_trait_definition(content: &str, symbol: &str, results: &mut LookupResult) {
    if results.found {
        return;
    }

    for line in content.lines() {
        if line.starts_with("  trait ") {
            let trimmed = line.trim();

            let trait_match = trimmed.starts_with(&format!("trait {}<", symbol))
                || trimmed.starts_with(&format!("trait {}:", symbol))
                || trimmed.starts_with(&format!("trait {} ", symbol));

            if trait_match {
                results.found = true;
                results.name = symbol.to_string();
                results.kind = "trait".to_string();
                results.definition_lines.push(trimmed.to_string());
                return;
            }
        }
    }
}

fn find_type_info(content: &str, symbol: &str, results: &mut LookupResult) {
    let mut in_impls = false;
    let mut in_derived = false;

    for line in content.lines() {
        if line == "Impls:" {
            in_impls = true;
            in_derived = false;
            continue;
        }

        if line == "Derived:" {
            in_impls = false;
            in_derived = true;
            continue;
        }

        if in_impls && line.starts_with("  ") {
            let trimmed = line.trim();
            if let Some((trait_name, types_part)) = trimmed.split_once(" -> ") {
                let types_str = types_part.trim_start_matches('[').trim_end_matches(']');
                let types: Vec<&str> = types_str.split(", ").collect();

                if types.contains(&symbol) {
                    results.implements.push(trait_name.to_string());
                }

                if trait_name.contains(symbol) || trait_name.starts_with(&format!("{}<", symbol)) {
                    for t in types {
                        if !results.implementors.contains(&t.to_string()) {
                            results.implementors.push(t.to_string());
                        }
                    }
                }
            }
        }

        if in_derived && line.starts_with("  ") {
            let trimmed = line.trim();
            if let Some((type_name, derives)) = trimmed.split_once(" - ") {
                if type_name == symbol {
                    for d in derives.split(", ") {
                        results.derives.push(d.to_string());
                    }
                }
            }
        }
    }
}

fn find_references(content: &str, symbol: &str, results: &mut LookupResult) {
    for line in content.lines() {
        if line.starts_with('[') || line.is_empty() {
            continue;
        }

        if let Some((name_part, rest)) = line.split_once(" [") {
            if name_part == symbol {
                if let Some(count_end) = rest.find(']') {
                    if let Ok(count) = rest[..count_end].parse::<usize>() {
                        results.ref_count = count;
                    }
                }

                if let Some(locs_start) = line.find("— ") {
                    let locs_part = &line[locs_start + 4..];
                    let locs_clean = if let Some(bracket) = locs_part.find(" [+") {
                        &locs_part[..bracket]
                    } else {
                        locs_part
                    };

                    for loc in locs_clean.split(", ") {
                        results.ref_locations.push(loc.trim().to_string());
                    }

                    if let Some(more_start) = locs_part.find("[+") {
                        if let Some(files_start) = locs_part[more_start..].find(" in ") {
                            let after_in = &locs_part[more_start + files_start + 4..];
                            if let Some(end) = after_in.find(" files]") {
                                if let Ok(files) = after_in[..end].parse::<usize>() {
                                    results.ref_total_files = files + results.ref_locations.len();
                                }
                            }
                        }
                    } else {
                        results.ref_total_files = results.ref_locations.len();
                    }
                }
                break;
            }
        }
    }
}

fn find_dependents(content: &str, defined_at: &str, results: &mut LookupResult) {
    if defined_at.is_empty() {
        return;
    }

    for line in content.lines() {
        if line.starts_with(defined_at) && line.contains(" [") && line.contains(" dependents]") {
            if let Some(bracket_start) = line.find(" [") {
                let after_bracket = &line[bracket_start + 2..];
                if let Some(space) = after_bracket.find(' ') {
                    if let Ok(count) = after_bracket[..space].parse::<usize>() {
                        results.dependent_count = count;
                    }
                }
            }
            break;
        }
    }
}

fn find_similar_symbols(content: &str, symbol: &str) -> Vec<(String, String, String)> {
    let symbol_lower = symbol.to_lowercase();
    let mut suggestions: Vec<(String, String, String)> = Vec::new();

    let mut current_file = String::new();

    for line in content.lines() {
        if line.starts_with('[') {
            continue;
        }

        let is_file_header = !line.starts_with(' ')
            && !line.is_empty()
            && (line.contains(".rs [") || line.contains(".rs:"));

        if is_file_header {
            current_file = line.split_whitespace().next().unwrap_or("").to_string();
            continue;
        }

        if line.starts_with("  ") && !line.starts_with("    ") {
            let trimmed = line.trim();

            let (kind, name) = extract_symbol_name_and_kind(trimmed);
            if name.is_empty() {
                continue;
            }

            if name.to_lowercase().contains(&symbol_lower) {
                suggestions.push((name, kind, current_file.clone()));
            }
        }
    }

    suggestions.sort_by(|a, b| {
        let a_exact = a.0.to_lowercase() == symbol_lower;
        let b_exact = b.0.to_lowercase() == symbol_lower;
        b_exact
            .cmp(&a_exact)
            .then_with(|| a.0.len().cmp(&b.0.len()))
    });

    suggestions.dedup_by(|a, b| a.0 == b.0);
    suggestions
}

fn extract_symbol_name_and_kind(line: &str) -> (String, String) {
    let prefixes = [
        ("pub struct ", "struct"),
        ("struct ", "struct"),
        ("pub enum ", "enum"),
        ("enum ", "enum"),
        ("pub trait ", "trait"),
        ("trait ", "trait"),
        ("pub async fn ", "fn"),
        ("async fn ", "fn"),
        ("pub fn ", "fn"),
        ("fn ", "fn"),
        ("pub const ", "const"),
        ("const ", "const"),
        ("pub type ", "type"),
        ("type ", "type"),
        ("pub mod ", "mod"),
        ("mod ", "mod"),
    ];

    for (prefix, kind) in prefixes {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name_end = rest.find(['<', '(', ' ', '{', ':']).unwrap_or(rest.len());
            let name = rest[..name_end].trim().to_string();
            return (kind.to_string(), name);
        }
    }

    (String::new(), String::new())
}

fn print_lookup_result(results: &LookupResult) {
    if results.defined_at.is_empty() {
        if results.kind == "external" {
            println!("{} [external type, not defined in project]", results.name);
        } else {
            println!("{} [{}]", results.name, results.kind);
        }
    } else {
        println!(
            "{} [{}] defined at {}",
            results.name, results.kind, results.defined_at
        );
    }

    for line in &results.definition_lines {
        println!("  {}", line);
    }

    if !results.impl_methods.is_empty() {
        println!();
        println!("  impl {}:", results.name);
        for method in &results.impl_methods {
            println!("    {}", method);
        }
    }

    if !results.derives.is_empty() {
        println!();
        println!("  Derives: {}", results.derives.join(", "));
    }

    if !results.implements.is_empty() {
        println!("  Implements: {}", results.implements.join(", "));
    }

    if !results.implementors.is_empty() {
        println!();
        let shown: Vec<_> = results.implementors.iter().take(5).cloned().collect();
        if results.implementors.len() > 5 {
            println!(
                "  Implemented by: {} [+{} more]",
                shown.join(", "),
                results.implementors.len() - 5
            );
        } else {
            println!("  Implemented by: {}", shown.join(", "));
        }
    }

    if results.ref_count > 0 {
        println!();
        let shown: Vec<_> = results.ref_locations.iter().take(4).cloned().collect();
        if results.ref_total_files > 4 {
            println!(
                "  Referenced in {} files:\n    {} [+{} files]",
                results.ref_total_files,
                shown.join(", "),
                results.ref_total_files - 4
            );
        } else {
            println!(
                "  Referenced in {} files:\n    {}",
                results.ref_total_files,
                shown.join(", ")
            );
        }
    }

    if results.dependent_count > 0 {
        println!();
        println!(
            "  {} files depend on {}",
            results.dependent_count, results.defined_at
        );
    }
}

pub async fn peek(root: &Path, tier: Tier, focus: Option<&str>, since: Option<&str>) -> Result<()> {
    let charter_dir = root.join(".charter");

    if !charter_dir.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    if let Ok(meta) = load_meta(root).await {
        if let Some(ref commit) = meta.git_commit {
            if let Some(warning) = check_staleness(root, commit).await {
                println!("{}", warning);
            }
        }
    }

    let changed_files = if let Some(since_ref) = since {
        match crate::git::get_changed_files(root, since_ref).await {
            Ok(changes) => {
                let changed_set: std::collections::HashSet<String> =
                    changes.iter().map(|c| c.path.clone()).collect();
                Some(DiffContext {
                    since_ref: since_ref.to_string(),
                    changed_files: changed_set,
                    added: changes
                        .iter()
                        .filter(|c| matches!(c.kind, crate::git::FileChangeKind::Added))
                        .map(|c| c.path.clone())
                        .collect(),
                    modified: changes
                        .iter()
                        .filter(|c| matches!(c.kind, crate::git::FileChangeKind::Modified))
                        .map(|c| c.path.clone())
                        .collect(),
                    deleted: changes
                        .iter()
                        .filter(|c| matches!(c.kind, crate::git::FileChangeKind::Deleted))
                        .map(|c| c.path.clone())
                        .collect(),
                })
            }
            Err(e) => {
                eprintln!("⚠ Could not get changes since '{}': {}", since_ref, e);
                None
            }
        }
    } else {
        None
    };

    let focus_normalized = focus.map(normalize_focus_path);

    if let Some(ref focus_path) = focus_normalized {
        check_focus_matches(&charter_dir, focus_path).await;
    }

    let preamble =
        generate_peek_preamble_with_diff(root, focus_normalized.as_deref(), changed_files.as_ref())
            .await;
    println!("{}", preamble);
    println!();

    match tier {
        Tier::Quick => {
            print_filtered_overview_with_diff(
                &charter_dir.join("overview.md"),
                focus_normalized.as_deref(),
                changed_files.as_ref(),
            )
            .await?;
        }
        Tier::Default => {
            print_filtered_overview_with_diff(
                &charter_dir.join("overview.md"),
                focus_normalized.as_deref(),
                changed_files.as_ref(),
            )
            .await?;
            println!();
            print_filtered_symbols_with_diff(
                &charter_dir.join("symbols.md"),
                focus_normalized.as_deref(),
                changed_files.as_ref(),
            )
            .await?;
            println!();
            print_filtered_types(&charter_dir.join("types.md"), focus_normalized.as_deref())
                .await?;
            println!();
            print_filtered_dependents(
                &charter_dir.join("dependents.md"),
                focus_normalized.as_deref(),
            )
            .await?;
        }
        Tier::Full => {
            print_filtered_overview_with_diff(
                &charter_dir.join("overview.md"),
                focus_normalized.as_deref(),
                changed_files.as_ref(),
            )
            .await?;
            println!();
            print_filtered_symbols_with_diff(
                &charter_dir.join("symbols.md"),
                focus_normalized.as_deref(),
                changed_files.as_ref(),
            )
            .await?;
            println!();
            print_filtered_types(&charter_dir.join("types.md"), focus_normalized.as_deref())
                .await?;
            println!();
            print_filtered_dependents(
                &charter_dir.join("dependents.md"),
                focus_normalized.as_deref(),
            )
            .await?;
            println!();
            print_filtered_refs(&charter_dir.join("refs.md"), focus_normalized.as_deref()).await?;
            println!();
            print_filtered_manifest_with_diff(
                &charter_dir.join("manifest.md"),
                focus_normalized.as_deref(),
                changed_files.as_ref(),
            )
            .await?;
        }
    }

    Ok(())
}

fn normalize_focus_path(focus: &str) -> String {
    let normalized = focus.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    let normalized = normalized.trim_end_matches('/');
    normalized.to_string()
}

fn path_matches_focus(path: &str, focus: &str) -> bool {
    let normalized_path = path.replace('\\', "/");
    normalized_path.starts_with(focus)
        || normalized_path.starts_with(&format!("{}/", focus))
        || normalized_path == focus
}

async fn check_focus_matches(charter_dir: &Path, focus: &str) {
    let Ok(content) = fs::read_to_string(charter_dir.join("symbols.md")).await else {
        return;
    };

    let mut all_paths: Vec<String> = Vec::new();
    let mut matching_paths: Vec<String> = Vec::new();
    let mut containing_paths: Vec<String> = Vec::new();

    for line in content.lines() {
        if line.starts_with(' ') || line.is_empty() || line.starts_with('[') {
            continue;
        }

        let is_file_header = line.contains(".rs [") || line.contains(".rs:");
        let is_compressed_dir = line.contains("/ [") && line.contains(" files,");

        if is_file_header || is_compressed_dir {
            let file_path = line.split_whitespace().next().unwrap_or("");
            if !file_path.is_empty() {
                all_paths.push(file_path.to_string());

                if path_matches_focus(file_path, focus) {
                    matching_paths.push(file_path.to_string());
                } else if file_path.contains(focus) {
                    containing_paths.push(file_path.to_string());
                }
            }
        }
    }

    if !matching_paths.is_empty() {
        return;
    }

    let mut suggestions: Vec<String> = Vec::new();
    for path in &containing_paths {
        let focus_pos = path.find(focus).unwrap_or(0);
        let after_focus = focus_pos + focus.len();
        let suggestion = if let Some(next_slash) = path[after_focus..].find('/') {
            format!("{}/", &path[..after_focus + next_slash])
        } else {
            format!("{}/", &path[..after_focus])
        };
        if !suggestions.contains(&suggestion) {
            suggestions.push(suggestion);
        }
    }

    if !suggestions.is_empty() {
        eprintln!(
            "⚠ Focus path '{}' matched 0 files. Similar paths found:",
            focus
        );
        for suggestion in suggestions.iter().take(5) {
            eprintln!("  {}", suggestion);
        }
        if let Some(first) = suggestions.first() {
            eprintln!("Try: charter read --focus {}", first);
        }
        eprintln!();
    } else {
        eprintln!(
            "⚠ Focus path '{}' matched 0 files. No similar paths found.",
            focus
        );

        let mut top_level: std::collections::HashSet<String> = std::collections::HashSet::new();
        for path in &all_paths {
            let parts: Vec<&str> = path.split('/').collect();
            let dir_parts: Vec<&str> = parts
                .iter()
                .take_while(|p| !p.ends_with(".rs"))
                .copied()
                .collect();
            if dir_parts.len() >= 2 {
                let prefix = format!("{}/{}/", dir_parts[0], dir_parts[1]);
                top_level.insert(prefix);
            } else if dir_parts.len() == 1 {
                let prefix = format!("{}/", dir_parts[0]);
                top_level.insert(prefix);
            }
        }

        if !top_level.is_empty() {
            eprintln!("Available paths:");
            let mut sorted: Vec<_> = top_level.into_iter().collect();
            sorted.sort();
            for path in sorted.iter().take(10) {
                eprintln!("  {}", path);
            }
        }
        eprintln!();
    }
}

#[allow(dead_code)]
async fn generate_peek_preamble(root: &Path, focus: Option<&str>) -> String {
    let charter_dir = root.join(".charter");
    let git_info = get_git_info(root).await.ok();
    let meta = load_meta(root).await.ok();

    let total_lines = meta.as_ref().map(|m| m.lines).unwrap_or(0);
    let file_count = meta.as_ref().map(|m| m.files).unwrap_or(0);

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    let stamp = match (&git_info, focus) {
        (Some(git), Some(focus_path)) => format!(
            "[charter @ {} | {} | {} files | {} lines | focus: {}]",
            git.commit_short, timestamp, file_count, total_lines, focus_path
        ),
        (Some(git), None) => format!(
            "[charter @ {} | {} | {} files | {} lines]",
            git.commit_short, timestamp, file_count, total_lines
        ),
        (None, Some(focus_path)) => format!(
            "[charter | {} | {} files | {} lines | no git | focus: {}]",
            timestamp, file_count, total_lines, focus_path
        ),
        (None, None) => format!(
            "[charter | {} | {} files | {} lines | no git]",
            timestamp, file_count, total_lines
        ),
    };

    let commit_ref = git_info
        .as_ref()
        .map(|g| g.commit_short.clone())
        .unwrap_or_else(|| "HEAD".to_string());

    let mut lines = vec![stamp];

    if let Some(focus_path) = focus {
        lines.push(format!("Focused on: {}/", focus_path));
    }

    lines.push(String::new());

    if let Some(workspace_summary) = parse_workspace_summary(&charter_dir).await {
        lines.push(workspace_summary);
    }

    if let Some(entry_summary) = parse_entry_points(&charter_dir).await {
        lines.push(entry_summary);
    }

    lines.push(String::new());

    if let Some(top_traits) = parse_top_traits(&charter_dir).await {
        lines.push("Top traits by impl count:".to_string());
        lines.push(format!("  {}", top_traits));
        lines.push(String::new());
    }

    if let Some(top_dependents) = parse_top_dependents(&charter_dir).await {
        lines.push("Most-depended-on files:".to_string());
        lines.push(format!("  {}", top_dependents));
        lines.push(String::new());
    }

    if let Some(top_refs) = parse_top_refs(&charter_dir).await {
        lines.push("Top referenced types:".to_string());
        lines.push(format!("  {}", top_refs));
        lines.push(String::new());
    }

    if let Some(high_churn) = parse_high_churn(&charter_dir).await {
        lines.push("High-churn files:".to_string());
        lines.push(format!("  {}", high_churn));
        lines.push(String::new());
    }

    if let Some(features) = parse_features(&charter_dir).await {
        lines.push(format!("Features: {}", features));
        lines.push(String::new());
    }

    lines.push("This context survives compaction. If you've lost track of codebase structure, everything you need is below.".to_string());
    lines.push(format!(
        "Verify against source for files changed since {}: git diff {}..HEAD --name-only",
        commit_ref, commit_ref
    ));
    lines.push(String::new());
    lines.push("---".to_string());

    lines.join("\n")
}

async fn generate_peek_preamble_with_diff(
    root: &Path,
    focus: Option<&str>,
    diff: Option<&DiffContext>,
) -> String {
    let charter_dir = root.join(".charter");
    let git_info = get_git_info(root).await.ok();
    let meta = load_meta(root).await.ok();

    let total_lines = meta.as_ref().map(|m| m.lines).unwrap_or(0);
    let file_count = meta.as_ref().map(|m| m.files).unwrap_or(0);

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    let since_part = diff
        .as_ref()
        .map(|d| format!(" | since: {}", d.since_ref))
        .unwrap_or_default();

    let stamp = match (&git_info, focus) {
        (Some(git), Some(focus_path)) => format!(
            "[charter @ {} | {} | {} files | {} lines | focus: {}{}]",
            git.commit_short, timestamp, file_count, total_lines, focus_path, since_part
        ),
        (Some(git), None) => format!(
            "[charter @ {} | {} | {} files | {} lines{}]",
            git.commit_short, timestamp, file_count, total_lines, since_part
        ),
        (None, Some(focus_path)) => format!(
            "[charter | {} | {} files | {} lines | no git | focus: {}{}]",
            timestamp, file_count, total_lines, focus_path, since_part
        ),
        (None, None) => format!(
            "[charter | {} | {} files | {} lines | no git{}]",
            timestamp, file_count, total_lines, since_part
        ),
    };

    let commit_ref = git_info
        .as_ref()
        .map(|g| g.commit_short.clone())
        .unwrap_or_else(|| "HEAD".to_string());

    let mut lines = vec![stamp];

    if let Some(diff_ctx) = diff {
        let total_changes = diff_ctx.added.len() + diff_ctx.modified.len() + diff_ctx.deleted.len();
        if total_changes > 0 {
            lines.push(String::new());
            lines.push(format!(
                "Changes since {}: {} files (+{} ~{} -{})",
                diff_ctx.since_ref,
                total_changes,
                diff_ctx.added.len(),
                diff_ctx.modified.len(),
                diff_ctx.deleted.len()
            ));
            lines.push("Markers: [+] added, [~] modified, [-] deleted".to_string());
        }
    }

    if let Some(focus_path) = focus {
        lines.push(format!("Focused on: {}/", focus_path));
    }

    lines.push(String::new());

    if let Some(workspace_summary) = parse_workspace_summary(&charter_dir).await {
        lines.push(workspace_summary);
    }

    if let Some(entry_summary) = parse_entry_points(&charter_dir).await {
        lines.push(entry_summary);
    }

    lines.push(String::new());

    if let Some(top_traits) = parse_top_traits(&charter_dir).await {
        lines.push("Top traits by impl count:".to_string());
        lines.push(format!("  {}", top_traits));
        lines.push(String::new());
    }

    if let Some(top_dependents) = parse_top_dependents(&charter_dir).await {
        lines.push("Most-depended-on files:".to_string());
        lines.push(format!("  {}", top_dependents));
        lines.push(String::new());
    }

    if let Some(top_refs) = parse_top_refs(&charter_dir).await {
        lines.push("Top referenced types:".to_string());
        lines.push(format!("  {}", top_refs));
        lines.push(String::new());
    }

    if let Some(high_churn) = parse_high_churn(&charter_dir).await {
        lines.push("High-churn files:".to_string());
        lines.push(format!("  {}", high_churn));
        lines.push(String::new());
    }

    if let Some(features) = parse_features(&charter_dir).await {
        lines.push(format!("Features: {}", features));
        lines.push(String::new());
    }

    lines.push("This context survives compaction. If you've lost track of codebase structure, everything you need is below.".to_string());
    lines.push(format!(
        "Verify against source for files changed since {}: git diff {}..HEAD --name-only",
        commit_ref, commit_ref
    ));
    lines.push(String::new());
    lines.push("---".to_string());

    lines.join("\n")
}

async fn parse_workspace_summary(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("overview.md"))
        .await
        .ok()?;
    let mut lines_iter = content.lines();

    lines_iter.next();
    lines_iter.next();

    let first_content_line = lines_iter.next()?;

    if first_content_line.starts_with("Workspace:") {
        let mut crate_count = 0;
        let mut primary_crate = None;
        let mut in_workspace_section = false;

        for line in content.lines() {
            if line == "Workspace:" {
                in_workspace_section = true;
                continue;
            }
            if in_workspace_section {
                if !line.starts_with("  ") && !line.is_empty() {
                    break;
                }
                if line.starts_with("  ") && (line.contains("[lib]") || line.contains("[bin]")) {
                    crate_count += 1;
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(name) = parts.first() {
                        if primary_crate.is_none() || line.contains("[lib]") {
                            primary_crate = Some(name.to_string());
                        }
                    }
                }
            }
        }

        if let Some(ref name) = primary_crate {
            Some(format!(
                "Rust workspace with {} crates. Primary: {} (lib).",
                crate_count, name
            ))
        } else {
            Some(format!("Rust workspace with {} crates.", crate_count))
        }
    } else if first_content_line.starts_with("crate ") {
        let crate_name = first_content_line.strip_prefix("crate ")?.trim();
        Some(format!("Rust crate: {}.", crate_name))
    } else {
        None
    }
}

async fn parse_entry_points(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("overview.md"))
        .await
        .ok()?;

    let mut bins = Vec::new();
    let mut examples = 0;
    let mut benches = 0;

    let mut in_entry_points = false;
    for line in content.lines() {
        if line == "Entry points:" {
            in_entry_points = true;
            continue;
        }
        if in_entry_points {
            if !line.starts_with("  ") && !line.is_empty() {
                break;
            }
            if line.contains("[bin]") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(name) = parts.first() {
                    if !bins.contains(&name.to_string()) {
                        bins.push(name.to_string());
                    }
                }
            } else if line.contains("[example]") {
                examples += 1;
            } else if line.contains("[bench]") {
                benches += 1;
            }
        }
    }

    if bins.is_empty() && examples == 0 && benches == 0 {
        return None;
    }

    let mut parts = Vec::new();
    if !bins.is_empty() {
        if bins.len() <= 3 {
            parts.push(format!("{} (bin)", bins.join(", ")));
        } else {
            parts.push(format!("{} binaries", bins.len()));
        }
    }
    if examples > 0 {
        parts.push(format!("{} examples", examples));
    }
    if benches > 0 {
        parts.push(format!("{} benches", benches));
    }

    Some(format!("Entry points: {}", parts.join(", ")))
}

async fn parse_top_traits(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("types.md"))
        .await
        .ok()?;

    let mut trait_counts: Vec<(String, usize)> = Vec::new();

    let mut in_impls = false;
    for line in content.lines() {
        if line == "Impls:" {
            in_impls = true;
            continue;
        }
        if in_impls {
            if !line.starts_with("  ") && !line.is_empty() {
                break;
            }
            if line.contains(" -> [") {
                let parts: Vec<&str> = line.trim().splitn(2, " -> ").collect();
                if parts.len() == 2 {
                    let trait_name = parts[0].to_string();
                    if is_infrastructure_trait(&trait_name) {
                        continue;
                    }
                    let types_str = parts[1].trim_start_matches('[').trim_end_matches(']');
                    let impl_count = types_str.split(',').count();
                    trait_counts.push((trait_name, impl_count));
                }
            }
        }
    }

    if trait_counts.is_empty() {
        return None;
    }

    trait_counts.sort_by(|a, b| b.1.cmp(&a.1));
    trait_counts.truncate(5);

    let formatted: Vec<String> = trait_counts
        .iter()
        .map(|(name, count)| format!("{} ({} impls)", name, count))
        .collect();

    Some(formatted.join(", "))
}

async fn parse_top_dependents(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("dependents.md"))
        .await
        .ok()?;

    let mut dependents: Vec<(String, usize)> = Vec::new();

    for line in content.lines() {
        if line.contains(" [") && line.contains(" dependents]") {
            let parts: Vec<&str> = line.splitn(2, " [").collect();
            if parts.len() == 2 {
                let file_path = parts[0].to_string();
                if let Some(count_str) = parts[1].strip_suffix(" dependents]") {
                    if let Ok(count) = count_str.parse::<usize>() {
                        if count > 0 {
                            dependents.push((file_path, count));
                        }
                    }
                }
            }
        }
    }

    if dependents.is_empty() {
        return None;
    }

    dependents.sort_by(|a, b| b.1.cmp(&a.1));
    dependents.truncate(5);

    let formatted: Vec<String> = dependents
        .iter()
        .map(|(path, count)| format!("{} ({})", path, count))
        .collect();

    Some(formatted.join(", "))
}

async fn parse_top_refs(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("refs.md")).await.ok()?;

    let mut refs: Vec<(String, usize)> = Vec::new();

    for line in content.lines() {
        if line.starts_with('[') || line.is_empty() {
            continue;
        }

        if let Some((name_part, _rest)) = line.split_once(" [") {
            if is_infrastructure_type(name_part) {
                continue;
            }
            if let Some(count_end) = line.find("] —") {
                let count_start = line.find(" [").unwrap_or(0) + 2;
                if let Ok(count) = line[count_start..count_end].parse::<usize>() {
                    refs.push((name_part.to_string(), count));
                }
            }
        }
    }

    if refs.is_empty() {
        return None;
    }

    refs.sort_by(|a, b| b.1.cmp(&a.1));
    refs.truncate(5);

    let formatted: Vec<String> = refs
        .iter()
        .map(|(name, count)| format!("{} ({})", name, count))
        .collect();

    Some(formatted.join(", "))
}

fn is_infrastructure_type(name: &str) -> bool {
    const INFRA_TYPES: &[&str] = &[
        "Result",
        "Error",
        "Option",
        "Vec",
        "String",
        "Box",
        "Arc",
        "Rc",
        "HashMap",
        "HashSet",
        "BTreeMap",
        "BTreeSet",
        "PhantomData",
        "Pin",
        "Cow",
        "Cell",
        "RefCell",
        "Mutex",
        "RwLock",
        "MutexGuard",
        "Sender",
        "Receiver",
    ];
    INFRA_TYPES.contains(&name)
}

fn is_infrastructure_trait(name: &str) -> bool {
    const INFRA_TRAITS: &[&str] = &[
        "Default",
        "Clone",
        "Debug",
        "Copy",
        "PartialEq",
        "Eq",
        "PartialOrd",
        "Ord",
        "Hash",
        "Send",
        "Sync",
        "Display",
        "From",
        "Into",
        "AsRef",
        "AsMut",
        "Deref",
        "DerefMut",
        "Drop",
        "Iterator",
        "IntoIterator",
        "Serialize",
        "Deserialize",
        "Component",
    ];
    let base_name = name.split('<').next().unwrap_or(name);
    INFRA_TRAITS.contains(&base_name)
}

async fn parse_high_churn(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("manifest.md"))
        .await
        .ok()?;

    let mut high_churn: Vec<String> = Vec::new();

    for line in content.lines() {
        if line.contains("[churn:high]") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(path) = parts.first() {
                let filename = path.rsplit('/').next().unwrap_or(path);
                high_churn.push(filename.to_string());
            }
        }
    }

    if high_churn.is_empty() {
        return None;
    }

    high_churn.truncate(5);
    Some(high_churn.join(", "))
}

async fn parse_features(charter_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(charter_dir.join("overview.md"))
        .await
        .ok()?;

    let mut features: Vec<String> = Vec::new();

    let mut in_features = false;
    for line in content.lines() {
        if line == "Features:" {
            in_features = true;
            continue;
        }
        if in_features {
            if !line.starts_with("  ") && !line.is_empty() {
                break;
            }
            let line = line.trim();
            if !line.is_empty() {
                let feature_name = line.split_whitespace().next().unwrap_or(line);
                let file_count = if line.contains("gates:") {
                    let gates_part = line.split("gates:").nth(1).unwrap_or("");
                    gates_part.split(',').count()
                } else {
                    0
                };
                if file_count > 0 {
                    features.push(format!("{} ({} files)", feature_name, file_count));
                } else {
                    features.push(feature_name.to_string());
                }
            }
        }
    }

    if features.is_empty() {
        return None;
    }

    features.truncate(5);
    Some(features.join(", "))
}

pub async fn stats(root: &Path) -> Result<()> {
    let meta_path = root.join(".charter/meta.json");

    if !meta_path.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    let content = fs::read_to_string(&meta_path).await?;
    let meta: serde_json::Value = serde_json::from_str(&content)?;

    let files = meta.get("files").and_then(|v| v.as_u64()).unwrap_or(0);
    let lines = meta.get("lines").and_then(|v| v.as_u64()).unwrap_or(0);
    let timestamp = meta
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let commit = meta.get("git_commit").and_then(|v| v.as_str());

    println!("charter status");
    println!("  files: {}", files);
    println!("  lines: {}", lines);
    println!("  captured: {}", timestamp);
    if let Some(commit) = commit {
        println!("  commit: {}", commit);
    }

    Ok(())
}

#[derive(serde::Deserialize)]
struct Meta {
    files: usize,
    lines: usize,
    git_commit: Option<String>,
}

async fn load_meta(root: &Path) -> Result<Meta> {
    let content = fs::read_to_string(root.join(".charter/meta.json")).await?;
    let meta: Meta = serde_json::from_str(&content)?;
    Ok(meta)
}

async fn check_staleness(root: &Path, captured_commit: &str) -> Option<String> {
    let mut all_changes: Vec<String> = Vec::new();

    let committed_output = tokio::process::Command::new("git")
        .args([
            "diff",
            "--name-status",
            &format!("{}..HEAD", captured_commit),
        ])
        .current_dir(root)
        .output()
        .await
        .ok()?;

    if committed_output.status.success() {
        let diff_output = String::from_utf8_lossy(&committed_output.stdout);
        for line in diff_output.lines() {
            if !line.is_empty() {
                all_changes.push(line.to_string());
            }
        }
    }

    let uncommitted_output = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .await
        .ok()?;

    if uncommitted_output.status.success() {
        let status_output = String::from_utf8_lossy(&uncommitted_output.stdout);
        for line in status_output.lines() {
            if line.is_empty() {
                continue;
            }
            let status_char = line.chars().next().unwrap_or(' ');
            let path = line.get(3..).unwrap_or("").trim();
            if path.is_empty() {
                continue;
            }
            let status = match status_char {
                'M' | ' ' => "M",
                'A' | '?' => "A",
                'D' => "D",
                'R' => "R",
                _ => "M",
            };
            let entry = format!("{}\t{}", status, path);
            if !all_changes.contains(&entry) {
                all_changes.push(entry);
            }
        }
    }

    if all_changes.is_empty() {
        return None;
    }

    let head_output = tokio::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .await
        .ok()?;

    let head_short = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    let suffix = if head_short == captured_commit[..7.min(captured_commit.len())] {
        " + uncommitted".to_string()
    } else {
        format!(" → {}", head_short)
    };

    let mut warning = format!(
        "⚠ {} file{} changed since capture ({}{}):\n",
        all_changes.len(),
        if all_changes.len() == 1 { "" } else { "s" },
        &captured_commit[..7.min(captured_commit.len())],
        suffix
    );

    for line in all_changes.iter().take(20) {
        warning.push_str(&format!("  {}\n", line));
    }

    if all_changes.len() > 20 {
        warning.push_str(&format!("  ... and {} more\n", all_changes.len() - 20));
    }

    warning.push_str("\nStructural context below may be inaccurate for these files. Read them directly for current state.\n");

    Some(warning)
}

#[allow(dead_code)]
async fn print_filtered_overview(path: &Path, focus: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    let Some(focus) = focus else {
        print_content_without_stamp(&content);
        return Ok(());
    };

    let mut in_module_tree = false;
    let mut current_section_matches = false;
    let mut skip_empty = true;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line == "Module tree:" {
            in_module_tree = true;
            println!("{}", line);
            continue;
        }

        if in_module_tree {
            if !line.starts_with("  ") && !line.is_empty() {
                in_module_tree = false;
            } else if line.starts_with("  ") {
                let trimmed = line.trim();
                let module_path = trimmed.split_whitespace().next().unwrap_or("");
                let module_path_normalized = module_path.replace("::", "/");

                if path_matches_focus(&module_path_normalized, focus)
                    || focus.starts_with(&module_path_normalized)
                {
                    println!("{}", line);
                }
                continue;
            }
        }

        if line.ends_with(':') && !line.starts_with(' ') {
            current_section_matches = true;
            println!("{}", line);
            continue;
        }

        if current_section_matches {
            println!("{}", line);
        }
    }

    Ok(())
}

async fn print_filtered_overview_with_diff(
    path: &Path,
    focus: Option<&str>,
    diff: Option<&DiffContext>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    if focus.is_none() && diff.is_none() {
        print_content_without_stamp(&content);
        return Ok(());
    }

    let mut in_module_tree = false;
    let mut current_section_matches = false;
    let mut skip_empty = true;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line == "Module tree:" {
            in_module_tree = true;
            println!("{}", line);
            continue;
        }

        if in_module_tree {
            if !line.starts_with("  ") && !line.is_empty() {
                in_module_tree = false;
            } else if line.starts_with("  ") {
                let trimmed = line.trim();
                let module_path = trimmed.split_whitespace().next().unwrap_or("");
                let module_path_normalized = module_path.replace("::", "/");

                let focus_match = focus.is_none_or(|f| {
                    path_matches_focus(&module_path_normalized, f)
                        || f.starts_with(&module_path_normalized)
                });

                if focus_match {
                    let marker = diff.map_or("", |d| d.get_marker(&module_path_normalized));
                    if marker.is_empty() {
                        println!("{}", line);
                    } else {
                        println!("{}{}", marker, trimmed);
                    }
                }
                continue;
            }
        }

        if line.ends_with(':') && !line.starts_with(' ') {
            current_section_matches = true;
            println!("{}", line);
            continue;
        }

        if current_section_matches {
            println!("{}", line);
        }
    }

    Ok(())
}

#[allow(dead_code)]
async fn print_filtered_symbols(path: &Path, focus: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    let Some(focus) = focus else {
        print_content_without_stamp(&content);
        return Ok(());
    };

    let mut current_file_matches = false;
    let mut buffer: Vec<String> = Vec::new();
    let mut skip_empty = true;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        let is_file_header = !line.starts_with(' ')
            && !line.is_empty()
            && (line.contains(".rs [") || line.contains(".rs:"));

        if is_file_header {
            if current_file_matches && !buffer.is_empty() {
                for buffered_line in &buffer {
                    println!("{}", buffered_line);
                }
                println!();
            }
            buffer.clear();

            let file_path = line.split_whitespace().next().unwrap_or("");
            current_file_matches = path_matches_focus(file_path, focus);

            if current_file_matches {
                buffer.push(line.to_string());
            }
        } else if current_file_matches {
            buffer.push(line.to_string());
        }
    }

    if current_file_matches && !buffer.is_empty() {
        for buffered_line in &buffer {
            println!("{}", buffered_line);
        }
    }

    Ok(())
}

async fn print_filtered_symbols_with_diff(
    path: &Path,
    focus: Option<&str>,
    diff: Option<&DiffContext>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    if focus.is_none() && diff.is_none() {
        print_content_without_stamp(&content);
        return Ok(());
    }

    let mut current_file_matches = false;
    let mut current_file_path = String::new();
    let mut buffer: Vec<String> = Vec::new();
    let mut skip_empty = true;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        let is_file_header = !line.starts_with(' ')
            && !line.is_empty()
            && (line.contains(".rs [") || line.contains(".rs:"));

        if is_file_header {
            if current_file_matches && !buffer.is_empty() {
                let marker = diff.map_or("", |d| d.get_marker(&current_file_path));
                for (index, buffered_line) in buffer.iter().enumerate() {
                    if index == 0 && !marker.is_empty() {
                        println!("{}{}", marker, buffered_line);
                    } else {
                        println!("{}", buffered_line);
                    }
                }
                println!();
            }
            buffer.clear();

            let file_path = line.split_whitespace().next().unwrap_or("");
            current_file_path = file_path.to_string();

            let focus_match = focus.is_none_or(|f| path_matches_focus(file_path, f));
            let diff_match = diff.is_none_or(|d| d.is_changed(file_path));
            current_file_matches = focus_match && (diff.is_none() || diff_match);

            if current_file_matches {
                buffer.push(line.to_string());
            }
        } else if current_file_matches {
            buffer.push(line.to_string());
        }
    }

    if current_file_matches && !buffer.is_empty() {
        let marker = diff.map_or("", |d| d.get_marker(&current_file_path));
        for (index, buffered_line) in buffer.iter().enumerate() {
            if index == 0 && !marker.is_empty() {
                println!("{}{}", marker, buffered_line);
            } else {
                println!("{}", buffered_line);
            }
        }
    }

    Ok(())
}

async fn print_filtered_types(path: &Path, focus: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    let Some(focus) = focus else {
        print_content_without_stamp(&content);
        return Ok(());
    };

    let focused_types = collect_types_in_focus_from_symbols(path, focus).await;

    let mut in_impls = false;
    let mut in_derived = false;
    let mut skip_empty = true;
    let mut printed_impls_header = false;
    let mut printed_derived_header = false;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line == "Impls:" {
            in_impls = true;
            in_derived = false;
            continue;
        }

        if line == "Derived:" {
            in_impls = false;
            in_derived = true;
            continue;
        }

        if in_impls && line.starts_with("  ") {
            let trimmed = line.trim();
            if let Some(types_part) = trimmed.split(" -> ").nth(1) {
                let types_str = types_part.trim_start_matches('[').trim_end_matches(']');
                for type_name in types_str.split(", ") {
                    if focused_types.contains(type_name) {
                        if !printed_impls_header {
                            println!("Impls:");
                            printed_impls_header = true;
                        }
                        println!("{}", line);
                        break;
                    }
                }
            }
            continue;
        }

        if in_derived && line.starts_with("  ") {
            let trimmed = line.trim();
            if let Some(type_name) = trimmed.split(" - ").next() {
                if focused_types.contains(type_name) {
                    if !printed_derived_header {
                        if printed_impls_header {
                            println!();
                        }
                        println!("Derived:");
                        printed_derived_header = true;
                    }
                    println!("{}", line);
                }
            }
            continue;
        }

        if !line.starts_with("  ") && !line.is_empty() {
            in_impls = false;
            in_derived = false;
        }
    }

    Ok(())
}

async fn collect_types_in_focus_from_symbols(
    types_path: &Path,
    focus: &str,
) -> std::collections::HashSet<String> {
    let mut types = std::collections::HashSet::new();

    let symbols_path = types_path.with_file_name("symbols.md");
    let Ok(content) = fs::read_to_string(&symbols_path).await else {
        return types;
    };

    let mut current_file_matches = false;

    for line in content.lines() {
        let is_file_header = !line.starts_with(' ')
            && !line.is_empty()
            && (line.contains(".rs [") || line.contains(".rs:"));

        if is_file_header {
            let file_path = line.split_whitespace().next().unwrap_or("");
            current_file_matches = path_matches_focus(file_path, focus);
        } else if current_file_matches && line.starts_with("  ") {
            let trimmed = line.trim();
            if trimmed.starts_with("pub struct ")
                || trimmed.starts_with("pub enum ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("enum ")
            {
                let without_vis = trimmed
                    .trim_start_matches("pub ")
                    .trim_start_matches("struct ")
                    .trim_start_matches("enum ");
                let type_name = without_vis
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('<')
                    .trim_end_matches('{');
                if !type_name.is_empty() {
                    types.insert(type_name.to_string());
                }
            }
        }
    }

    types
}

async fn print_filtered_dependents(path: &Path, focus: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    let Some(focus) = focus else {
        print_content_without_stamp(&content);
        return Ok(());
    };

    let mut skip_empty = true;
    let mut header_printed = false;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line.starts_with("# ") {
            continue;
        }

        if line.contains(" [") && line.contains(" dependents]") {
            let file_path = line.split(" [").next().unwrap_or("");
            if path_matches_focus(file_path, focus) {
                if !header_printed {
                    println!("# Dependents");
                    println!();
                    header_printed = true;
                }
                println!("{}", line);
            }
        } else if line.starts_with("  ") && header_printed {
            println!("{}", line);
        }
    }

    Ok(())
}

async fn print_filtered_refs(path: &Path, focus: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    let Some(focus) = focus else {
        print_content_without_stamp(&content);
        return Ok(());
    };

    let mut skip_empty = true;
    let mut header_printed = false;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line.contains(" — ") {
            if let Some(refs_part) = line.split(" — ").nth(1) {
                let mut has_focus_ref = false;
                for ref_entry in refs_part.split(", ") {
                    let file_part = ref_entry.split(':').next().unwrap_or("");
                    if path_matches_focus(file_part, focus) {
                        has_focus_ref = true;
                        break;
                    }
                }
                if has_focus_ref {
                    if !header_printed {
                        println!("# Cross-References (focused)");
                        println!();
                        header_printed = true;
                    }
                    println!("{}", line);
                }
            }
        }
    }

    Ok(())
}

#[allow(dead_code)]
async fn print_filtered_manifest(path: &Path, focus: Option<&str>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    let Some(focus) = focus else {
        print_content_without_stamp(&content);
        return Ok(());
    };

    let mut skip_empty = true;
    let mut header_printed = false;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line.starts_with("# ") {
            continue;
        }

        let file_path = line.split_whitespace().next().unwrap_or("");
        if path_matches_focus(file_path, focus) {
            if !header_printed {
                println!("# File Manifest");
                println!();
                header_printed = true;
            }
            println!("{}", line);
        }
    }

    Ok(())
}

async fn print_filtered_manifest_with_diff(
    path: &Path,
    focus: Option<&str>,
    diff: Option<&DiffContext>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path).await?;

    if focus.is_none() && diff.is_none() {
        print_content_without_stamp(&content);
        return Ok(());
    }

    let mut skip_empty = true;
    let mut header_printed = false;

    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            continue;
        }

        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;

        if line.starts_with("# ") {
            continue;
        }

        let file_path = line.split_whitespace().next().unwrap_or("");
        let focus_match = focus.is_none_or(|f| path_matches_focus(file_path, f));
        let diff_match = diff.is_none_or(|d| d.is_changed(file_path));

        if focus_match && (diff.is_none() || diff_match) {
            if !header_printed {
                println!("# File Manifest");
                println!();
                header_printed = true;
            }
            let marker = diff.map_or("", |d| d.get_marker(file_path));
            if marker.is_empty() {
                println!("{}", line);
            } else {
                println!("{}{}", marker, line);
            }
        }
    }

    Ok(())
}

fn print_content_without_stamp(content: &str) {
    let mut skip_empty = true;
    for line in content.lines() {
        if line.starts_with("[charter @") || line.starts_with("[charter |") {
            skip_empty = true;
            continue;
        }
        if skip_empty && line.is_empty() {
            continue;
        }
        skip_empty = false;
        println!("{}", line);
    }
}
