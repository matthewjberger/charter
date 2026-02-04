use anyhow::Result;
use std::path::Path;
use tokio::fs;

pub async fn query(root: &Path, query_str: &str, limit: usize) -> Result<()> {
    let atlas_dir = root.join(".atlas");

    if !atlas_dir.exists() {
        eprintln!("No .atlas/ directory found. Run 'atlas' first.");
        std::process::exit(1);
    }

    let query_type = parse_query(query_str);

    match query_type {
        QueryType::CallersOf { target } => {
            find_callers(&atlas_dir, &target, limit).await?;
        }
        QueryType::CalleesOf { target } => {
            find_callees(&atlas_dir, &target, limit).await?;
        }
        QueryType::ImplementorsOf { trait_name } => {
            find_implementors(&atlas_dir, &trait_name, limit).await?;
        }
        QueryType::UsersOf { symbol } => {
            find_users(&atlas_dir, &symbol, limit).await?;
        }
        QueryType::ErrorsIn { file } => {
            find_errors_in(&atlas_dir, &file, limit).await?;
        }
        QueryType::Hotspots => {
            find_hotspots(&atlas_dir, limit).await?;
        }
        QueryType::PublicApi => {
            find_public_api(&atlas_dir, limit).await?;
        }
        QueryType::Keyword { terms } => {
            keyword_search(&atlas_dir, &terms, limit).await?;
        }
    }

    Ok(())
}

enum QueryType {
    CallersOf { target: String },
    CalleesOf { target: String },
    ImplementorsOf { trait_name: String },
    UsersOf { symbol: String },
    ErrorsIn { file: String },
    Hotspots,
    PublicApi,
    Keyword { terms: Vec<String> },
}

fn parse_query(query: &str) -> QueryType {
    let query_lower = query.to_lowercase();

    if query_lower.starts_with("callers of ") {
        let target = query[11..].trim().to_string();
        return QueryType::CallersOf { target };
    }

    if query_lower.starts_with("callees of ") || query_lower.starts_with("calls from ") {
        let target = query[11..].trim().to_string();
        return QueryType::CalleesOf { target };
    }

    if query_lower.starts_with("implementors of ") || query_lower.starts_with("impls of ") {
        let start = if query_lower.starts_with("impls of ") {
            9
        } else {
            16
        };
        let trait_name = query[start..].trim().to_string();
        return QueryType::ImplementorsOf { trait_name };
    }

    if query_lower.starts_with("users of ") || query_lower.starts_with("references to ") {
        let start = if query_lower.starts_with("users of ") {
            9
        } else {
            14
        };
        let symbol = query[start..].trim().to_string();
        return QueryType::UsersOf { symbol };
    }

    if query_lower.starts_with("errors in ") {
        let file = query[10..].trim().to_string();
        return QueryType::ErrorsIn { file };
    }

    if query_lower == "hotspots" || query_lower == "complex functions" || query_lower == "hot" {
        return QueryType::Hotspots;
    }

    if query_lower == "public api" || query_lower == "public functions" || query_lower == "exports"
    {
        return QueryType::PublicApi;
    }

    let terms: Vec<String> = query
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() > 2)
        .collect();

    QueryType::Keyword { terms }
}

async fn find_callers(atlas_dir: &Path, target: &str, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("calls.md")).await?;

    println!("Callers of '{}':", target);
    println!();

    let target_lower = target.to_lowercase();
    let mut found = 0;

    for line in content.lines() {
        if line.starts_with("  ") && line.contains(" → ") {
            let parts: Vec<&str> = line.trim().splitn(2, " → ").collect();
            if parts.len() == 2 {
                let callees = parts[1];
                let callees_lower = callees.to_lowercase();
                if callees_lower.contains(&target_lower) {
                    let caller = parts[0];
                    println!("  {} calls {}", caller, target);
                    found += 1;
                    if found >= limit {
                        break;
                    }
                }
            }
        }
    }

    if found == 0 {
        println!("  No callers found for '{}'", target);
    } else {
        println!();
        println!("Found {} caller(s)", found);
    }

    Ok(())
}

async fn find_callees(atlas_dir: &Path, target: &str, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("calls.md")).await?;

    println!("Callees of '{}':", target);
    println!();

    let target_lower = target.to_lowercase();
    let mut found = 0;

    for line in content.lines() {
        if line.starts_with("  ") && line.contains(" → ") {
            let parts: Vec<&str> = line.trim().splitn(2, " → ").collect();
            if parts.len() == 2 {
                let caller = parts[0].to_lowercase();
                if caller.contains(&target_lower) {
                    let callees = parts[1];
                    println!("  {} → {}", parts[0], callees);
                    found += 1;
                    if found >= limit {
                        break;
                    }
                }
            }
        }
    }

    if found == 0 {
        println!("  No callees found for '{}'", target);
    } else {
        println!();
        println!("Found {} match(es)", found);
    }

    Ok(())
}

async fn find_implementors(atlas_dir: &Path, trait_name: &str, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("types.md")).await?;

    println!("Implementors of '{}':", trait_name);
    println!();

    let trait_lower = trait_name.to_lowercase();
    let mut found = 0;
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

            if line.starts_with("  ") && line.contains(" -> ") {
                let parts: Vec<&str> = line.trim().splitn(2, " -> ").collect();
                if parts.len() == 2 {
                    let impl_trait = parts[0].to_lowercase();
                    if impl_trait.contains(&trait_lower) {
                        let types = parts[1].trim_start_matches('[').trim_end_matches(']');
                        println!("  {} implements {}", types, parts[0]);
                        found += 1;
                        if found >= limit {
                            break;
                        }
                    }
                }
            }
        }
    }

    if found == 0 {
        println!("  No implementors found for '{}'", trait_name);
    } else {
        println!();
        println!("Found {} trait implementation(s)", found);
    }

    Ok(())
}

async fn find_users(atlas_dir: &Path, symbol: &str, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("refs.md")).await?;

    println!("References to '{}':", symbol);
    println!();

    let symbol_lower = symbol.to_lowercase();
    let mut found = 0;

    for line in content.lines() {
        if line.starts_with('[') || line.is_empty() {
            continue;
        }

        if let Some((name_part, _)) = line.split_once(" [") {
            if name_part.to_lowercase() == symbol_lower {
                println!("  {}", line);
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    if found == 0 {
        let symbols_content = fs::read_to_string(atlas_dir.join("symbols.md"))
            .await
            .unwrap_or_default();
        let mut partial_matches = Vec::new();

        for line in symbols_content.lines() {
            if line.starts_with("  ") && !line.starts_with("    ") {
                let trimmed = line.trim();
                if trimmed.to_lowercase().contains(&symbol_lower) {
                    partial_matches.push(trimmed.to_string());
                }
            }
        }

        if partial_matches.is_empty() {
            println!("  No references found for '{}'", symbol);
        } else {
            println!("  No exact matches. Similar symbols:");
            for sym_match in partial_matches.iter().take(5) {
                println!("    {}", sym_match);
            }
        }
    } else {
        println!();
        println!("Found {} reference(s)", found);
    }

    Ok(())
}

async fn find_errors_in(atlas_dir: &Path, file: &str, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("errors.md")).await?;

    println!("Errors in '{}':", file);
    println!();

    let file_lower = file.to_lowercase();
    let mut found = 0;
    let mut current_matches = false;

    for line in content.lines() {
        if line.contains(":") && !line.starts_with("  ") && !line.starts_with("#") {
            let file_path = line.split(':').next().unwrap_or("");
            current_matches = file_path.to_lowercase().contains(&file_lower);

            if current_matches {
                println!("{}", line);
                found += 1;
                if found >= limit {
                    break;
                }
            }
        } else if current_matches && line.starts_with("  ") {
            println!("{}", line);
        }
    }

    if found == 0 {
        println!("  No error patterns found in '{}'", file);
    }

    Ok(())
}

async fn find_hotspots(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("hotspots.md")).await?;

    println!("Top {} hotspots by importance:", limit);
    println!();

    let mut found = 0;

    for line in content.lines() {
        if line.contains("[score=") && !line.starts_with("#") && !line.starts_with("[") {
            println!("  {}", line);
            found += 1;
            if found >= limit {
                break;
            }
        }
    }

    if found == 0 {
        println!("  No hotspots found");
    }

    Ok(())
}

async fn find_public_api(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("symbols.md")).await?;

    println!("Public API (first {} items):", limit);
    println!();

    let mut found = 0;
    let mut current_file = String::new();

    for line in content.lines() {
        if !line.starts_with(' ')
            && !line.is_empty()
            && !line.starts_with('[')
            && line.contains(".rs")
        {
            current_file = line.split_whitespace().next().unwrap_or("").to_string();
        }

        if line.starts_with("  pub ") && !line.starts_with("    ") {
            let trimmed = line.trim();
            if trimmed.starts_with("pub fn ")
                || trimmed.starts_with("pub struct ")
                || trimmed.starts_with("pub enum ")
                || trimmed.starts_with("pub trait ")
            {
                println!("  {} → {}", current_file, trimmed);
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    if found == 0 {
        println!("  No public API found");
    }

    Ok(())
}

async fn keyword_search(atlas_dir: &Path, terms: &[String], limit: usize) -> Result<()> {
    let mut results: Vec<(String, f32)> = Vec::new();

    let symbols_content = fs::read_to_string(atlas_dir.join("symbols.md"))
        .await
        .unwrap_or_default();
    search_in_content(&symbols_content, terms, "symbols", &mut results);

    let types_content = fs::read_to_string(atlas_dir.join("types.md"))
        .await
        .unwrap_or_default();
    search_in_content(&types_content, terms, "types", &mut results);

    let calls_content = fs::read_to_string(atlas_dir.join("calls.md"))
        .await
        .unwrap_or_default();
    search_in_content(&calls_content, terms, "calls", &mut results);

    let errors_content = fs::read_to_string(atlas_dir.join("errors.md"))
        .await
        .unwrap_or_default();
    search_in_content(&errors_content, terms, "errors", &mut results);

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.dedup_by(|a, b| a.0 == b.0);

    println!("Search results for '{}':", terms.join(" "));
    println!();

    if results.is_empty() {
        println!("  No results found");
    } else {
        for (line, score) in results.iter().take(limit) {
            println!("  [relevance={:.1}] {}", score, line);
        }
        println!();
        println!("Found {} result(s)", results.len().min(limit));
    }

    Ok(())
}

fn search_in_content(
    content: &str,
    terms: &[String],
    source: &str,
    results: &mut Vec<(String, f32)>,
) {
    for line in content.lines() {
        if line.starts_with('[') || line.is_empty() {
            continue;
        }

        let line_lower = line.to_lowercase();
        let mut score = 0.0;

        for term in terms {
            if line_lower.contains(term) {
                score += 1.0;
                if line.trim().to_lowercase().starts_with(term) {
                    score += 0.5;
                }
            }
        }

        if score > 0.0 {
            let trimmed = line.trim();
            if !trimmed.is_empty() && trimmed.len() < 200 {
                results.push((format!("[{}] {}", source, trimmed), score));
            }
        }
    }
}
