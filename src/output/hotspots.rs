use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::extract::complexity::{FunctionComplexity, ImportanceTier};
use crate::pipeline::PipelineResult;

pub async fn write_hotspots(
    atlas_dir: &Path,
    result: &PipelineResult,
    churn_data: &HashMap<PathBuf, u32>,
    stamp: &str,
) -> Result<()> {
    let file = tokio::fs::File::create(atlas_dir.join("hotspots.md")).await?;
    let mut writer = BufWriter::new(file);

    writer.write_all(stamp.as_bytes()).await?;
    writer.write_all(b"\n\n").await?;
    writer.write_all(b"# Hotspots\n\n").await?;

    let mut all_functions: Vec<(String, FunctionComplexity)> = Vec::new();

    for file_result in &result.files {
        let churn_score = churn_data.get(&file_result.path).copied().unwrap_or(0);

        for mut func in file_result.parsed.complexity.clone() {
            func.metrics.churn_score = churn_score;
            all_functions.push((file_result.relative_path.clone(), func));
        }
    }

    update_call_sites(&mut all_functions, result);

    all_functions.sort_by(|a, b| {
        b.1.metrics
            .importance_score()
            .cmp(&a.1.metrics.importance_score())
    });

    let high_tier: Vec<_> = all_functions
        .iter()
        .filter(|(_, f)| f.metrics.tier() == ImportanceTier::High)
        .collect();

    let medium_tier: Vec<_> = all_functions
        .iter()
        .filter(|(_, f)| f.metrics.tier() == ImportanceTier::Medium)
        .collect();

    if high_tier.is_empty() && medium_tier.is_empty() {
        writer
            .write_all(b"No high-complexity functions detected.\n")
            .await?;
        writer.flush().await?;
        return Ok(());
    }

    if !high_tier.is_empty() {
        writer.write_all(b"## High Importance\n\n").await?;
        writer
            .write_all(b"Functions with score >= 30. Critical paths requiring careful review.\n\n")
            .await?;

        for (file_path, func) in high_tier.iter().take(50) {
            let line = format_hotspot_line(file_path, func);
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }

        if high_tier.len() > 50 {
            writer
                .write_all(
                    format!(
                        "\n[+{} more high-importance functions]\n",
                        high_tier.len() - 50
                    )
                    .as_bytes(),
                )
                .await?;
        }

        writer.write_all(b"\n").await?;
    }

    if !medium_tier.is_empty() {
        writer.write_all(b"## Medium Importance\n\n").await?;
        writer
            .write_all(b"Functions with score 15-29. Worth understanding but not critical.\n\n")
            .await?;

        for (file_path, func) in medium_tier.iter().take(30) {
            let line = format_hotspot_line(file_path, func);
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }

        if medium_tier.len() > 30 {
            writer
                .write_all(
                    format!(
                        "\n[+{} more medium-importance functions]\n",
                        medium_tier.len() - 30
                    )
                    .as_bytes(),
                )
                .await?;
        }
    }

    writer.write_all(b"\n## Scoring\n\n").await?;
    writer
        .write_all(b"Score = (cyclomatic * 2) + (lines / 10) + (call_sites * 3) + (churn * 2) + (public ? 10 : 0)\n")
        .await?;
    writer.write_all(b"- High: >= 30\n").await?;
    writer.write_all(b"- Medium: 15-29\n").await?;
    writer.write_all(b"- Low: < 15 (not shown)\n").await?;

    writer.flush().await?;
    Ok(())
}

fn format_hotspot_line(file_path: &str, func: &FunctionComplexity) -> String {
    let qualified = func.qualified_name();
    let metrics = &func.metrics;
    let score = metrics.importance_score();

    let mut details = Vec::new();
    details.push(format!("cc={}", metrics.cyclomatic));
    details.push(format!("lines={}", metrics.line_count));
    if metrics.nesting_depth > 2 {
        details.push(format!("depth={}", metrics.nesting_depth));
    }
    if metrics.call_sites > 0 {
        details.push(format!("called={}", metrics.call_sites));
    }
    if metrics.churn_score > 0 {
        details.push(format!("churn={}", metrics.churn_score));
    }
    if metrics.is_public {
        details.push("pub".to_string());
    }

    format!(
        "{}:{} {} [score={}] ({})",
        file_path,
        func.line,
        qualified,
        score,
        details.join(", ")
    )
}

fn update_call_sites(functions: &mut [(String, FunctionComplexity)], result: &PipelineResult) {
    let mut call_counts: HashMap<String, u32> = HashMap::new();

    for file_result in &result.files {
        for call_info in &file_result.parsed.call_graph {
            for callee in &call_info.callees {
                let key = match &callee.target_type {
                    Some(type_name) => format!("{}::{}", type_name, callee.target),
                    None => callee.target.clone(),
                };
                *call_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    for (_, func) in functions.iter_mut() {
        let key = func.qualified_name();
        if let Some(count) = call_counts.get(&key) {
            func.metrics.call_sites = *count;
        }

        if let Some(count) = call_counts.get(&func.name) {
            func.metrics.call_sites = func.metrics.call_sites.max(*count);
        }
    }
}
