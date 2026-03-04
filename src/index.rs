use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

use crate::cache::Cache;
use crate::detect::detect_workspace;
use crate::extract::symbols::{FunctionBody, Symbol, SymbolKind};
use crate::pipeline::{self, PipelineResult, walk};

pub struct Index {
    pub result: PipelineResult,
    pub symbol_table: HashMap<String, (String, usize)>,
    pub references: HashMap<String, Vec<(String, usize)>>,
    pub symbols_by_name: HashMap<String, Vec<SymbolInfo>>,
    pub impl_map: HashMap<String, Vec<ImplInfo>>,
    pub reverse_impl_map: HashMap<String, Vec<String>>,
    pub call_graph: HashMap<String, Vec<CallTarget>>,
    pub reverse_calls: HashMap<String, Vec<CallerInfo>>,
    pub derive_map: HashMap<String, Vec<String>>,
    pub snippets_by_name: HashMap<String, Vec<SnippetInfo>>,
    pub imports_by_file: HashMap<String, Vec<FileImport>>,
    pub imports_by_symbol: HashMap<String, Vec<ImportLocation>>,
    pub external_symbols: HashMap<String, Vec<ExternalSymbolInfo>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImplInfo {
    pub type_name: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallTarget {
    pub name: String,
    pub receiver_type: Option<String>,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallerInfo {
    pub name: String,
    pub impl_type: Option<String>,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnippetInfo {
    pub function_name: String,
    pub impl_type: Option<String>,
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub body: String,
    pub importance_score: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub signature: Option<String>,
    pub visibility: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalSymbolInfo {
    pub name: String,
    pub kind: String,
    pub crate_name: String,
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileImport {
    pub path: String,
    pub line: usize,
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportLocation {
    pub file: String,
    pub source_path: String,
    pub line: usize,
}

impl Index {
    pub fn new(
        result: PipelineResult,
        symbol_table: HashMap<String, (String, usize)>,
        references: HashMap<String, Vec<(String, usize)>>,
    ) -> Self {
        let mut symbols_by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut impl_map: HashMap<String, Vec<ImplInfo>> = HashMap::new();
        let mut reverse_impl_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut call_graph: HashMap<String, Vec<CallTarget>> = HashMap::new();
        let mut reverse_calls: HashMap<String, Vec<CallerInfo>> = HashMap::new();
        let mut derive_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut snippets_by_name: HashMap<String, Vec<SnippetInfo>> = HashMap::new();
        let mut imports_by_file: HashMap<String, Vec<FileImport>> = HashMap::new();
        let mut imports_by_symbol: HashMap<String, Vec<ImportLocation>> = HashMap::new();
        let external_symbols: HashMap<String, Vec<ExternalSymbolInfo>> = HashMap::new();

        for file in &result.files {
            for symbol in &file.parsed.symbols.symbols {
                let info = symbol_to_info(symbol, &file.relative_path);
                symbols_by_name
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(info);
            }

            for (trait_name, type_name) in &file.parsed.symbols.impl_map {
                let impl_info = ImplInfo {
                    type_name: type_name.clone(),
                    file: file.relative_path.clone(),
                    line: find_impl_line(&file.parsed.symbols.inherent_impls, type_name)
                        .unwrap_or(0),
                };
                impl_map
                    .entry(trait_name.clone())
                    .or_default()
                    .push(impl_info);
                reverse_impl_map
                    .entry(type_name.clone())
                    .or_default()
                    .push(trait_name.clone());
            }

            for inherent_impl in &file.parsed.symbols.inherent_impls {
                for method in &inherent_impl.methods {
                    let qualified = format!("{}::{}", inherent_impl.type_name, method.name);
                    let info = SymbolInfo {
                        name: method.name.clone(),
                        kind: "method".to_string(),
                        file: file.relative_path.clone(),
                        line: method.line,
                        signature: Some(method.signature.clone()),
                        visibility: format!("{}", method.visibility),
                    };
                    symbols_by_name
                        .entry(qualified)
                        .or_default()
                        .push(info.clone());
                    symbols_by_name
                        .entry(method.name.clone())
                        .or_default()
                        .push(info);
                }
            }

            let inherent_method_keys: std::collections::HashSet<String> = file
                .parsed
                .symbols
                .inherent_impls
                .iter()
                .flat_map(|imp| {
                    imp.methods
                        .iter()
                        .map(move |m| format!("{}::{}", imp.type_name, m.name))
                })
                .collect();

            for complexity_entry in &file.parsed.complexity {
                if let Some(ref parent_type) = complexity_entry.impl_type {
                    let qualified = format!("{}::{}", parent_type, complexity_entry.name);
                    if inherent_method_keys.contains(&qualified) {
                        continue;
                    }
                    let info = SymbolInfo {
                        name: complexity_entry.name.clone(),
                        kind: "method".to_string(),
                        file: file.relative_path.clone(),
                        line: complexity_entry.line,
                        signature: None,
                        visibility: if complexity_entry.metrics.is_public {
                            "pub".to_string()
                        } else {
                            "private".to_string()
                        },
                    };
                    symbols_by_name
                        .entry(qualified)
                        .or_default()
                        .push(info.clone());
                    symbols_by_name
                        .entry(complexity_entry.name.clone())
                        .or_default()
                        .push(info);
                }
            }

            for call_info in &file.parsed.call_graph {
                let caller = call_info.caller.qualified_name();
                let bare_caller = call_info.caller.name.clone();
                let caller_impl_type = call_info.caller.impl_type.clone();
                let caller_line = call_info.line;
                for callee in &call_info.callees {
                    let resolved_type = match (&callee.target_type, &caller_impl_type) {
                        (Some(t), Some(concrete)) if t == "Self" => Some(concrete.clone()),
                        (other, _) => other.clone(),
                    };
                    let callee_name = match &resolved_type {
                        Some(type_name) => format!("{}::{}", type_name, callee.target),
                        None => callee.target.clone(),
                    };
                    let target = CallTarget {
                        name: callee_name.clone(),
                        receiver_type: resolved_type,
                        file: file.relative_path.clone(),
                        line: callee.line,
                    };
                    call_graph
                        .entry(caller.clone())
                        .or_default()
                        .push(target.clone());
                    if bare_caller != caller {
                        call_graph
                            .entry(bare_caller.clone())
                            .or_default()
                            .push(target);
                    }
                    let caller_info = CallerInfo {
                        name: caller.clone(),
                        impl_type: caller_impl_type.clone(),
                        file: file.relative_path.clone(),
                        line: caller_line,
                    };
                    reverse_calls
                        .entry(callee_name.clone())
                        .or_default()
                        .push(caller_info.clone());
                    if callee_name != callee.target {
                        reverse_calls
                            .entry(callee.target.clone())
                            .or_default()
                            .push(caller_info);
                    }
                }
            }

            for derive in &file.parsed.derives {
                derive_map
                    .entry(derive.target.clone())
                    .or_default()
                    .extend(derive.traits.clone());
            }

            for captured in &file.parsed.captured_bodies {
                let key = if let Some(ref impl_type) = captured.impl_type {
                    format!("{}::{}", impl_type, captured.function_name)
                } else {
                    captured.function_name.clone()
                };
                let body_text = body_text_from_captured(&captured.body);
                let end_line = compute_end_line(captured.line, &captured.body);
                let hint = if captured.body.full_text.is_none() {
                    Some(format!(
                        "Use read_source(file=\"{}\", start_line={}, end_line={}) for full source",
                        file.relative_path, captured.line, end_line,
                    ))
                } else {
                    None
                };
                let snippet = SnippetInfo {
                    function_name: captured.function_name.clone(),
                    impl_type: captured.impl_type.clone(),
                    file: file.relative_path.clone(),
                    line: captured.line,
                    end_line,
                    body: body_text,
                    importance_score: captured.importance_score,
                    hint,
                };
                snippets_by_name
                    .entry(key.clone())
                    .or_default()
                    .push(snippet.clone());
                if captured.impl_type.is_some() {
                    snippets_by_name
                        .entry(captured.function_name.clone())
                        .or_default()
                        .push(snippet);
                }
            }

            for import in &file.parsed.imports {
                let symbols = parse_import_symbols(&import.path);
                let file_import = FileImport {
                    path: import.path.clone(),
                    line: import.line,
                    symbols: symbols.clone(),
                };
                imports_by_file
                    .entry(file.relative_path.clone())
                    .or_default()
                    .push(file_import);
                for symbol in symbols {
                    imports_by_symbol
                        .entry(symbol)
                        .or_default()
                        .push(ImportLocation {
                            file: file.relative_path.clone(),
                            source_path: import.path.clone(),
                            line: import.line,
                        });
                }
            }
        }

        for traits in derive_map.values_mut() {
            traits.sort();
            traits.dedup();
        }

        for callers in reverse_calls.values_mut() {
            callers.sort_by(|a, b| {
                a.file
                    .cmp(&b.file)
                    .then_with(|| a.name.cmp(&b.name))
                    .then_with(|| a.line.cmp(&b.line))
            });
            callers.dedup_by(|a, b| a.file == b.file && a.name == b.name && a.line == b.line);
        }

        Self {
            result,
            symbol_table,
            references,
            symbols_by_name,
            impl_map,
            reverse_impl_map,
            call_graph,
            reverse_calls,
            derive_map,
            snippets_by_name,
            imports_by_file,
            imports_by_symbol,
            external_symbols,
        }
    }
}

fn find_impl_line(
    inherent_impls: &[crate::extract::symbols::InherentImpl],
    type_name: &str,
) -> Option<usize> {
    for inherent_impl in inherent_impls {
        if inherent_impl.type_name == type_name {
            return inherent_impl.methods.first().map(|m| m.line);
        }
    }
    None
}

fn symbol_to_info(symbol: &Symbol, file: &str) -> SymbolInfo {
    let (kind, signature) = match &symbol.kind {
        SymbolKind::Struct { .. } => ("struct".to_string(), None),
        SymbolKind::Enum { .. } => ("enum".to_string(), None),
        SymbolKind::Trait { .. } => ("trait".to_string(), None),
        SymbolKind::Function { signature, .. } => ("function".to_string(), Some(signature.clone())),
        SymbolKind::Const { const_type, .. } => ("const".to_string(), Some(const_type.clone())),
        SymbolKind::Static { static_type, .. } => ("static".to_string(), Some(static_type.clone())),
        SymbolKind::TypeAlias { aliased_type } => ("type".to_string(), Some(aliased_type.clone())),
        SymbolKind::Mod => ("mod".to_string(), None),
        SymbolKind::Class { .. } => ("class".to_string(), None),
        SymbolKind::PythonFunction { .. } => ("function".to_string(), None),
        SymbolKind::Variable { type_hint, .. } => ("variable".to_string(), type_hint.clone()),
        SymbolKind::PythonModule => ("module".to_string(), None),
    };

    SymbolInfo {
        name: symbol.name.clone(),
        kind,
        file: file.to_string(),
        line: symbol.line,
        signature,
        visibility: format!("{}", symbol.visibility),
    }
}

fn compute_end_line(start_line: usize, body: &FunctionBody) -> usize {
    if let Some(ref text) = body.full_text {
        return start_line + text.lines().count().saturating_sub(1);
    }
    if let Some(ref summary) = body.summary {
        return start_line + summary.line_count.saturating_sub(1);
    }
    start_line
}

fn body_text_from_captured(body: &FunctionBody) -> String {
    if let Some(ref text) = body.full_text {
        return text.clone();
    }
    if let Some(ref summary) = body.summary {
        let mut parts = Vec::new();
        parts.push(format!(
            "{} lines, {} statements",
            summary.line_count, summary.statement_count
        ));
        if !summary.key_calls.is_empty() {
            parts.push(format!("calls: {}", summary.key_calls.join(", ")));
        }
        if !summary.early_returns.is_empty() {
            parts.push(format!("returns: {}", summary.early_returns.join(", ")));
        }
        return parts.join("; ");
    }
    "[body not captured]".to_string()
}

fn parse_import_symbols(path: &str) -> Vec<String> {
    let path = path.trim_start_matches("use ");
    let path = path.trim_end_matches(';');

    if let Some(brace_start) = path.find('{') {
        if let Some(brace_end) = path.rfind('}') {
            let inner = &path[brace_start + 1..brace_end];
            return inner
                .split(',')
                .filter_map(|segment| {
                    let segment = segment.trim();
                    if segment.is_empty() {
                        return None;
                    }
                    if segment.contains('{') {
                        let nested = parse_import_symbols(segment);
                        return Some(nested);
                    }
                    let name = if let Some(alias_pos) = segment.find(" as ") {
                        &segment[alias_pos + 4..]
                    } else {
                        segment.rsplit("::").next().unwrap_or(segment)
                    };
                    let name = name.trim();
                    if name.is_empty() || name == "*" || name == "self" {
                        None
                    } else {
                        Some(vec![name.to_string()])
                    }
                })
                .flatten()
                .collect();
        }
    }

    if let Some(alias_pos) = path.find(" as ") {
        let name = path[alias_pos + 4..].trim();
        if !name.is_empty() {
            return vec![name.to_string()];
        }
    }

    if path.ends_with("::*") {
        return vec![format!("{}::*", path.trim_end_matches("::*"))];
    }

    let last_segment = path.rsplit("::").next().unwrap_or(path).trim();
    if last_segment.is_empty() || last_segment == "self" {
        return Vec::new();
    }
    vec![last_segment.to_string()]
}

pub async fn build_index(root: &Path) -> Result<Index> {
    let workspace = detect_workspace(root).await?;
    let cache_path = root.join(".charter/cache.bin");
    let cache = Cache::load(&cache_path).await.unwrap_or_default();
    let walk_result = walk::walk_directory(root).await?;
    let result =
        pipeline::run_phase1_with_walk(root, &workspace, &cache, None, walk_result).await?;
    let symbol_table = pipeline::build_symbol_table(&result.files);
    let references = pipeline::run_phase2(&result.files, &symbol_table);
    Ok(Index::new(result, symbol_table, references))
}
