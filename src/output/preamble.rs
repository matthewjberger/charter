#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use crate::detect::WorkspaceInfo;
use crate::pipeline::PipelineResult;

pub fn generate_preamble(
    result: &PipelineResult,
    churn_data: &HashMap<PathBuf, u32>,
    has_claude_md: bool,
) -> String {
    let mut lines = Vec::new();

    lines.push(format_stamp(result));
    lines.push(String::new());

    lines.push(format_workspace_summary(&result.workspace, result));

    if let Some(entry_points) = format_entry_points(&result.workspace) {
        lines.push(entry_points);
    }

    if let Some(key_traits) = format_key_traits(result) {
        lines.push(key_traits);
    }

    if let Some(most_depended) = format_most_depended(result) {
        lines.push(most_depended);
    }

    if let Some(high_churn) = format_high_churn(result, churn_data) {
        lines.push(high_churn);
    }

    if let Some(features) = format_features(&result.workspace) {
        lines.push(features);
    }

    lines.push(String::new());

    if has_claude_md {
        lines.push("Structural context follows. CLAUDE.md contains project intent and workflow instructions — consult both.".to_string());
    } else {
        lines.push("Structural context follows.".to_string());
    }

    if let Some(git) = &result.git_info {
        lines.push(format!(
            "Treat symbol locations, trait maps, and cross-references as ground truth. Verify against actual source for files changed since commit {}.",
            git.commit_short
        ));
    }

    lines.join("\n")
}

fn format_stamp(result: &PipelineResult) -> String {
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    match &result.git_info {
        Some(git) => format!(
            "[charter @ {} | {} | {} files | {} lines]",
            git.commit_short,
            timestamp,
            result.files.len(),
            result.total_lines
        ),
        None => format!(
            "[charter | {} | {} files | {} lines | no git]",
            timestamp,
            result.files.len(),
            result.total_lines
        ),
    }
}

fn format_workspace_summary(workspace: &WorkspaceInfo, result: &PipelineResult) -> String {
    if workspace.is_workspace {
        let lib_crates: Vec<_> = workspace
            .members
            .iter()
            .filter(|c| c.crate_type == crate::detect::CrateType::Lib)
            .collect();

        let primary = lib_crates
            .iter()
            .max_by_key(|c| {
                result
                    .files
                    .iter()
                    .filter(|f| f.relative_path.starts_with(&format!("crates/{}/", c.name)))
                    .map(|f| f.lines)
                    .sum::<usize>()
            })
            .map(|c| c.name.as_str());

        if let Some(primary_name) = primary {
            let primary_lines: usize = result
                .files
                .iter()
                .filter(|f| {
                    f.relative_path
                        .starts_with(&format!("crates/{}/", primary_name))
                        || f.relative_path.starts_with("src/")
                })
                .map(|f| f.lines)
                .sum();

            format!(
                "Rust workspace with {} crates. Primary: {} (lib, {}k lines).",
                workspace.members.len(),
                primary_name,
                primary_lines / 1000
            )
        } else {
            format!("Rust workspace with {} crates.", workspace.members.len())
        }
    } else if let Some(first) = workspace.members.first() {
        format!(
            "Rust {} crate: {} ({} files, {} lines).",
            first.crate_type,
            first.name,
            result.files.len(),
            result.total_lines
        )
    } else {
        format!(
            "Rust project ({} files, {} lines).",
            result.files.len(),
            result.total_lines
        )
    }
}

fn format_entry_points(workspace: &WorkspaceInfo) -> Option<String> {
    let mut bins = Vec::new();
    let mut examples = 0;
    let mut benches = 0;

    for crate_info in &workspace.members {
        for target in &crate_info.targets {
            match target.kind {
                crate::detect::TargetKind::Bin => {
                    if !bins.contains(&target.name) {
                        bins.push(target.name.clone());
                    }
                }
                crate::detect::TargetKind::Example => examples += 1,
                crate::detect::TargetKind::Bench => benches += 1,
                _ => {}
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

    Some(format!("Entry points: {}.", parts.join(", ")))
}

fn format_key_traits(result: &PipelineResult) -> Option<String> {
    let mut trait_impl_counts: HashMap<String, usize> = HashMap::new();

    for file in &result.files {
        for (trait_name, _type_name) in &file.parsed.symbols.impl_map {
            let simple_name = trait_name.split('<').next().unwrap_or(trait_name);
            *trait_impl_counts
                .entry(simple_name.to_string())
                .or_default() += 1;
        }
    }

    if trait_impl_counts.is_empty() {
        return None;
    }

    let mut sorted: Vec<_> = trait_impl_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let top_traits: Vec<String> = sorted
        .into_iter()
        .filter(|(name, count)| *count >= 3 && !is_std_trait(name))
        .take(5)
        .map(|(name, count)| format!("{} ({} impls)", name, count))
        .collect();

    if top_traits.is_empty() {
        return None;
    }

    Some(format!(
        "Key traits (most implemented): {}.",
        top_traits.join(", ")
    ))
}

fn is_std_trait(name: &str) -> bool {
    matches!(
        name,
        "Debug"
            | "Clone"
            | "Copy"
            | "Default"
            | "PartialEq"
            | "Eq"
            | "PartialOrd"
            | "Ord"
            | "Hash"
            | "Display"
            | "From"
            | "Into"
            | "AsRef"
            | "AsMut"
            | "Deref"
            | "DerefMut"
            | "Drop"
            | "Send"
            | "Sync"
            | "Serialize"
            | "Deserialize"
    )
}

fn format_most_depended(result: &PipelineResult) -> Option<String> {
    let mut dependent_counts: HashMap<String, usize> = HashMap::new();

    for file in &result.files {
        for import in &file.parsed.imports {
            if import.path.starts_with("crate::") {
                let parts: Vec<&str> = import.path.split("::").collect();
                if parts.len() >= 2 {
                    let module = parts[1..].join("::");
                    *dependent_counts.entry(module).or_default() += 1;
                }
            }
        }
    }

    if dependent_counts.is_empty() {
        return None;
    }

    let mut sorted: Vec<_> = dependent_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let top_files: Vec<String> = sorted
        .into_iter()
        .take(4)
        .map(|(path, count)| format!("{} ({})", path, count))
        .collect();

    if top_files.is_empty() {
        return None;
    }

    Some(format!("Most-depended-on: {}.", top_files.join(", ")))
}

fn format_high_churn(
    result: &PipelineResult,
    churn_data: &HashMap<PathBuf, u32>,
) -> Option<String> {
    if churn_data.is_empty() {
        return None;
    }

    let mut file_churn: Vec<(String, u32)> = Vec::new();

    for file in &result.files {
        if let Some(count) = churn_data.get(&file.path) {
            if *count > 0 {
                let name = file
                    .relative_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&file.relative_path)
                    .to_string();
                file_churn.push((name, *count));
            }
        }
    }

    file_churn.sort_by(|a, b| b.1.cmp(&a.1));

    let high_churn: Vec<String> = file_churn
        .into_iter()
        .take(5)
        .filter(|(_, count)| *count >= 5)
        .map(|(name, _)| name)
        .collect();

    if high_churn.is_empty() {
        return None;
    }

    Some(format!("High-churn: {}.", high_churn.join(", ")))
}

fn format_features(workspace: &WorkspaceInfo) -> Option<String> {
    let mut all_features: Vec<String> = Vec::new();

    for crate_info in &workspace.members {
        for feature in &crate_info.features {
            if !feature.name.starts_with("__")
                && feature.name != "default"
                && !all_features.contains(&feature.name)
            {
                all_features.push(feature.name.clone());
            }
        }
    }

    if all_features.is_empty() {
        return None;
    }

    all_features.sort();

    if all_features.len() <= 6 {
        Some(format!("Features: {}.", all_features.join(", ")))
    } else {
        let shown: Vec<_> = all_features.iter().take(5).cloned().collect();
        Some(format!(
            "Features: {}, +{} more.",
            shown.join(", "),
            all_features.len() - 5
        ))
    }
}
