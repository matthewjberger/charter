use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::extract::calls::CallInfo;
use crate::pipeline::PipelineResult;

struct CallerEntry {
    caller_name: String,
    file: String,
    line: usize,
    is_async: bool,
    is_try: bool,
}

pub async fn write_calls(charter_dir: &Path, result: &PipelineResult, stamp: &str) -> Result<()> {
    let file = tokio::fs::File::create(charter_dir.join("calls.md")).await?;
    let mut writer = BufWriter::new(file);

    writer.write_all(stamp.as_bytes()).await?;
    writer.write_all(b"\n\n").await?;
    writer.write_all(b"# Call Graph\n\n").await?;

    let mut all_calls: Vec<&CallInfo> = Vec::new();
    for file_result in &result.files {
        for call_info in &file_result.parsed.call_graph {
            all_calls.push(call_info);
        }
    }

    if all_calls.is_empty() {
        writer.write_all(b"No function calls detected.\n").await?;
        writer.flush().await?;
        return Ok(());
    }

    let call_counts = compute_call_counts(&all_calls);
    let hot_paths = find_hot_paths(&call_counts);

    if !hot_paths.is_empty() {
        writer.write_all(b"## Hot Paths\n\n").await?;
        writer
            .write_all(b"Functions called most frequently across the codebase.\n\n")
            .await?;

        for (target, count) in hot_paths.iter().take(20) {
            let line = format!("{} [called {} times]\n", target, count);
            writer.write_all(line.as_bytes()).await?;
        }
        writer.write_all(b"\n").await?;
    }

    writer.write_all(b"## Call Map\n\n").await?;
    writer
        .write_all(b"Function -> callee relationships by file.\n\n")
        .await?;

    let mut files_with_calls: Vec<&str> = result
        .files
        .iter()
        .filter(|f| !f.parsed.call_graph.is_empty())
        .map(|f| f.relative_path.as_str())
        .collect();
    files_with_calls.sort();

    for file_path in files_with_calls {
        let file_result = result
            .files
            .iter()
            .find(|f| f.relative_path == file_path)
            .unwrap();

        if file_result.parsed.call_graph.is_empty() {
            continue;
        }

        let call_count: usize = file_result
            .parsed
            .call_graph
            .iter()
            .map(|c| c.callees.len())
            .sum();

        let line = format!(
            "{} [{} functions, {} calls]\n",
            file_path,
            file_result.parsed.call_graph.len(),
            call_count
        );
        writer.write_all(line.as_bytes()).await?;

        for call_info in &file_result.parsed.call_graph {
            if call_info.callees.is_empty() {
                continue;
            }

            let caller_name = call_info.caller.qualified_name();
            let callees: Vec<String> = call_info
                .callees
                .iter()
                .map(|edge| {
                    let mut target = edge.qualified_target();
                    if edge.is_async_call {
                        target = format!("{}.await", target);
                    }
                    if edge.is_try_call {
                        target = format!("{}?", target);
                    }
                    target
                })
                .collect();

            let unique_callees: HashSet<&str> = callees.iter().map(|s| s.as_str()).collect();
            let mut sorted_callees: Vec<&str> = unique_callees.into_iter().collect();
            sorted_callees.sort();

            if sorted_callees.len() <= 5 {
                let line = format!("  {} → {}\n", caller_name, sorted_callees.join(", "));
                writer.write_all(line.as_bytes()).await?;
            } else {
                let shown: Vec<&str> = sorted_callees.iter().take(4).copied().collect();
                let line = format!(
                    "  {} → {} [+{} more]\n",
                    caller_name,
                    shown.join(", "),
                    sorted_callees.len() - 4
                );
                writer.write_all(line.as_bytes()).await?;
            }
        }

        writer.write_all(b"\n").await?;
    }

    let reverse_graph = build_reverse_call_graph(result);
    write_callers_section(&mut writer, &reverse_graph).await?;

    let async_calls = count_async_calls(&all_calls);
    let try_calls = count_try_calls(&all_calls);

    if async_calls > 0 || try_calls > 0 {
        writer.write_all(b"## Stats\n\n").await?;
        let line = format!(
            "Total calls: {}, async calls: {}, fallible calls (?): {}\n",
            all_calls.iter().map(|c| c.callees.len()).sum::<usize>(),
            async_calls,
            try_calls
        );
        writer.write_all(line.as_bytes()).await?;
    }

    writer.flush().await?;
    Ok(())
}

fn compute_call_counts(calls: &[&CallInfo]) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();

    for call_info in calls {
        for edge in &call_info.callees {
            let target = edge.qualified_target();
            *counts.entry(target).or_insert(0) += 1;
        }
    }

    counts
}

fn find_hot_paths(call_counts: &HashMap<String, u32>) -> Vec<(String, u32)> {
    let mut hot: Vec<(String, u32)> = call_counts
        .iter()
        .filter(|(target, count)| **count >= 3 && !is_common_utility(target))
        .map(|(target, count)| (target.clone(), *count))
        .collect();

    hot.sort_by(|a, b| b.1.cmp(&a.1));
    hot
}

fn is_common_utility(name: &str) -> bool {
    const COMMON: &[&str] = &[
        "unwrap",
        "expect",
        "clone",
        "to_string",
        "to_owned",
        "into",
        "from",
        "as_ref",
        "as_mut",
        "ok",
        "err",
        "some",
        "none",
        "push",
        "pop",
        "insert",
        "remove",
        "get",
        "len",
        "is_empty",
        "iter",
        "collect",
        "map",
        "filter",
        "and_then",
        "or_else",
        "ok_or",
        "ok_or_else",
        "unwrap_or",
        "unwrap_or_else",
        "unwrap_or_default",
        "default",
        "new",
        "with_capacity",
        "format!",
        "println!",
        "eprintln!",
        "write!",
        "writeln!",
        "vec!",
        "debug!",
        "info!",
        "warn!",
        "error!",
        "trace!",
    ];

    let base_name = name.split("::").last().unwrap_or(name);
    COMMON.contains(&base_name)
}

fn count_async_calls(calls: &[&CallInfo]) -> usize {
    calls
        .iter()
        .flat_map(|c| &c.callees)
        .filter(|e| e.is_async_call)
        .count()
}

fn count_try_calls(calls: &[&CallInfo]) -> usize {
    calls
        .iter()
        .flat_map(|c| &c.callees)
        .filter(|e| e.is_try_call)
        .count()
}

fn build_reverse_call_graph(result: &PipelineResult) -> HashMap<String, Vec<CallerEntry>> {
    let mut reverse: HashMap<String, Vec<CallerEntry>> = HashMap::new();

    for file_result in &result.files {
        for call_info in &file_result.parsed.call_graph {
            let caller_name = call_info.caller.qualified_name();

            for edge in &call_info.callees {
                let target = edge.qualified_target();

                if is_common_utility(&target) {
                    continue;
                }

                reverse.entry(target).or_default().push(CallerEntry {
                    caller_name: caller_name.clone(),
                    file: file_result.relative_path.clone(),
                    line: edge.line,
                    is_async: edge.is_async_call,
                    is_try: edge.is_try_call,
                });
            }
        }
    }

    for callers in reverse.values_mut() {
        callers.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.caller_name.cmp(&b.caller_name))
                .then_with(|| a.line.cmp(&b.line))
        });
        callers.dedup_by(|a, b| a.file == b.file && a.caller_name == b.caller_name);
    }

    reverse
}

async fn write_callers_section(
    writer: &mut BufWriter<tokio::fs::File>,
    reverse_graph: &HashMap<String, Vec<CallerEntry>>,
) -> Result<()> {
    let mut targets_with_callers: Vec<(&String, &Vec<CallerEntry>)> = reverse_graph
        .iter()
        .filter(|(_, callers)| callers.len() >= 3)
        .collect();

    if targets_with_callers.is_empty() {
        return Ok(());
    }

    targets_with_callers.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));

    writer.write_all(b"## Callers\n\n").await?;
    writer
        .write_all(b"Functions that call each target, sorted by call count.\n\n")
        .await?;

    for (target, callers) in targets_with_callers.iter().take(50) {
        let header = format!("{} [{} callers]\n", target, callers.len());
        writer.write_all(header.as_bytes()).await?;

        let shown_count = 5.min(callers.len());
        for caller in callers.iter().take(shown_count) {
            let mut suffix = String::new();
            if caller.is_async {
                suffix.push_str(" [async]");
            }
            if caller.is_try {
                suffix.push_str(" [?]");
            }

            let line = format!(
                "  {} ({}:{}){}\n",
                caller.caller_name, caller.file, caller.line, suffix
            );
            writer.write_all(line.as_bytes()).await?;
        }

        if callers.len() > shown_count {
            let more_line = format!("  [+{} more]\n", callers.len() - shown_count);
            writer.write_all(more_line.as_bytes()).await?;
        }

        writer.write_all(b"\n").await?;
    }

    Ok(())
}
