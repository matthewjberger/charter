use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::extract::errors::{ErrorInfo, ErrorReturnType};
use crate::extract::symbols::Visibility;
use crate::pipeline::PipelineResult;

pub async fn write_errors(atlas_dir: &Path, result: &PipelineResult, stamp: &str) -> Result<()> {
    let file = tokio::fs::File::create(atlas_dir.join("errors.md")).await?;
    let mut writer = BufWriter::new(file);

    writer.write_all(stamp.as_bytes()).await?;
    writer.write_all(b"\n\n").await?;
    writer.write_all(b"# Error Propagation\n\n").await?;

    let mut all_errors: Vec<(&str, &ErrorInfo)> = Vec::new();
    for file_result in &result.files {
        for error_info in &file_result.parsed.error_info {
            all_errors.push((&file_result.relative_path, error_info));
        }
    }

    if all_errors.is_empty() {
        writer
            .write_all(b"No fallible functions detected.\n")
            .await?;
        writer.flush().await?;
        return Ok(());
    }

    let error_sources: Vec<_> = all_errors
        .iter()
        .filter(|(_, info)| info.is_error_source())
        .collect();

    let public_fallible: Vec<_> = all_errors
        .iter()
        .filter(|(file_path, info)| {
            is_public_function(
                result,
                file_path,
                &info.function_id.name,
                info.function_id.impl_type.as_deref(),
            )
        })
        .collect();

    let propagation_heavy: Vec<_> = all_errors
        .iter()
        .filter(|(_, info)| info.propagation_count() >= 3)
        .collect();

    if !error_sources.is_empty() {
        writer.write_all(b"## Error Origins\n\n").await?;
        writer
            .write_all(b"Functions that create errors (Err(), anyhow!(), bail!, None).\n\n")
            .await?;

        for (file_path, info) in error_sources.iter().take(30) {
            let qualified = info.function_id.qualified_name();
            let origins: Vec<String> = info
                .error_origins
                .iter()
                .map(|o| {
                    if let Some(msg) = &o.message {
                        let msg_short = if msg.len() > 40 {
                            format!("{}...", &msg[..37])
                        } else {
                            msg.clone()
                        };
                        format!("{}:{} {} \"{}\"", file_path, o.line, o.kind, msg_short)
                    } else {
                        format!("{}:{} {}", file_path, o.line, o.kind)
                    }
                })
                .collect();

            let line = format!("{}:{} {}\n", file_path, info.line, qualified);
            writer.write_all(line.as_bytes()).await?;

            for origin in origins.iter().take(3) {
                let line = format!("  {}\n", origin);
                writer.write_all(line.as_bytes()).await?;
            }
            if origins.len() > 3 {
                let line = format!("  [+{} more origins]\n", origins.len() - 3);
                writer.write_all(line.as_bytes()).await?;
            }
        }

        if error_sources.len() > 30 {
            writer
                .write_all(
                    format!(
                        "\n[+{} more error-originating functions]\n",
                        error_sources.len() - 30
                    )
                    .as_bytes(),
                )
                .await?;
        }
        writer.write_all(b"\n").await?;
    }

    if !public_fallible.is_empty() {
        writer.write_all(b"## Public API Surface\n\n").await?;
        writer
            .write_all(b"Public functions that return Result or Option.\n\n")
            .await?;

        let mut by_return_type: HashMap<String, Vec<String>> = HashMap::new();

        for (file_path, info) in &public_fallible {
            let return_type_str = format_return_type(&info.return_type);
            let entry = format!(
                "{}:{} {}",
                file_path,
                info.line,
                info.function_id.qualified_name()
            );
            by_return_type
                .entry(return_type_str)
                .or_default()
                .push(entry);
        }

        let mut sorted_types: Vec<_> = by_return_type.keys().collect();
        sorted_types.sort();

        for return_type in sorted_types {
            let functions = by_return_type.get(return_type).unwrap();
            let line = format!("{} [{}]\n", return_type, functions.len());
            writer.write_all(line.as_bytes()).await?;

            for func in functions.iter().take(5) {
                let line = format!("  {}\n", func);
                writer.write_all(line.as_bytes()).await?;
            }
            if functions.len() > 5 {
                let line = format!("  [+{} more]\n", functions.len() - 5);
                writer.write_all(line.as_bytes()).await?;
            }
        }
        writer.write_all(b"\n").await?;
    }

    if !propagation_heavy.is_empty() {
        writer.write_all(b"## Propagation Chains\n\n").await?;
        writer
            .write_all(b"Functions with 3+ error propagation points (?).\n\n")
            .await?;

        let mut sorted: Vec<_> = propagation_heavy.clone();
        sorted.sort_by(|a, b| b.1.propagation_count().cmp(&a.1.propagation_count()));

        for (file_path, info) in sorted.iter().take(20) {
            let qualified = info.function_id.qualified_name();
            let line = format!(
                "{}:{} {} [{} propagation points]\n",
                file_path,
                info.line,
                qualified,
                info.propagation_count()
            );
            writer.write_all(line.as_bytes()).await?;

            for prop in info.propagation_points.iter().take(3) {
                let line = format!("  L{}: {}\n", prop.line, prop.expression);
                writer.write_all(line.as_bytes()).await?;
            }
            if info.propagation_points.len() > 3 {
                let line = format!("  [+{} more points]\n", info.propagation_points.len() - 3);
                writer.write_all(line.as_bytes()).await?;
            }
        }
    }

    writer.write_all(b"\n## Stats\n\n").await?;
    let total_fallible = all_errors.len();
    let total_origins: usize = all_errors.iter().map(|(_, i)| i.error_origins.len()).sum();
    let total_propagations: usize = all_errors
        .iter()
        .map(|(_, i)| i.propagation_points.len())
        .sum();

    let stats = format!(
        "Fallible functions: {}\nError origin points: {}\nPropagation points (?): {}\n",
        total_fallible, total_origins, total_propagations
    );
    writer.write_all(stats.as_bytes()).await?;

    writer.flush().await?;
    Ok(())
}

fn format_return_type(return_type: &ErrorReturnType) -> String {
    match return_type {
        ErrorReturnType::Result { ok_type, err_type } => {
            let ok_short = shorten_type(ok_type);
            let err_short = shorten_type(err_type);
            format!("Result<{}, {}>", ok_short, err_short)
        }
        ErrorReturnType::Option { some_type } => {
            let some_short = shorten_type(some_type);
            format!("Option<{}>", some_short)
        }
        ErrorReturnType::Neither => "()".to_string(),
    }
}

fn shorten_type(type_str: &str) -> String {
    if type_str.len() <= 20 {
        return type_str.to_string();
    }

    if let Some(last_segment) = type_str.split("::").last() {
        if last_segment.len() <= 20 {
            return last_segment.to_string();
        }
    }

    format!("{}...", &type_str[..17])
}

fn is_public_function(
    result: &PipelineResult,
    file_path: &str,
    fn_name: &str,
    impl_type: Option<&str>,
) -> bool {
    for file_result in &result.files {
        if file_result.relative_path != file_path {
            continue;
        }

        for symbol in &file_result.parsed.symbols.symbols {
            if symbol.name == fn_name {
                return matches!(symbol.visibility, Visibility::Public | Visibility::PubCrate);
            }
        }

        if let Some(type_name) = impl_type {
            for inherent_impl in &file_result.parsed.symbols.inherent_impls {
                if inherent_impl.type_name == type_name {
                    for method in &inherent_impl.methods {
                        if method.name == fn_name {
                            return matches!(
                                method.visibility,
                                Visibility::Public | Visibility::PubCrate
                            );
                        }
                    }
                }
            }
        }
    }

    false
}
