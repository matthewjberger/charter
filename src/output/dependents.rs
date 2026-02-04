use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::pipeline::PipelineResult;

pub async fn write_dependents(
    atlas_dir: &Path,
    result: &PipelineResult,
    stamp: &str,
) -> Result<()> {
    let path = atlas_dir.join("dependents.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(64 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    let module_tree = build_module_tree(result);
    let dependents = build_dependent_map(result, &module_tree);

    if dependents.is_empty() || dependents.values().all(|v| v.is_empty()) {
        writeln!(buffer, "(no dependencies found)")?;
        file.write_all(&buffer).await?;
        return Ok(());
    }

    let mut sorted: Vec<_> = dependents.into_iter().collect();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));

    for (file_path, deps) in sorted {
        if deps.is_empty() {
            continue;
        }

        writeln!(buffer, "{} [{} dependents]", file_path, deps.len())?;

        let grouped = group_by_directory(&deps);

        for (dir, files) in grouped {
            if dir.is_empty() {
                writeln!(buffer, "  {}", files.join(", "))?;
            } else {
                writeln!(buffer, "  {}: {}", dir, files.join(", "))?;
            }
        }

        writeln!(buffer)?;
    }

    file.write_all(&buffer).await?;
    Ok(())
}

fn build_module_tree(result: &PipelineResult) -> HashMap<String, String> {
    let mut tree: HashMap<String, String> = HashMap::new();

    for file in &result.files {
        if !file.relative_path.ends_with(".rs") {
            continue;
        }

        let crate_prefix = extract_crate_prefix(&file.relative_path);
        let module_path = file_path_to_module_path(&file.relative_path);
        if !module_path.is_empty() {
            let full_key = format!("{}:{}", crate_prefix, module_path);
            tree.insert(full_key, file.relative_path.clone());
        }
    }

    tree
}

fn extract_crate_prefix(file_path: &str) -> String {
    if let Some(src_pos) = file_path.find("/src/") {
        file_path[..src_pos].to_string()
    } else if file_path.starts_with("src/") {
        String::new()
    } else {
        file_path.split('/').next().unwrap_or("").to_string()
    }
}

fn file_path_to_module_path(file_path: &str) -> String {
    let path = if let Some(src_pos) = file_path.find("/src/") {
        &file_path[src_pos + 5..]
    } else {
        file_path.strip_prefix("src/").unwrap_or(file_path)
    };

    let path = path.strip_suffix(".rs").unwrap_or(path);

    if path == "lib" || path == "main" {
        return "crate".to_string();
    }

    let parts: Vec<&str> = path.split('/').collect();
    let mut module_parts = Vec::new();

    for (index, part) in parts.iter().enumerate() {
        if *part == "mod" {
            continue;
        }
        if index == parts.len() - 1 && (*part == "lib" || *part == "main") {
            continue;
        }
        module_parts.push(*part);
    }

    if module_parts.is_empty() {
        "crate".to_string()
    } else {
        format!("crate::{}", module_parts.join("::"))
    }
}

fn build_dependent_map(
    result: &PipelineResult,
    module_tree: &HashMap<String, String>,
) -> HashMap<String, Vec<String>> {
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

    for file in &result.files {
        dependents.entry(file.relative_path.clone()).or_default();
    }

    for file in &result.files {
        let importing_file = &file.relative_path;

        for import in &file.parsed.imports {
            let resolved_files =
                resolve_import_to_files(&import.path, importing_file, module_tree, result);

            for target_file in resolved_files {
                if target_file != *importing_file {
                    dependents
                        .entry(target_file)
                        .or_default()
                        .push(importing_file.clone());
                }
            }
        }
    }

    for deps in dependents.values_mut() {
        deps.sort();
        deps.dedup();
    }

    dependents
}

fn resolve_import_to_files(
    import_path: &str,
    importing_file: &str,
    module_tree: &HashMap<String, String>,
    result: &PipelineResult,
) -> Vec<String> {
    let mut results = Vec::new();

    let crate_prefix = extract_crate_prefix(importing_file);
    let normalized = normalize_import_path(import_path, importing_file);

    for candidate in &normalized {
        let full_key = format!("{}:{}", crate_prefix, candidate);
        if let Some(file) = module_tree.get(&full_key) {
            results.push(file.clone());
            break;
        }
    }

    if results.is_empty() {
        for candidate in &normalized {
            let prefix = format!("{}:{}::", crate_prefix, candidate);
            for (module_path, file_path) in module_tree {
                if module_path.starts_with(&prefix) {
                    results.push(file_path.clone());
                    break;
                }
            }
            if !results.is_empty() {
                break;
            }
        }
    }

    if results.is_empty() {
        for candidate in &normalized {
            let file_candidates = module_path_to_possible_files(candidate);
            for fc in file_candidates {
                if result.files.iter().any(|f| f.relative_path == fc) {
                    results.push(fc);
                }
            }
        }
    }

    results.sort();
    results.dedup();
    results
}

fn normalize_import_path(import_path: &str, importing_file: &str) -> Vec<String> {
    let mut results = Vec::new();

    let cleaned = import_path
        .split('{')
        .next()
        .unwrap_or(import_path)
        .trim_end_matches("::");

    if cleaned.starts_with("crate::") {
        results.push(cleaned.to_string());

        let parts: Vec<&str> = cleaned.split("::").collect();
        for end in (2..=parts.len()).rev() {
            results.push(parts[..end].join("::"));
        }
    } else if cleaned.starts_with("super::") {
        let current_module = file_path_to_module_path(importing_file);
        let current_parts: Vec<&str> = current_module.split("::").collect();

        let super_count = cleaned.matches("super::").count();
        let remainder = cleaned.trim_start_matches("super::").replace("super::", "");

        if current_parts.len() > super_count {
            let base: Vec<&str> = current_parts[..current_parts.len() - super_count].to_vec();
            let resolved = if remainder.is_empty() {
                base.join("::")
            } else {
                format!("{}::{}", base.join("::"), remainder)
            };
            results.push(resolved.clone());

            let parts: Vec<&str> = resolved.split("::").collect();
            for end in (2..=parts.len()).rev() {
                results.push(parts[..end].join("::"));
            }
        }
    } else if cleaned.starts_with("self::") {
        let current_module = file_path_to_module_path(importing_file);
        let remainder = cleaned.strip_prefix("self::").unwrap_or("");
        let resolved = if remainder.is_empty() {
            current_module
        } else {
            format!("{}::{}", current_module, remainder)
        };
        results.push(resolved.clone());

        let parts: Vec<&str> = resolved.split("::").collect();
        for end in (2..=parts.len()).rev() {
            results.push(parts[..end].join("::"));
        }
    }

    results
}

fn module_path_to_possible_files(module_path: &str) -> Vec<String> {
    let mut results = Vec::new();

    let path = module_path.strip_prefix("crate::").unwrap_or(module_path);

    if path == "crate" || path.is_empty() {
        results.push("src/lib.rs".to_string());
        results.push("src/main.rs".to_string());
        return results;
    }

    let file_path = path.replace("::", "/");

    results.push(format!("src/{}.rs", file_path));

    results.push(format!("src/{}/mod.rs", file_path));

    let parts: Vec<&str> = path.split("::").collect();
    if parts.len() >= 2 {
        let parent = parts[..parts.len() - 1].join("/");
        results.push(format!("src/{}.rs", parent));
    }

    results
}

fn group_by_directory(files: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();

    for file in files {
        let dir = file.rsplit('/').nth(1).unwrap_or("").to_string();
        let filename = file.rsplit('/').next().unwrap_or(file).to_string();
        groups.entry(dir).or_default().push(filename);
    }

    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    for (_, files) in &mut sorted {
        files.sort();
    }

    sorted
}
