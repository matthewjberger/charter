use anyhow::Result;
use std::path::Path;

use crate::extract::symbols::Visibility;
use crate::index::{Index, build_index};

pub async fn query(root: &Path, query_str: &str, limit: usize) -> Result<()> {
    let index = build_index(root).await?;

    let query_type = parse_query(query_str);

    match query_type {
        QueryType::CallersOf { target } => find_callers(&index, &target, limit),
        QueryType::CalleesOf { target } => find_callees(&index, &target, limit),
        QueryType::ImplementorsOf { trait_name } => find_implementors(&index, &trait_name, limit),
        QueryType::UsersOf { symbol } => find_users(&index, &symbol, limit),
        QueryType::ErrorsIn { file } => find_errors_in(&index, &file, limit),
        QueryType::Hotspots => find_hotspots(&index, limit),
        QueryType::PublicApi => find_public_api(&index, limit),
        QueryType::Panics => find_panics(&index, limit),
        QueryType::PanicsIn { file } => find_panics_in(&index, &file, limit),
        QueryType::UnsafeCode => find_unsafe_code(&index, limit),
        QueryType::AsyncFunctions => find_async_functions(&index, limit),
        QueryType::Lifetimes => find_lifetimes(&index, limit),
        QueryType::Tests => find_tests(&index, limit),
        QueryType::Keyword { terms } => keyword_search(&index, &terms, limit),
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

fn find_callers(index: &Index, target: &str, limit: usize) {
    println!("Callers of '{}':", target);
    println!();

    let mut callers = Vec::new();

    if let Some(caller_list) = index.reverse_calls.get(target) {
        callers.extend(caller_list.iter());
    }

    let suffix = format!("::{}", target);
    for (qualified_name, caller_list) in &index.reverse_calls {
        if qualified_name.ends_with(&suffix) && qualified_name.as_str() != target {
            callers.extend(caller_list.iter());
        }
    }

    callers.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.name.cmp(&b.name)));
    callers.dedup_by(|a, b| a.file == b.file && a.name == b.name && a.line == b.line);

    if callers.is_empty() {
        println!("  No callers found for '{}'", target);
    } else {
        for caller in callers.iter().take(limit) {
            let impl_suffix = caller
                .impl_type
                .as_deref()
                .map(|t| format!(" (impl {})", t))
                .unwrap_or_default();
            println!(
                "  {} ({}:{}){} calls {}",
                caller.name, caller.file, caller.line, impl_suffix, target
            );
        }
        println!();
        println!("Found {} caller(s)", callers.len().min(limit));
    }
}

fn find_callees(index: &Index, target: &str, limit: usize) {
    println!("Callees of '{}':", target);
    println!();

    let mut callees = Vec::new();

    if let Some(targets) = index.call_graph.get(target) {
        callees.extend(targets.iter());
    }

    let suffix = format!("::{}", target);
    for (qualified_name, targets) in &index.call_graph {
        if qualified_name.ends_with(&suffix) && qualified_name.as_str() != target {
            callees.extend(targets.iter());
        }
    }

    if callees.is_empty() {
        println!("  No callees found for '{}'", target);
    } else {
        for callee in callees.iter().take(limit) {
            let receiver = callee
                .receiver_type
                .as_deref()
                .map(|t| format!(" [on {}]", t))
                .unwrap_or_default();
            println!(
                "  {} → {}{} ({}:{})",
                target, callee.name, receiver, callee.file, callee.line
            );
        }
        println!();
        println!("Found {} callee(s)", callees.len().min(limit));
    }
}

fn find_implementors(index: &Index, trait_name: &str, limit: usize) {
    println!("Implementors of '{}':", trait_name);
    println!();

    let mut found = 0;

    if let Some(impls) = index.impl_map.get(trait_name) {
        for impl_info in impls.iter().take(limit) {
            println!(
                "  {} implements {} ({}:{})",
                impl_info.type_name, trait_name, impl_info.file, impl_info.line
            );
            found += 1;
        }
    }

    let trait_lower = trait_name.to_lowercase();
    if found == 0 {
        for (name, impls) in &index.impl_map {
            if name.to_lowercase().contains(&trait_lower) && name != trait_name {
                for impl_info in impls {
                    println!(
                        "  {} implements {} ({}:{})",
                        impl_info.type_name, name, impl_info.file, impl_info.line
                    );
                    found += 1;
                    if found >= limit {
                        break;
                    }
                }
            }
            if found >= limit {
                break;
            }
        }
    }

    if found == 0 {
        println!("  No implementors found for '{}'", trait_name);
    } else {
        println!();
        println!("Found {} trait implementation(s)", found);
    }
}

fn find_users(index: &Index, symbol: &str, limit: usize) {
    println!("References to '{}':", symbol);
    println!();

    let mut found = 0;

    if let Some(refs) = index.references.get(symbol) {
        for (file, line) in refs.iter().take(limit) {
            println!("  {}:{}", file, line);
            found += 1;
        }
    }

    if found == 0 {
        let symbol_lower = symbol.to_lowercase();
        let mut partial_matches = Vec::new();

        for name in index.symbols_by_name.keys() {
            if name.to_lowercase().contains(&symbol_lower) {
                partial_matches.push(name.as_str());
            }
        }

        if partial_matches.is_empty() {
            println!("  No references found for '{}'", symbol);
        } else {
            partial_matches.sort();
            println!("  No exact matches. Similar symbols:");
            for sym_match in partial_matches.iter().take(5) {
                println!("    {}", sym_match);
            }
        }
    } else {
        println!();
        println!("Found {} reference(s)", found);
    }
}

fn find_errors_in(index: &Index, file: &str, limit: usize) {
    println!("Errors in '{}':", file);
    println!();

    let file_lower = file.to_lowercase();
    let mut found = 0;

    for file_result in &index.result.files {
        if !file_result
            .relative_path
            .to_lowercase()
            .contains(&file_lower)
        {
            continue;
        }

        for error_info in &file_result.parsed.error_info {
            let func_name = error_info.function_id.qualified_name();
            println!(
                "  {} ({}:{}) → {:?}",
                func_name, file_result.relative_path, error_info.line, error_info.return_type
            );
            found += 1;
            if found >= limit {
                break;
            }
        }

        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No error patterns found in '{}'", file);
    }
}

fn find_hotspots(index: &Index, limit: usize) {
    println!("Top {} hotspots by complexity:", limit);
    println!();

    let mut hotspots: Vec<(String, String, usize, u32)> = Vec::new();

    for file in &index.result.files {
        for cx in &file.parsed.complexity {
            hotspots.push((
                cx.qualified_name(),
                file.relative_path.clone(),
                cx.line,
                cx.metrics.cyclomatic,
            ));
        }
    }

    hotspots.sort_by(|a, b| b.3.cmp(&a.3));

    if hotspots.is_empty() {
        println!("  No hotspots found");
    } else {
        for (name, file, line, cyclomatic) in hotspots.iter().take(limit) {
            println!("  {} [score={}] ({}:{})", name, cyclomatic, file, line);
        }
    }
}

fn find_public_api(index: &Index, limit: usize) {
    println!("Public API (first {} items):", limit);
    println!();

    let mut found = 0;

    for file in &index.result.files {
        for symbol in &file.parsed.symbols.symbols {
            if symbol.visibility == Visibility::Public {
                let kind = match &symbol.kind {
                    crate::extract::symbols::SymbolKind::Function { signature, .. } => {
                        format!("fn {}", signature)
                    }
                    crate::extract::symbols::SymbolKind::Struct { .. } => {
                        format!("struct {}", symbol.name)
                    }
                    crate::extract::symbols::SymbolKind::Enum { .. } => {
                        format!("enum {}", symbol.name)
                    }
                    crate::extract::symbols::SymbolKind::Trait { .. } => {
                        format!("trait {}", symbol.name)
                    }
                    _ => continue,
                };
                println!("  {} → pub {}", file.relative_path, kind);
                found += 1;
                if found >= limit {
                    return;
                }
            }
        }
    }

    if found == 0 {
        println!("  No public API found");
    }
}

fn find_panics(index: &Index, limit: usize) {
    println!("Panic Points (first {}):", limit);
    println!();

    let mut found = 0;

    for file in &index.result.files {
        for panic_point in &file.parsed.safety.panic_points {
            let func = panic_point
                .containing_function
                .as_deref()
                .unwrap_or("(top-level)");
            println!(
                "  {:?} in {} ({}:{})",
                panic_point.kind, func, file.relative_path, panic_point.line
            );
            found += 1;
            if found >= limit {
                break;
            }
        }
        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No panic points found");
    }
}

fn find_panics_in(index: &Index, file: &str, limit: usize) {
    println!("Panic Points in '{}':", file);
    println!();

    let file_lower = file.to_lowercase();
    let mut found = 0;

    for file_result in &index.result.files {
        if !file_result
            .relative_path
            .to_lowercase()
            .contains(&file_lower)
        {
            continue;
        }

        for panic_point in &file_result.parsed.safety.panic_points {
            let func = panic_point
                .containing_function
                .as_deref()
                .unwrap_or("(top-level)");
            println!(
                "  {:?} in {} ({}:{})",
                panic_point.kind, func, file_result.relative_path, panic_point.line
            );
            found += 1;
            if found >= limit {
                break;
            }
        }

        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No panic points found in '{}'", file);
    }
}

fn find_unsafe_code(index: &Index, limit: usize) {
    println!("Unsafe Code (first {}):", limit);
    println!();

    let mut found = 0;

    for file in &index.result.files {
        for unsafe_block in &file.parsed.safety.unsafe_blocks {
            let func = unsafe_block
                .containing_function
                .as_deref()
                .unwrap_or("(top-level)");
            let ops: Vec<String> = unsafe_block
                .operations
                .iter()
                .map(|op| format!("{:?}", op))
                .collect();
            println!(
                "  {} in {} ({}:{}) [{}]",
                func,
                file.relative_path,
                unsafe_block.line,
                file.relative_path,
                ops.join(", ")
            );
            found += 1;
            if found >= limit {
                break;
            }
        }
        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No unsafe blocks found");
    }
}

fn find_async_functions(index: &Index, limit: usize) {
    println!("Async Analysis (first {}):", limit);
    println!();

    let mut found = 0;

    for file in &index.result.files {
        for async_fn in &file.parsed.async_info.async_functions {
            let qualified = match &async_fn.impl_type {
                Some(t) => format!("{}::{}", t, async_fn.name),
                None => async_fn.name.clone(),
            };
            println!(
                "  async {} ({}:{}) [{} awaits, {} spawns]",
                qualified,
                file.relative_path,
                async_fn.line,
                async_fn.awaits.len(),
                async_fn.spawns.len()
            );
            found += 1;
            if found >= limit {
                break;
            }
        }
        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No async functions found");
    }
}

fn find_lifetimes(index: &Index, limit: usize) {
    println!("Lifetime Analysis (first {}):", limit);
    println!();

    let mut found = 0;

    for file in &index.result.files {
        for func_lt in &file.parsed.lifetimes.function_lifetimes {
            let qualified = match &func_lt.impl_type {
                Some(t) => format!("{}::{}", t, func_lt.function_name),
                None => func_lt.function_name.clone(),
            };
            println!(
                "  {} ({}:{}) lifetimes: [{}]{}",
                qualified,
                file.relative_path,
                func_lt.line,
                func_lt.lifetimes.join(", "),
                if func_lt.has_static {
                    " (has 'static)"
                } else {
                    ""
                }
            );
            found += 1;
            if found >= limit {
                break;
            }
        }
        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No lifetime information found");
    }
}

fn find_tests(index: &Index, limit: usize) {
    println!("Test Coverage (first {}):", limit);
    println!();

    let mut found = 0;

    for file in &index.result.files {
        if file.parsed.test_functions.is_empty() {
            continue;
        }

        println!(
            "  {} [{} tests]{}",
            file.relative_path,
            file.parsed.test_functions.len(),
            if file.parsed.has_test_module {
                " (has #[cfg(test)] module)"
            } else {
                ""
            }
        );

        for test_fn in &file.parsed.test_functions {
            println!("    {}", test_fn);
            found += 1;
            if found >= limit {
                break;
            }
        }

        if found >= limit {
            break;
        }
    }

    if found == 0 {
        println!("  No test coverage information found");
    }
}

fn keyword_search(index: &Index, terms: &[String], limit: usize) {
    println!("Search results for '{}':", terms.join(" "));
    println!();

    let mut results: Vec<(String, f32)> = Vec::new();

    for (name, syms) in &index.symbols_by_name {
        let name_lower = name.to_lowercase();
        let mut score = 0.0f32;

        for term in terms {
            if name_lower.contains(term) {
                score += 1.0;
                if name_lower.starts_with(term) {
                    score += 0.5;
                }
            } else {
                let fuzzy = fuzzy_score(&name_lower, term);
                if fuzzy > 0.7 {
                    score += fuzzy;
                }
            }
        }

        if score > 0.0 {
            for sym in syms {
                results.push((
                    format!(
                        "[{}] {} {} ({}:{})",
                        sym.kind, sym.visibility, name, sym.file, sym.line
                    ),
                    score,
                ));
            }
        }
    }

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.dedup_by(|a, b| a.0 == b.0);

    if results.is_empty() {
        println!("  No results found");
    } else {
        for (line, score) in results.iter().take(limit) {
            println!("  [relevance={:.1}] {}", score, line);
        }
        println!();
        println!("Found {} result(s)", results.len().min(limit));
    }
}

fn fuzzy_score(text: &str, pattern: &str) -> f32 {
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
