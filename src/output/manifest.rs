use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::pipeline::PipelineResult;

pub async fn write_manifest(
    atlas_dir: &Path,
    result: &PipelineResult,
    churn_data: &HashMap<PathBuf, u32>,
    stamp: &str,
) -> Result<()> {
    let path = atlas_dir.join("manifest.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(64 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    let (high_threshold, med_threshold) = calculate_churn_thresholds(churn_data);

    let test_modules = find_test_modules(result);

    for file_result in &result.files {
        let churn_count = churn_data.get(&file_result.path).copied().unwrap_or(0);
        let churn_label = super::churn_label(churn_count, high_threshold, med_threshold);
        let role = super::file_role(&file_result.path);

        write!(
            buffer,
            "{} [{} lines] {} {}",
            file_result.relative_path, file_result.lines, role, churn_label
        )?;

        let mut test_info = Vec::new();

        if file_result.parsed.has_test_module {
            let test_fns = &file_result.parsed.test_functions;
            if test_fns.is_empty() {
                test_info.push("inline #[cfg(test)]".to_string());
            } else if test_fns.len() <= 3 {
                test_info.push(format!("inline: {}", test_fns.join(", ")));
            } else {
                test_info.push(format!(
                    "inline: {}, +{} more",
                    test_fns[..3].join(", "),
                    test_fns.len() - 3
                ));
            }
        }

        if let Some(test_files) = test_modules.get(&file_result.relative_path) {
            for test_file in test_files {
                test_info.push(test_file.clone());
            }
        }

        if !test_info.is_empty() {
            write!(buffer, " [tests: {}]", test_info.join("; "))?;
        }

        writeln!(buffer)?;
    }

    file.write_all(&buffer).await?;
    Ok(())
}

fn calculate_churn_thresholds(churn_data: &HashMap<PathBuf, u32>) -> (u32, u32) {
    if churn_data.is_empty() {
        return (10, 5);
    }

    let mut counts: Vec<u32> = churn_data.values().copied().collect();
    counts.sort_unstable();

    let len = counts.len();
    let high = counts.get(len * 2 / 3).copied().unwrap_or(10);
    let med = counts.get(len / 3).copied().unwrap_or(5);

    (high.max(1), med.max(1))
}

fn find_test_modules(result: &PipelineResult) -> HashMap<String, Vec<String>> {
    let mut test_map: HashMap<String, Vec<String>> = HashMap::new();

    let test_files: Vec<_> = result
        .files
        .iter()
        .filter(|f| {
            let role = super::file_role(&f.path);
            role == "[test]"
        })
        .collect();

    for test_file in &test_files {
        let test_name = test_file
            .relative_path
            .rsplit('/')
            .next()
            .unwrap_or(&test_file.relative_path);

        let base_name = test_name
            .strip_suffix("_test.rs")
            .or_else(|| test_name.strip_suffix("_tests.rs"))
            .unwrap_or("");

        if !base_name.is_empty() {
            for source_file in &result.files {
                let source_name = source_file
                    .relative_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&source_file.relative_path);

                if source_name == format!("{}.rs", base_name) {
                    test_map
                        .entry(source_file.relative_path.clone())
                        .or_default()
                        .push(test_file.relative_path.clone());
                }
            }
        }
    }

    test_map
}
