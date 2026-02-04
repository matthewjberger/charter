use anyhow::Result;
use std::path::Path;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::pipeline::{CapturedBody, PipelineResult};

pub async fn write_snippets(atlas_dir: &Path, result: &PipelineResult, stamp: &str) -> Result<()> {
    let file = tokio::fs::File::create(atlas_dir.join("snippets.md")).await?;
    let mut writer = BufWriter::new(file);

    writer.write_all(stamp.as_bytes()).await?;
    writer.write_all(b"\n\n").await?;
    writer.write_all(b"# Implementation Snippets\n\n").await?;
    writer
        .write_all(b"Function bodies captured for high and medium importance functions.\n\n")
        .await?;

    let mut all_bodies: Vec<(&str, &CapturedBody)> = Vec::new();

    for file_result in &result.files {
        for body in &file_result.parsed.captured_bodies {
            all_bodies.push((&file_result.relative_path, body));
        }
    }

    if all_bodies.is_empty() {
        writer.write_all(b"No function bodies captured.\n").await?;
        writer.flush().await?;
        return Ok(());
    }

    all_bodies.sort_by(|a, b| b.1.importance_score.cmp(&a.1.importance_score));

    let full_bodies: Vec<_> = all_bodies
        .iter()
        .filter(|(_, body)| body.body.full_text.is_some())
        .collect();

    let summaries: Vec<_> = all_bodies
        .iter()
        .filter(|(_, body)| body.body.full_text.is_none() && body.body.summary.is_some())
        .collect();

    if !full_bodies.is_empty() {
        writer
            .write_all(b"## Full Implementations (High Importance)\n\n")
            .await?;

        for (file_path, body) in full_bodies.iter().take(30) {
            let qualified = qualified_name(&body.function_name, body.impl_type.as_deref());

            let header = format!(
                "### {}:{} {} [score={}]\n\n",
                file_path, body.line, qualified, body.importance_score
            );
            writer.write_all(header.as_bytes()).await?;

            writer.write_all(b"```rust\n").await?;
            if let Some(ref text) = body.body.full_text {
                writer.write_all(text.as_bytes()).await?;
            }
            writer.write_all(b"\n```\n\n").await?;
        }

        if full_bodies.len() > 30 {
            let msg = format!(
                "[+{} more full implementations not shown]\n\n",
                full_bodies.len() - 30
            );
            writer.write_all(msg.as_bytes()).await?;
        }
    }

    if !summaries.is_empty() {
        writer
            .write_all(b"## Summaries (Medium Importance)\n\n")
            .await?;

        for (file_path, body) in summaries.iter().take(50) {
            let qualified = qualified_name(&body.function_name, body.impl_type.as_deref());

            let header = format!(
                "{}:{} {} [score={}]\n",
                file_path, body.line, qualified, body.importance_score
            );
            writer.write_all(header.as_bytes()).await?;

            if let Some(ref summary) = body.body.summary {
                let stats = format!(
                    "  {} lines, {} statements\n",
                    summary.line_count, summary.statement_count
                );
                writer.write_all(stats.as_bytes()).await?;

                if !summary.early_returns.is_empty() {
                    writer.write_all(b"  Early returns:\n").await?;
                    for ret in summary.early_returns.iter().take(3) {
                        let line = format!("    {}\n", ret);
                        writer.write_all(line.as_bytes()).await?;
                    }
                }

                if !summary.key_calls.is_empty() {
                    let calls_str = summary
                        .key_calls
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    let line = format!("  Key calls: {}\n", calls_str);
                    writer.write_all(line.as_bytes()).await?;
                }
            }

            writer.write_all(b"\n").await?;
        }

        if summaries.len() > 50 {
            let msg = format!("[+{} more summaries not shown]\n\n", summaries.len() - 50);
            writer.write_all(msg.as_bytes()).await?;
        }
    }

    let stats = format!(
        "## Stats\n\nFull implementations captured: {}\nSummaries captured: {}\n",
        full_bodies.len(),
        summaries.len()
    );
    writer.write_all(stats.as_bytes()).await?;

    writer.flush().await?;
    Ok(())
}

fn qualified_name(name: &str, impl_type: Option<&str>) -> String {
    match impl_type {
        Some(type_name) => format!("{}::{}", type_name, name),
        None => name.to_string(),
    }
}
