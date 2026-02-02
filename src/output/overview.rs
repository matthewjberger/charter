use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::detect::{CrateInfo, CrateType, TargetKind};
use crate::pipeline::PipelineResult;

pub async fn write_overview(atlas_dir: &Path, result: &PipelineResult, stamp: &str) -> Result<()> {
    let path = atlas_dir.join("overview.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(64 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    if result.workspace.is_workspace {
        writeln!(buffer, "Workspace:")?;
        for crate_info in &result.workspace.members {
            write_crate_line(&mut buffer, crate_info)?;
        }
        writeln!(buffer)?;
    }

    for crate_info in &result.workspace.members {
        write_module_tree(&mut buffer, result, crate_info)?;
    }

    write_entry_points(&mut buffer, result)?;
    write_features(&mut buffer, result)?;

    file.write_all(&buffer).await?;
    Ok(())
}

fn write_crate_line(buffer: &mut Vec<u8>, crate_info: &CrateInfo) -> Result<()> {
    let crate_type = match crate_info.crate_type {
        CrateType::Lib => "[lib]",
        CrateType::Bin => "[bin]",
        CrateType::ProcMacro => "[proc-macro]",
    };

    let deps: String = crate_info
        .dependencies
        .iter()
        .filter(|d| crate_info.dependencies.contains(d))
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    if deps.is_empty() {
        writeln!(buffer, "  {} {}", crate_info.name, crate_type)?;
    } else {
        writeln!(buffer, "  {} {} -> {}", crate_info.name, crate_type, deps)?;
    }

    Ok(())
}

fn write_module_tree(
    buffer: &mut Vec<u8>,
    result: &PipelineResult,
    crate_info: &CrateInfo,
) -> Result<()> {
    writeln!(buffer, "crate {}", crate_info.name)?;

    let crate_path_str = crate_info.path.to_string_lossy().replace('\\', "/");

    let mut module_files: Vec<_> = result
        .files
        .iter()
        .filter(|f| {
            let file_path = f.relative_path.replace('\\', "/");
            file_path.starts_with(&format!(
                "{}/src/",
                crate_path_str.trim_start_matches(&format!(
                    "{}/",
                    result.workspace.root.to_string_lossy().replace('\\', "/")
                ))
            )) || file_path.starts_with("src/")
        })
        .collect();

    module_files.sort_by(|a, b| {
        let a_depth = a.relative_path.matches('/').count();
        let b_depth = b.relative_path.matches('/').count();
        a_depth
            .cmp(&b_depth)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });

    let mut seen_modules: HashMap<String, bool> = HashMap::new();

    for file in module_files {
        let path = &file.relative_path;

        if !path.ends_with(".rs") {
            continue;
        }

        let module_path = path_to_module_path(path);

        if seen_modules.contains_key(&module_path) {
            continue;
        }
        seen_modules.insert(module_path.clone(), true);

        let indent = "  ".repeat(path.matches('/').count().saturating_sub(1) + 1);
        let doc = file.parsed.module_doc.as_deref().unwrap_or("");

        if doc.is_empty() {
            writeln!(buffer, "{}{}", indent, path)?;
        } else {
            let doc_truncated = if doc.len() > 80 {
                format!("{}...", &doc[..77])
            } else {
                doc.to_string()
            };
            writeln!(buffer, "{}{} - \"{}\"", indent, path, doc_truncated)?;
        }
    }

    writeln!(buffer)?;
    Ok(())
}

fn path_to_module_path(path: &str) -> String {
    let path = path.strip_prefix("src/").unwrap_or(path);
    let path = path.strip_suffix(".rs").unwrap_or(path);
    path.replace('/', "::")
}

fn write_entry_points(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut has_entries = false;

    for crate_info in &result.workspace.members {
        for target in &crate_info.targets {
            if !has_entries {
                writeln!(buffer, "Entry points:")?;
                has_entries = true;
            }

            let kind = match target.kind {
                TargetKind::Lib => "[lib]",
                TargetKind::Bin => "[bin]",
                TargetKind::Example => "[example]",
                TargetKind::Bench => "[bench]",
            };

            let path_display = target
                .path
                .strip_prefix(&result.workspace.root)
                .unwrap_or(&target.path)
                .to_string_lossy()
                .replace('\\', "/");

            writeln!(buffer, "  {} {} -> {}", target.name, kind, path_display)?;
        }
    }

    if has_entries {
        writeln!(buffer)?;
    }

    Ok(())
}

fn write_features(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut has_features = false;

    for crate_info in &result.workspace.members {
        if !crate_info.features.is_empty() {
            if !has_features {
                writeln!(buffer, "Features:")?;
                has_features = true;
            }

            for feature in &crate_info.features {
                let gated_files: Vec<_> = result
                    .files
                    .iter()
                    .filter(|f| {
                        f.parsed
                            .cfgs
                            .iter()
                            .any(|cfg| cfg.condition.contains(&feature.name))
                    })
                    .map(|f| f.relative_path.clone())
                    .take(5)
                    .collect();

                if gated_files.is_empty() {
                    writeln!(buffer, "  {}", feature.name)?;
                } else {
                    writeln!(
                        buffer,
                        "  {} - gates: {}",
                        feature.name,
                        gated_files.join(", ")
                    )?;
                }
            }
        }
    }

    if has_features {
        writeln!(buffer)?;
    }

    Ok(())
}
