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
        QueryType::Panics => {
            find_panics(&atlas_dir, limit).await?;
        }
        QueryType::PanicsIn { file } => {
            find_panics_in(&atlas_dir, &file, limit).await?;
        }
        QueryType::UnsafeCode => {
            find_unsafe_code(&atlas_dir, limit).await?;
        }
        QueryType::AsyncFunctions => {
            find_async_functions(&atlas_dir, limit).await?;
        }
        QueryType::Lifetimes => {
            find_lifetimes(&atlas_dir, limit).await?;
        }
        QueryType::Tests => {
            find_tests(&atlas_dir, limit).await?;
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
    Panics,
    PanicsIn { file: String },
    UnsafeCode,
    AsyncFunctions,
    Lifetimes,
    Tests,
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

    if query_lower == "panics" || query_lower == "panic points" || query_lower == "unwraps" {
        return QueryType::Panics;
    }

    if query_lower.starts_with("panics in ") {
        let file = query[10..].trim().to_string();
        return QueryType::PanicsIn { file };
    }

    if query_lower == "unsafe" || query_lower == "unsafe code" || query_lower == "unsafe blocks" {
        return QueryType::UnsafeCode;
    }

    if query_lower == "async" || query_lower == "async functions" || query_lower == "async fns" {
        return QueryType::AsyncFunctions;
    }

    if query_lower == "lifetimes" || query_lower == "lifetime" || query_lower == "borrows" {
        return QueryType::Lifetimes;
    }

    if query_lower == "tests" || query_lower == "test functions" || query_lower == "test coverage" {
        return QueryType::Tests;
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
            } else {
                let fuzzy_score = fuzzy_match(&line_lower, term);
                if fuzzy_score > 0.7 {
                    score += fuzzy_score;
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

fn fuzzy_match(text: &str, pattern: &str) -> f32 {
    if text.contains(pattern) {
        return 1.0;
    }

    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .collect();

    for word in &words {
        if word.len() >= pattern.len() {
            let distance = levenshtein_distance(word, pattern);
            let max_len = word.len().max(pattern.len());
            let similarity = 1.0 - (distance as f32 / max_len as f32);
            if similarity > 0.7 {
                return similarity;
            }
        }
    }

    0.0
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for (index, row) in matrix.iter_mut().enumerate().take(a_len + 1) {
        row[0] = index;
    }
    for (index, value) in matrix[0].iter_mut().enumerate().take(b_len + 1) {
        *value = index;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    matrix[a_len][b_len]
}

async fn find_panics(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("safety.md")).await?;

    println!("Panic Points (first {}):", limit);
    println!();

    let mut found = 0;
    let mut in_panics = false;

    for line in content.lines() {
        if line == "## Panic Points" {
            in_panics = true;
            continue;
        }

        if in_panics {
            if line.starts_with("## ") && line != "## Panic Points" {
                break;
            }

            if line.contains(":")
                && (line.contains(".unwrap()")
                    || line.contains(".expect(")
                    || line.contains("panic!")
                    || line.contains("L") && line.contains("in "))
            {
                println!("  {}", line.trim());
                found += 1;
                if found >= limit {
                    break;
                }
            } else if line.starts_with("Summary:")
                || line.starts_with("  .unwrap")
                || line.starts_with("  index")
                || line.starts_with("  panic")
                || line.starts_with("  assert")
            {
                println!("{}", line);
            }
        }
    }

    if found == 0 {
        println!("  No panic points found (run 'atlas' to generate safety.md)");
    }

    Ok(())
}

async fn find_panics_in(atlas_dir: &Path, file: &str, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("safety.md")).await?;

    println!("Panic Points in '{}':", file);
    println!();

    let file_lower = file.to_lowercase();
    let mut found = 0;

    for line in content.lines() {
        let matches_file = line.contains(&file_lower)
            || (line.contains(":") && line.to_lowercase().contains(&file_lower));
        let is_panic_line = line.contains(".unwrap()")
            || line.contains(".expect(")
            || line.contains("panic!")
            || line.contains(" in ");

        if matches_file && is_panic_line {
            println!("  {}", line.trim());
            found += 1;
            if found >= limit {
                break;
            }
        }
    }

    if found == 0 {
        println!("  No panic points found in '{}'", file);
    }

    Ok(())
}

async fn find_unsafe_code(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("safety.md")).await?;

    println!("Unsafe Code (first {}):", limit);
    println!();

    let mut found = 0;
    let mut in_unsafe = false;

    for line in content.lines() {
        if line == "## Unsafe Blocks" || line == "## Unsafe Code" {
            in_unsafe = true;
            continue;
        }

        if in_unsafe {
            if line.starts_with("## ") {
                break;
            }

            if !line.is_empty() && !line.starts_with('#') {
                println!("  {}", line.trim());
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    if found == 0 {
        println!("  No unsafe blocks found");
    }

    Ok(())
}

async fn find_async_functions(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("safety.md")).await?;

    println!("Async Analysis (first {}):", limit);
    println!();

    let mut found = 0;
    let mut in_async = false;

    for line in content.lines() {
        if line == "## Async Analysis" {
            in_async = true;
            continue;
        }

        if in_async {
            if line.starts_with("## ") && line != "## Async Analysis" {
                break;
            }

            if !line.is_empty() {
                println!("{}", line);
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    if found == 0 {
        println!("  No async analysis found");
    }

    Ok(())
}

async fn find_lifetimes(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("safety.md")).await?;

    println!("Lifetime Analysis (first {}):", limit);
    println!();

    let mut found = 0;
    let mut in_lifetimes = false;

    for line in content.lines() {
        if line == "## Lifetime Analysis" {
            in_lifetimes = true;
            continue;
        }

        if in_lifetimes {
            if line.starts_with("## ") && line != "## Lifetime Analysis" {
                break;
            }

            if !line.is_empty() {
                println!("{}", line);
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    if found == 0 {
        println!("  No lifetime information found");
    }

    Ok(())
}

async fn find_tests(atlas_dir: &Path, limit: usize) -> Result<()> {
    let content = fs::read_to_string(atlas_dir.join("safety.md")).await?;

    println!("Test Coverage (first {}):", limit);
    println!();

    let mut found = 0;
    let mut in_tests = false;

    for line in content.lines() {
        if line == "## Test Coverage" {
            in_tests = true;
            continue;
        }

        if in_tests {
            if line.starts_with("## ") && line != "## Test Coverage" {
                break;
            }

            if !line.is_empty() {
                println!("{}", line);
                found += 1;
                if found >= limit {
                    break;
                }
            }
        }
    }

    if found == 0 {
        println!("  No test coverage information found");
    }

    Ok(())
}
