use anyhow::Result;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    schemars::JsonSchema,
    service::ServiceExt,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::Cache;
use crate::detect::detect_workspace;
use crate::external;
use crate::extract::symbols::{SymbolKind, Visibility};
use crate::index::{
    CallTarget, CallerInfo, ExternalSymbolInfo, FileImport, ImplInfo, ImportLocation, Index,
    SnippetInfo,
};
use crate::pipeline::{self, FileResult, walk};

#[derive(Clone)]
pub struct CharterServer {
    index: Arc<RwLock<Index>>,
    root: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FindSymbolParams {
    pub name: String,
    pub kind: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FindImplementationsParams {
    pub symbol: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FindCallersParams {
    pub symbol: String,
    #[serde(default)]
    pub depth: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FindDependenciesParams {
    pub symbol: String,
    pub direction: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FindReferencesParams {
    pub symbol: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetModuleTreeParams {
    pub root: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetTypeHierarchyParams {
    pub symbol: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SummarizeParams {
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchSymbolsParams {
    pub query: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub regex: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetSnippetParams {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetImportsParams {
    pub file: Option<String>,
    pub symbol: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GetImportsResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_imports: Option<Vec<FileImport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_locations: Option<Vec<ImportLocation>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetDefinitionParams {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct DefinitionResult {
    pub definitions: Vec<DefinitionInfo>,
}

#[derive(Debug, Serialize)]
pub struct DefinitionInfo {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub visibility: String,
    pub generics: String,
    pub definition: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadSourceParams {
    pub file: String,
    pub start_line: usize,
    #[serde(default)]
    pub end_line: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ReadSourceResult {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub language: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchTextParams {
    pub pattern: String,
    pub glob: Option<String>,
    pub case_insensitive: Option<bool>,
    pub context_lines: Option<usize>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchTextResult {
    pub matches: Vec<TextMatch>,
    pub files_searched: usize,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct TextMatch {
    pub file: String,
    pub line: usize,
    pub text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

struct TextSearchSink {
    matches: Vec<TextMatch>,
    current_before: Vec<String>,
    current_after_target: Option<usize>,
    max_results: usize,
    file: String,
}

impl TextSearchSink {
    fn new(file: String, max_results: usize) -> Self {
        Self {
            matches: Vec::new(),
            current_before: Vec::new(),
            current_after_target: None,
            max_results,
            file,
        }
    }

    fn is_full(&self) -> bool {
        self.matches.len() >= self.max_results
    }
}

impl Sink for TextSearchSink {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        mat: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        if self.is_full() {
            return Ok(false);
        }

        let line_number = mat.line_number().unwrap_or(0) as usize;
        let text = String::from_utf8_lossy(mat.bytes()).trim_end().to_string();

        self.current_after_target = None;

        let before = std::mem::take(&mut self.current_before);

        self.matches.push(TextMatch {
            file: self.file.clone(),
            line: line_number,
            text,
            context_before: before,
            context_after: Vec::new(),
        });

        self.current_after_target = Some(self.matches.len() - 1);

        Ok(true)
    }

    fn context(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        let line_text = String::from_utf8_lossy(context.bytes())
            .trim_end()
            .to_string();

        match context.kind() {
            SinkContextKind::Before => {
                self.current_before.push(line_text);
            }
            SinkContextKind::After => {
                if let Some(index) = self.current_after_target {
                    if let Some(m) = self.matches.get_mut(index) {
                        m.context_after.push(line_text);
                    }
                }
            }
            _ => {}
        }

        Ok(!self.is_full())
    }

    fn context_break(&mut self, _searcher: &grep_searcher::Searcher) -> Result<bool, Self::Error> {
        self.current_before.clear();
        self.current_after_target = None;
        Ok(!self.is_full())
    }
}

#[derive(Debug, Serialize)]
pub struct SymbolResult {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    pub visibility: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crate_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub symbols: Vec<SymbolResult>,
    pub traits: Vec<TraitResult>,
    pub calls: Vec<CallResult>,
}

#[derive(Debug, Serialize)]
pub struct TraitResult {
    pub trait_name: String,
    pub implementors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CallResult {
    pub caller: String,
    pub callees: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImplementationsResult {
    pub trait_implementors: Vec<ImplInfo>,
    pub type_implements: Vec<String>,
    pub methods: Vec<MethodResult>,
    pub derived_traits: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MethodResult {
    pub name: String,
    pub signature: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct CallersResult {
    pub callers: Vec<CallerWithDepth>,
}

#[derive(Debug, Serialize)]
pub struct CallerWithDepth {
    pub name: String,
    pub impl_type: Option<String>,
    pub file: String,
    pub line: usize,
    pub depth: usize,
}

#[derive(Debug, Serialize)]
pub struct DependenciesResult {
    pub upstream: Vec<CallTarget>,
    pub downstream: Vec<CallerInfo>,
    pub references: Vec<ReferenceInfo>,
}

#[derive(Debug, Serialize)]
pub struct ReferenceInfo {
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct ModuleTreeResult {
    pub modules: Vec<ModuleInfo>,
}

#[derive(Debug, Serialize)]
pub struct ModuleInfo {
    pub path: String,
    pub symbol_count: usize,
}

#[derive(Debug, Serialize)]
pub struct TypeHierarchyResult {
    pub implementors: Vec<ImplInfo>,
    pub implements: Vec<String>,
    pub derived_traits: Vec<String>,
    pub supertraits: Vec<String>,
    pub base_classes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SummaryResult {
    pub files: usize,
    pub lines: usize,
    pub symbols: usize,
    pub git_commit: Option<String>,
    pub by_kind: KindCounts,
    pub visibility: VisibilityCounts,
    pub high_complexity: Vec<ComplexityInfo>,
}

#[derive(Debug, Serialize)]
pub struct KindCounts {
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub functions: usize,
    pub classes: usize,
}

#[derive(Debug, Serialize)]
pub struct VisibilityCounts {
    pub public: usize,
    pub private: usize,
}

#[derive(Debug, Serialize)]
pub struct ComplexityInfo {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub cyclomatic: u32,
}

#[derive(Debug, Serialize)]
pub struct RescanResult {
    pub old_file_count: usize,
    pub new_file_count: usize,
    pub cache_persisted: bool,
}

#[derive(Debug, Serialize)]
pub struct FindReferencesResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<ReferenceInfo>,
    pub references: Vec<ReferenceInfo>,
}

#[derive(Debug, Serialize)]
pub struct SnippetResult {
    pub snippets: Vec<SnippetInfo>,
}

#[tool_router]
impl CharterServer {
    fn new(index: Arc<RwLock<Index>>, root: PathBuf) -> Self {
        Self {
            index,
            root,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search for symbols by name (fuzzy/partial matching supported). Filter by kind and limit results. Set regex=true to match symbol names against a regex pattern."
    )]
    async fn search_symbols(
        &self,
        params: Parameters<SearchSymbolsParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let limit = params.0.limit.unwrap_or(50);
        let use_regex = params.0.regex.unwrap_or(false);

        let compiled_regex = if use_regex {
            match regex::RegexBuilder::new(&params.0.query)
                .case_insensitive(true)
                .build()
            {
                Ok(re) => Some(re),
                Err(error) => {
                    return Ok(format!(
                        "{{\"error\": \"Invalid regex pattern: {}\"}}",
                        error
                    ));
                }
            }
        } else {
            None
        };

        let query = params.0.query.to_lowercase();

        let mut scored_symbols: Vec<(u32, SymbolResult)> = Vec::new();
        let mut traits = Vec::new();
        let mut calls = Vec::new();

        for (name, syms) in &index.symbols_by_name {
            let score = if let Some(ref re) = compiled_regex {
                if re.is_match(name) { 100 } else { 0 }
            } else {
                let name_lower = name.to_lowercase();
                let bare_lower = bare_name(&name_lower);
                symbol_match_score(&query, &name_lower, bare_lower)
            };
            if score == 0 {
                continue;
            }
            for sym in syms {
                if let Some(ref kind_filter) = params.0.kind {
                    if &sym.kind != kind_filter {
                        continue;
                    }
                }
                let kind_bonus = match sym.kind.as_str() {
                    "struct" | "enum" | "trait" => 10,
                    "function" => 5,
                    _ => 0,
                };
                scored_symbols.push((
                    score + kind_bonus,
                    SymbolResult {
                        name: name.clone(),
                        kind: sym.kind.clone(),
                        file: sym.file.clone(),
                        line: sym.line,
                        signature: sym.signature.clone(),
                        visibility: sym.visibility.clone(),
                        crate_name: None,
                    },
                ));
            }
        }

        for (name, ext_syms) in &index.external_symbols {
            let score = if let Some(ref re) = compiled_regex {
                if re.is_match(name) { 50 } else { 0 }
            } else {
                let name_lower = name.to_lowercase();
                let bare_lower = bare_name(&name_lower);
                let raw = symbol_match_score(&query, &name_lower, bare_lower);
                raw / 2
            };
            if score == 0 {
                continue;
            }
            for sym in ext_syms {
                if let Some(ref kind_filter) = params.0.kind {
                    if &sym.kind != kind_filter {
                        continue;
                    }
                }
                scored_symbols.push((
                    score,
                    SymbolResult {
                        name: name.clone(),
                        kind: sym.kind.clone(),
                        file: format!("[{}] {}", sym.crate_name, sym.file),
                        line: sym.line,
                        signature: sym.signature.clone(),
                        visibility: "pub".to_string(),
                        crate_name: Some(sym.crate_name.clone()),
                    },
                ));
            }
        }

        scored_symbols.sort_by(|a, b| b.0.cmp(&a.0));
        scored_symbols
            .dedup_by(|a, b| a.1.name == b.1.name && a.1.file == b.1.file && a.1.line == b.1.line);
        let symbols: Vec<SymbolResult> = scored_symbols
            .into_iter()
            .take(limit)
            .map(|(_, sym)| sym)
            .collect();

        if compiled_regex.is_none() {
            for (trait_name, impls) in &index.impl_map {
                let trait_lower = trait_name.to_lowercase();
                let bare = bare_name(&trait_lower);
                if bare == query || trait_lower == query || bare.contains(&query) {
                    traits.push(TraitResult {
                        trait_name: trait_name.clone(),
                        implementors: impls.iter().map(|i| i.type_name.clone()).collect(),
                    });
                }
            }

            for (caller, targets) in &index.call_graph {
                let caller_lower = caller.to_lowercase();
                let bare = bare_name(&caller_lower);
                if bare == query || caller_lower == query || bare.contains(&query) {
                    calls.push(CallResult {
                        caller: caller.clone(),
                        callees: targets.iter().map(|t| t.name.clone()).collect(),
                    });
                }
            }

            traits.truncate(limit / 2);
            calls.truncate(limit / 2);
        }

        let result = SearchResult {
            symbols,
            traits,
            calls,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Find a symbol by exact name or fuzzy match. Filter by kind (struct, enum, trait, function, method, etc.)"
    )]
    async fn find_symbol(&self, params: Parameters<FindSymbolParams>) -> Result<String, McpError> {
        let index = self.index.read().await;
        let query_lower = params.0.name.to_lowercase();
        let mut scored: Vec<(u32, SymbolResult)> = Vec::new();

        for (name, symbols) in &index.symbols_by_name {
            let name_lower = name.to_lowercase();
            let bare_lower = bare_name(&name_lower);
            let score = symbol_match_score(&query_lower, &name_lower, bare_lower);
            if score == 0 {
                continue;
            }
            for sym in symbols {
                if let Some(ref kind_filter) = params.0.kind {
                    if &sym.kind != kind_filter {
                        continue;
                    }
                }
                let kind_bonus = match sym.kind.as_str() {
                    "struct" | "enum" | "trait" => 10,
                    "function" => 5,
                    _ => 0,
                };
                scored.push((
                    score + kind_bonus,
                    SymbolResult {
                        name: name.clone(),
                        kind: sym.kind.clone(),
                        file: sym.file.clone(),
                        line: sym.line,
                        signature: sym.signature.clone(),
                        visibility: sym.visibility.clone(),
                        crate_name: None,
                    },
                ));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .dedup_by(|a, b| a.1.name == b.1.name && a.1.file == b.1.file && a.1.line == b.1.line);
        let results: Vec<SymbolResult> = scored.into_iter().take(20).map(|(_, sym)| sym).collect();

        Ok(serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string()))
    }

    #[tool(
        description = "Find implementations of a trait or methods on a type. Includes derive-generated impls."
    )]
    async fn find_implementations(
        &self,
        params: Parameters<FindImplementationsParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let symbol = &params.0.symbol;
        let suffix = format!("::{}", symbol);

        let mut trait_implementors: Vec<ImplInfo> =
            index.impl_map.get(symbol).cloned().unwrap_or_default();
        let suffix_generic = format!("::{}", symbol);
        for (key, impls) in &index.impl_map {
            if key == symbol {
                continue;
            }
            let stripped = strip_generics(key);
            if stripped == symbol
                || key.ends_with(&suffix_generic)
                || stripped.ends_with(&suffix_generic)
            {
                trait_implementors.extend(impls.clone());
            }
        }

        let mut type_implements: Vec<String> = index
            .reverse_impl_map
            .get(symbol)
            .cloned()
            .unwrap_or_default();
        for (key, traits) in &index.reverse_impl_map {
            if key != symbol && (key.ends_with(&suffix) || strip_generics(key) == symbol) {
                type_implements.extend(traits.clone());
            }
        }

        let mut derived_traits: Vec<String> =
            index.derive_map.get(symbol).cloned().unwrap_or_default();
        for (key, traits) in &index.derive_map {
            if key != symbol && (key.ends_with(&suffix) || strip_generics(key) == symbol) {
                derived_traits.extend(traits.clone());
            }
        }

        let mut methods = Vec::new();
        for file in &index.result.files {
            for inherent_impl in &file.parsed.symbols.inherent_impls {
                if inherent_impl.type_name == *symbol || inherent_impl.type_name.ends_with(&suffix)
                {
                    for method in &inherent_impl.methods {
                        methods.push(MethodResult {
                            name: method.name.clone(),
                            signature: method.signature.clone(),
                            file: file.relative_path.clone(),
                            line: method.line,
                        });
                    }
                }
            }
        }
        methods.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.file.cmp(&b.file)));
        methods.dedup_by(|a, b| a.name == b.name && a.file == b.file && a.line == b.line);

        trait_implementors.dedup_by(|a, b| a.type_name == b.type_name && a.file == b.file);
        type_implements.sort();
        type_implements.dedup();
        derived_traits.sort();
        derived_traits.dedup();

        let result = ImplementationsResult {
            trait_implementors,
            type_implements,
            methods,
            derived_traits,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Find all references to a type or symbol across the codebase. Returns definition location and all usage sites with file:line."
    )]
    async fn find_references(
        &self,
        params: Parameters<FindReferencesParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let symbol = &params.0.symbol;
        let suffix = format!("::{}", symbol);

        let definition = index
            .symbol_table
            .get(symbol)
            .map(|(file, line)| ReferenceInfo {
                file: file.clone(),
                line: *line,
            });

        let mut refs: Vec<ReferenceInfo> = Vec::new();

        if let Some(ref_list) = index.references.get(symbol) {
            refs.extend(ref_list.iter().map(|(file, line)| ReferenceInfo {
                file: file.clone(),
                line: *line,
            }));
        }

        for (key, ref_list) in &index.references {
            if key != symbol && key.ends_with(&suffix) {
                refs.extend(ref_list.iter().map(|(file, line)| ReferenceInfo {
                    file: file.clone(),
                    line: *line,
                }));
            }
        }

        if refs.is_empty() {
            let query_words = split_pascal_case(symbol);
            if query_words.len() >= 2 {
                for (key, ref_list) in &index.references {
                    let key_words = split_pascal_case(key);
                    if pascal_words_match(&query_words, &key_words) {
                        refs.extend(ref_list.iter().map(|(file, line)| ReferenceInfo {
                            file: file.clone(),
                            line: *line,
                        }));
                    }
                }
            }
        }

        if refs.is_empty() && !crate::pipeline::is_pascal_case(symbol) {
            if let Some(caller_list) = index.reverse_calls.get(symbol) {
                refs.extend(caller_list.iter().map(|caller| ReferenceInfo {
                    file: caller.file.clone(),
                    line: caller.line,
                }));
            }
            for (key, caller_list) in &index.reverse_calls {
                if key != symbol && key.ends_with(&suffix) {
                    refs.extend(caller_list.iter().map(|caller| ReferenceInfo {
                        file: caller.file.clone(),
                        line: caller.line,
                    }));
                }
            }
        }

        refs.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
        refs.dedup_by(|a, b| a.file == b.file && a.line == b.line);

        let result = FindReferencesResult {
            definition,
            references: refs,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Find all call sites of a function or method. Optional depth (1-3) for transitive callers. Returns caller name, file, line, and depth level."
    )]
    async fn find_callers(
        &self,
        params: Parameters<FindCallersParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let max_depth = params.0.depth.unwrap_or(1).clamp(1, 3);
        let mut all_callers: Vec<CallerWithDepth> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String, usize)> =
            std::collections::HashSet::new();
        let mut current_symbols = vec![params.0.symbol.clone()];

        for depth in 1..=max_depth {
            let mut next_symbols = Vec::new();
            for symbol in &current_symbols {
                let suffix = format!("::{}", symbol);
                let mut depth_callers: Vec<CallerInfo> = Vec::new();

                if let Some(caller_list) = index.reverse_calls.get(symbol) {
                    depth_callers.extend(caller_list.clone());
                }
                for (qualified_name, caller_list) in &index.reverse_calls {
                    if qualified_name.ends_with(&suffix) && qualified_name.as_str() != symbol {
                        depth_callers.extend(caller_list.clone());
                    }
                }

                if depth_callers.is_empty() {
                    if let Some(bare_method) = symbol.rsplit("::").next() {
                        if bare_method != symbol {
                            if let Some(caller_list) = index.reverse_calls.get(bare_method) {
                                depth_callers.extend(caller_list.clone());
                            }
                        }
                    }
                }

                for caller in depth_callers {
                    let key = (caller.name.clone(), caller.file.clone(), caller.line);
                    if seen.insert(key) {
                        next_symbols.push(caller.name.clone());
                        all_callers.push(CallerWithDepth {
                            name: caller.name,
                            impl_type: caller.impl_type,
                            file: caller.file,
                            line: caller.line,
                            depth,
                        });
                    }
                }
            }
            current_symbols = next_symbols;
            if current_symbols.is_empty() {
                break;
            }
        }

        if all_callers.is_empty() {
            let suffix = format!("::{}", params.0.symbol);
            let mut ref_entries: Vec<(String, usize)> = Vec::new();

            if let Some(ref_list) = index.references.get(&params.0.symbol) {
                ref_entries.extend(ref_list.clone());
            }
            for (key, ref_list) in &index.references {
                if key != &params.0.symbol && key.ends_with(&suffix) {
                    ref_entries.extend(ref_list.clone());
                }
            }

            for (file, line) in ref_entries {
                let key = ("<type reference>".to_string(), file.clone(), line);
                if seen.insert(key) {
                    all_callers.push(CallerWithDepth {
                        name: "<type reference>".to_string(),
                        impl_type: None,
                        file,
                        line,
                        depth: 0,
                    });
                }
            }
        }

        all_callers.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.line.cmp(&b.line))
        });

        let result = CallersResult {
            callers: all_callers,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Find dependencies of a symbol (upstream = what it calls, downstream = what calls it, both). Includes file:line info."
    )]
    async fn find_dependencies(
        &self,
        params: Parameters<FindDependenciesParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let direction = params.0.direction.to_lowercase();
        let symbol = &params.0.symbol;
        let suffix = format!("::{}", symbol);

        let type_file = symbol
            .split("::")
            .next()
            .and_then(|type_name| index.symbol_table.get(type_name))
            .map(|(file, _)| file.clone());

        let mut upstream = Vec::new();
        if direction == "upstream" || direction == "both" {
            if let Some(targets) = index.call_graph.get(symbol) {
                upstream.extend(targets.clone());
            }
            for (key, targets) in &index.call_graph {
                if key != symbol && key.ends_with(&suffix) {
                    upstream.extend(targets.clone());
                }
            }
            if upstream.is_empty() {
                if let Some(ref target_file) = type_file {
                    if let Some(bare_method) = symbol.rsplit("::").next() {
                        for (key, targets) in &index.call_graph {
                            if key.ends_with(&format!("::{}", bare_method))
                                && targets.iter().any(|t| t.file == *target_file)
                            {
                                upstream.extend(targets.clone());
                            }
                        }
                    }
                }
            }
            upstream.retain(|t| !is_trivial_dependency(&t.name));
            upstream.dedup_by(|a, b| a.name == b.name && a.file == b.file && a.line == b.line);
        }

        let mut downstream = Vec::new();
        if direction == "downstream" || direction == "both" {
            if let Some(callers) = index.reverse_calls.get(symbol) {
                downstream.extend(callers.clone());
            }
            for (key, callers) in &index.reverse_calls {
                if key != symbol && key.ends_with(&suffix) {
                    downstream.extend(callers.clone());
                }
            }
            if downstream.is_empty() {
                if let Some(bare_method) = symbol.rsplit("::").next() {
                    if let Some(callers) = index.reverse_calls.get(bare_method) {
                        if let Some(ref target_file) = type_file {
                            downstream
                                .extend(callers.iter().filter(|c| c.file == *target_file).cloned());
                        }
                    }
                }
            }
            downstream.dedup_by(|a, b| a.name == b.name && a.file == b.file && a.line == b.line);
        }

        let mut references: Vec<ReferenceInfo> = Vec::new();
        if let Some(refs) = index.references.get(symbol) {
            references.extend(refs.iter().take(50).map(|(file, line)| ReferenceInfo {
                file: file.clone(),
                line: *line,
            }));
        }
        for (key, refs) in &index.references {
            if key != symbol && key.ends_with(&suffix) {
                references.extend(refs.iter().take(50).map(|(file, line)| ReferenceInfo {
                    file: file.clone(),
                    line: *line,
                }));
            }
        }
        references.truncate(50);

        let result = DependenciesResult {
            upstream,
            downstream,
            references,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Get the module tree structure of the codebase. Returns file paths with symbol counts."
    )]
    async fn get_module_tree(
        &self,
        params: Parameters<GetModuleTreeParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let filter_prefix = params.0.root.as_deref().unwrap_or("");

        let mut modules: Vec<ModuleInfo> = index
            .result
            .files
            .iter()
            .filter(|f| f.relative_path.starts_with(filter_prefix) || filter_prefix.is_empty())
            .map(|f| ModuleInfo {
                path: f.relative_path.clone(),
                symbol_count: f.parsed.symbols.symbols.len(),
            })
            .collect();

        modules.sort_by(|a, b| a.path.cmp(&b.path));

        let result = ModuleTreeResult { modules };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Get the type hierarchy for a symbol (traits it implements, derive-generated impls, types that implement it, supertraits)"
    )]
    async fn get_type_hierarchy(
        &self,
        params: Parameters<GetTypeHierarchyParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let symbol = &params.0.symbol;
        let suffix = format!("::{}", symbol);

        let mut implementors: Vec<ImplInfo> =
            index.impl_map.get(symbol).cloned().unwrap_or_default();
        for (key, impls) in &index.impl_map {
            if key == symbol {
                continue;
            }
            let stripped = strip_generics(key);
            if stripped == symbol || key.ends_with(&suffix) || stripped.ends_with(&suffix) {
                implementors.extend(impls.clone());
            }
        }
        implementors.dedup_by(|a, b| a.type_name == b.type_name && a.file == b.file);

        let mut implements: Vec<String> = index
            .reverse_impl_map
            .get(symbol)
            .cloned()
            .unwrap_or_default();
        for (key, traits) in &index.reverse_impl_map {
            if key != symbol && (key.ends_with(&suffix) || strip_generics(key) == symbol) {
                implements.extend(traits.clone());
            }
        }
        implements.sort();
        implements.dedup();

        let mut derived_traits: Vec<String> =
            index.derive_map.get(symbol).cloned().unwrap_or_default();
        for (key, traits) in &index.derive_map {
            if key != symbol && (key.ends_with(&suffix) || strip_generics(key) == symbol) {
                derived_traits.extend(traits.clone());
            }
        }
        derived_traits.sort();
        derived_traits.dedup();

        let mut supertraits = Vec::new();
        let mut base_classes = Vec::new();

        for file in &index.result.files {
            for symbol in &file.parsed.symbols.symbols {
                if symbol.name == params.0.symbol {
                    if let SymbolKind::Trait {
                        supertraits: st, ..
                    } = &symbol.kind
                    {
                        supertraits.extend(st.clone());
                    }
                    if let SymbolKind::Class { bases, .. } = &symbol.kind {
                        base_classes.extend(bases.clone());
                    }
                }
            }
        }

        let result = TypeHierarchyResult {
            implementors,
            implements,
            derived_traits,
            supertraits,
            base_classes,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Get an architectural summary of the codebase or a specific module. Returns structured JSON with counts and hotspots."
    )]
    async fn summarize(&self, params: Parameters<SummarizeParams>) -> Result<String, McpError> {
        let index = self.index.read().await;

        let files: Vec<&FileResult> = if let Some(ref scope) = params.0.scope {
            index
                .result
                .files
                .iter()
                .filter(|f| f.relative_path.starts_with(scope))
                .collect()
        } else {
            index.result.files.iter().collect()
        };

        let total_files = files.len();
        let total_lines: usize = files.iter().map(|f| f.lines).sum();
        let total_symbols: usize = files.iter().map(|f| f.parsed.symbols.symbols.len()).sum();

        let git_commit = index
            .result
            .git_info
            .as_ref()
            .map(|g| g.commit_short.clone());

        let mut structs = 0;
        let mut enums = 0;
        let mut traits = 0;
        let mut functions = 0;
        let mut classes = 0;

        for file in &files {
            for symbol in &file.parsed.symbols.symbols {
                match &symbol.kind {
                    SymbolKind::Struct { .. } => structs += 1,
                    SymbolKind::Enum { .. } => enums += 1,
                    SymbolKind::Trait { .. } => traits += 1,
                    SymbolKind::Function { .. } => functions += 1,
                    SymbolKind::Class { .. } => classes += 1,
                    _ => {}
                }
            }
        }

        let mut pub_count = 0;
        let mut priv_count = 0;
        for file in &files {
            for symbol in &file.parsed.symbols.symbols {
                match symbol.visibility {
                    Visibility::Public => pub_count += 1,
                    Visibility::Private => priv_count += 1,
                    _ => {}
                }
            }
        }

        let mut high_complexity = Vec::new();
        for file in &files {
            for cx in &file.parsed.complexity {
                if cx.metrics.cyclomatic >= 10 {
                    high_complexity.push(ComplexityInfo {
                        name: cx.name.clone(),
                        file: file.relative_path.clone(),
                        line: cx.line,
                        cyclomatic: cx.metrics.cyclomatic,
                    });
                }
            }
        }
        high_complexity.sort_by(|a, b| b.cyclomatic.cmp(&a.cyclomatic));
        high_complexity.truncate(20);

        let result = SummaryResult {
            files: total_files,
            lines: total_lines,
            symbols: total_symbols,
            git_commit,
            by_kind: KindCounts {
                structs,
                enums,
                traits,
                functions,
                classes,
            },
            visibility: VisibilityCounts {
                public: pub_count,
                private: priv_count,
            },
            high_complexity,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Re-scan the codebase and return a summary of changes. Persists cache to disk."
    )]
    async fn rescan(&self) -> Result<String, McpError> {
        let root = self.root.clone();

        let workspace = match detect_workspace(&root).await {
            Ok(w) => w,
            Err(e) => {
                return Ok(format!(
                    "{{\"error\": \"Failed to detect workspace: {}\"}}",
                    e
                ));
            }
        };

        let cache_path = root.join(".charter/cache.bin");
        let cache = Cache::load(&cache_path).await.unwrap_or_default();

        let walk_result = match walk::walk_directory(&root).await {
            Ok(w) => w,
            Err(e) => {
                return Ok(format!(
                    "{{\"error\": \"Failed to walk directory: {}\"}}",
                    e
                ));
            }
        };

        let old_file_count = {
            let index = self.index.read().await;
            index.result.files.len()
        };

        let result = match pipeline::run_phase1_with_walk(
            &root,
            &workspace,
            &cache,
            None,
            walk_result,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(format!("{{\"error\": \"Failed to run pipeline: {}\"}}", e));
            }
        };

        let symbol_table = pipeline::build_symbol_table(&result.files);
        let references = pipeline::run_phase2(&result.files, &symbol_table);

        let new_file_count = result.files.len();

        let new_cache = build_cache(&result.files);
        let cache_persisted = new_cache.save(&cache_path).await.is_ok();

        let new_index = Index::new(result, symbol_table, references);

        let mut index = self.index.write().await;
        *index = new_index;

        let result = RescanResult {
            old_file_count,
            new_file_count,
            cache_persisted,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Get import/use statements. If file is given, returns all imports in that file. If symbol is given, returns all files that import that symbol. Both can be specified."
    )]
    async fn get_imports(
        &self,
        params: Parameters<GetImportsParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;

        let file_imports = params.0.file.as_ref().map(|file| {
            let mut results = Vec::new();
            if let Some(imports) = index.imports_by_file.get(file) {
                results.extend(imports.clone());
            }
            for (key, imports) in &index.imports_by_file {
                if key != file && (key.ends_with(file) || file.ends_with(key.as_str())) {
                    results.extend(imports.clone());
                }
            }
            results
        });

        let symbol_locations = params.0.symbol.as_ref().map(|symbol| {
            let mut results = Vec::new();
            if let Some(locations) = index.imports_by_symbol.get(symbol) {
                results.extend(locations.clone());
            }
            for (key, locations) in &index.imports_by_symbol {
                if key != symbol && (key.ends_with(symbol) || symbol.ends_with(key.as_str())) {
                    results.extend(locations.clone());
                }
            }
            results
        });

        if file_imports.as_ref().is_none_or(|v| v.is_empty())
            && symbol_locations.as_ref().is_none_or(|v| v.is_empty())
        {
            return Ok("{\"error\": \"No imports found. Provide file or symbol parameter.\"}".to_string());
        }

        let result = GetImportsResult {
            file_imports,
            symbol_locations,
        };
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Get the function body/implementation of a symbol. Returns captured snippets with importance scores."
    )]
    async fn get_snippet(&self, params: Parameters<GetSnippetParams>) -> Result<String, McpError> {
        let index = self.index.read().await;
        let mut snippets = Vec::new();
        let query = &params.0.name;
        let query_lower = query.to_lowercase();

        if let Some(snips) = index.snippets_by_name.get(query) {
            snippets.extend(snips.clone());
        }

        for (name, snips) in &index.snippets_by_name {
            if name.ends_with(&format!("::{}", query)) && name != query {
                snippets.extend(snips.clone());
            }
        }

        if snippets.is_empty() {
            if let Some(method_part) = query.rsplit("::").next() {
                if method_part != query {
                    let method_lower = method_part.to_lowercase();
                    for (name, snips) in &index.snippets_by_name {
                        let name_lower = name.to_lowercase();
                        let name_method = name_lower.rsplit("::").next().unwrap_or(&name_lower);
                        if name_method == method_lower && name_lower.contains(&query_lower) {
                            snippets.extend(snips.clone());
                        }
                    }
                }
            }
            if snippets.is_empty() {
                for (name, snips) in &index.snippets_by_name {
                    let name_lower = name.to_lowercase();
                    if name_lower == query_lower {
                        snippets.extend(snips.clone());
                    }
                }
            }
            snippets.truncate(10);
        }

        snippets.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));
        let mut seen = std::collections::HashSet::new();
        snippets.retain(|s| seen.insert((s.file.clone(), s.line)));

        let result = SnippetResult { snippets };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Get the full definition of a struct, enum, or trait by name. Returns fields, variants, or method signatures."
    )]
    async fn get_definition(
        &self,
        params: Parameters<GetDefinitionParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let query = &params.0.name;
        let query_lower = query.to_lowercase();
        let suffix = format!("::{}", query);
        let mut definitions = Vec::new();

        for file in &index.result.files {
            for symbol in &file.parsed.symbols.symbols {
                let name_matches = symbol.name == *query
                    || symbol.name.eq_ignore_ascii_case(query)
                    || symbol.name.ends_with(&suffix);
                if !name_matches {
                    continue;
                }
                let (kind, definition) = format_symbol_definition(symbol);
                if let Some(def) = definition {
                    definitions.push(DefinitionInfo {
                        name: symbol.name.clone(),
                        kind,
                        file: file.relative_path.clone(),
                        line: symbol.line,
                        visibility: format!("{}", symbol.visibility),
                        generics: symbol.generics.clone(),
                        definition: def,
                    });
                }
            }
        }

        if definitions.is_empty() {
            for file in &index.result.files {
                for symbol in &file.parsed.symbols.symbols {
                    let name_lower = symbol.name.to_lowercase();
                    if !name_lower.contains(&query_lower) {
                        continue;
                    }
                    let (kind, definition) = format_symbol_definition(symbol);
                    if let Some(def) = definition {
                        definitions.push(DefinitionInfo {
                            name: symbol.name.clone(),
                            kind,
                            file: file.relative_path.clone(),
                            line: symbol.line,
                            visibility: format!("{}", symbol.visibility),
                            generics: symbol.generics.clone(),
                            definition: def,
                        });
                    }
                }
            }
            definitions.truncate(10);
        }

        if definitions.is_empty() {
            for (name, ext_syms) in &index.external_symbols {
                let name_lower = name.to_lowercase();
                let bare = bare_name(&name_lower);
                if bare == query_lower
                    || name_lower == query_lower
                    || name_lower.ends_with(&format!("::{}", query_lower))
                {
                    for sym in ext_syms {
                        let def = sym
                            .signature
                            .clone()
                            .unwrap_or_else(|| format!("{} {}", sym.kind, name));
                        definitions.push(DefinitionInfo {
                            name: name.clone(),
                            kind: sym.kind.clone(),
                            file: format!("[{}] {}", sym.crate_name, sym.file),
                            line: sym.line,
                            visibility: "pub".to_string(),
                            generics: String::new(),
                            definition: def,
                        });
                    }
                }
            }
            definitions.truncate(10);
        }

        let result = DefinitionResult { definitions };
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Read source code from a file at specific line range. Use this to read any function body not captured in snippets."
    )]
    async fn read_source(&self, params: Parameters<ReadSourceParams>) -> Result<String, McpError> {
        let index = self.index.read().await;

        let file_result = index
            .result
            .files
            .iter()
            .find(|f| f.relative_path == params.0.file);

        let file_result = match file_result {
            Some(f) => f,
            None => {
                return Ok(format!(
                    "{{\"error\": \"File '{}' not found in index\"}}",
                    params.0.file
                ));
            }
        };

        let file_path = &file_result.path;
        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(format!("{{\"error\": \"Failed to read file: {}\"}}", e));
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let start = params.0.start_line.saturating_sub(1);
        let end = params.0.end_line.unwrap_or(start + 50).min(lines.len());

        if start >= lines.len() {
            return Ok(format!(
                "{{\"error\": \"Start line {} beyond file length {}\"}}",
                params.0.start_line,
                lines.len()
            ));
        }

        let selected: Vec<&str> = lines[start..end].to_vec();
        let content = selected.join("\n");

        let language = if params.0.file.ends_with(".py") || params.0.file.ends_with(".pyi") {
            "python"
        } else {
            "rust"
        };

        let result = ReadSourceResult {
            file: params.0.file.clone(),
            start_line: start + 1,
            end_line: end,
            content,
            language: language.to_string(),
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Search for text patterns (regex) across all indexed files. Supports glob filtering, case insensitivity, and context lines. Use for finding .unwrap() calls, TODO comments, or any text pattern."
    )]
    async fn search_text(&self, params: Parameters<SearchTextParams>) -> Result<String, McpError> {
        let index = self.index.read().await;
        let max_results = params.0.max_results.unwrap_or(100);
        let case_insensitive = params.0.case_insensitive.unwrap_or(false);
        let context_lines = params.0.context_lines.unwrap_or(0);

        let matcher = match RegexMatcherBuilder::new()
            .case_insensitive(case_insensitive)
            .build(&params.0.pattern)
        {
            Ok(m) => m,
            Err(error) => {
                return Ok(format!(
                    "{{\"error\": \"Invalid regex pattern: {}\"}}",
                    error
                ));
            }
        };

        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .before_context(context_lines)
            .after_context(context_lines)
            .build();

        let glob_filter = params.0.glob.as_deref();

        let mut all_matches: Vec<TextMatch> = Vec::new();
        let mut files_searched = 0usize;
        let mut truncated = false;

        for file in &index.result.files {
            if let Some(glob) = glob_filter {
                if !matches_glob(&file.relative_path, glob) {
                    continue;
                }
            }

            files_searched += 1;

            let remaining = max_results.saturating_sub(all_matches.len());
            if remaining == 0 {
                truncated = true;
                break;
            }

            let mut sink = TextSearchSink::new(file.relative_path.clone(), remaining);

            let search_result = searcher.search_path(&matcher, &file.path, &mut sink);

            if search_result.is_ok() {
                if sink.is_full() && all_matches.len() + sink.matches.len() >= max_results {
                    truncated = true;
                }
                all_matches.extend(sink.matches);
            }

            if all_matches.len() >= max_results {
                all_matches.truncate(max_results);
                truncated = true;
                break;
            }
        }

        let result = SearchTextResult {
            matches: all_matches,
            files_searched,
            truncated,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }
}

fn build_cache(files: &[FileResult]) -> Cache {
    let mut cache = Cache::default();

    for file in files {
        let mtime = std::fs::metadata(&file.path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        cache.entries.insert(
            file.relative_path.clone(),
            crate::cache::CacheEntry {
                hash: file.hash.clone(),
                mtime,
                size: file.size,
                lines: file.lines,
                data: crate::cache::FileData {
                    parsed: file.parsed.clone(),
                },
            },
        );
    }

    cache
}

fn is_trivial_dependency(name: &str) -> bool {
    const TRIVIAL_METHODS: &[&str] = &[
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
        "is_some",
        "is_none",
        "is_ok",
        "is_err",
        "is_some_and",
        "is_none_or",
        "contains",
        "extend",
        "clear",
        "with_capacity",
        "capacity",
        "reserve",
        "truncate",
        "drain",
        "retain",
        "sort",
        "sort_by",
        "dedup",
        "dedup_by",
        "first",
        "last",
        "split",
        "join",
        "trim",
        "trim_start",
        "trim_end",
        "starts_with",
        "ends_with",
        "to_lowercase",
        "to_uppercase",
        "as_str",
        "as_bytes",
        "as_slice",
        "to_vec",
        "take",
        "replace",
        "swap",
        "min",
        "max",
        "clamp",
        "abs",
        "floor",
        "ceil",
        "round",
        "powi",
        "powf",
        "sqrt",
        "sin",
        "cos",
        "tan",
        "atan2",
        "from_le_bytes",
        "from_be_bytes",
        "to_le_bytes",
        "to_be_bytes",
        "chunks",
        "chunks_exact",
        "windows",
        "enumerate",
        "zip",
        "flat_map",
        "flatten",
        "any",
        "all",
        "find",
        "position",
        "count",
        "sum",
        "fold",
        "for_each",
        "copied",
        "cloned",
        "rev",
        "skip",
        "chain",
        "peekable",
        "fuse",
        "map_or",
        "map_or_else",
        "map_err",
        "try_inverse",
        "unwrap_or_else",
        "saturating_sub",
        "saturating_add",
        "checked_add",
        "checked_sub",
        "try_into",
        "try_from",
        "fmt",
        "write",
        "read",
        "flush",
        "drop",
    ];

    let bare = name.rsplit("::").next().unwrap_or(name);
    let bare = bare.split('.').next_back().unwrap_or(bare);

    if TRIVIAL_METHODS.contains(&bare) {
        return true;
    }

    let receiver = name.split("::").next().unwrap_or("");
    const TRIVIAL_TYPES: &[&str] = &[
        "Vec", "String", "Option", "Result", "Box", "Arc", "Rc", "HashMap", "HashSet", "BTreeMap",
        "BTreeSet", "f32", "f64", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64",
        "isize", "bool", "str", "Some", "None", "Ok", "Err",
    ];

    if TRIVIAL_TYPES.contains(&receiver) {
        return true;
    }

    name == "?"
}

fn format_symbol_definition(symbol: &crate::extract::symbols::Symbol) -> (String, Option<String>) {
    use crate::extract::symbols::VariantPayload;
    match &symbol.kind {
        SymbolKind::Struct { fields } => {
            let mut def = format!(
                "{}struct {}{}",
                symbol.visibility.prefix(),
                symbol.name,
                symbol.generics,
            );
            if fields.is_empty() {
                def.push(';');
            } else {
                def.push_str(" {\n");
                for field in fields {
                    def.push_str(&format!(
                        "    {}{}: {},\n",
                        field.visibility.prefix(),
                        field.name,
                        field.field_type
                    ));
                }
                def.push('}');
            }
            ("struct".to_string(), Some(def))
        }
        SymbolKind::Enum { variants } => {
            let mut def = format!(
                "{}enum {}{}",
                symbol.visibility.prefix(),
                symbol.name,
                symbol.generics,
            );
            def.push_str(" {\n");
            for variant in variants {
                match &variant.payload {
                    None => def.push_str(&format!("    {},\n", variant.name)),
                    Some(VariantPayload::Tuple(types)) => {
                        def.push_str(&format!("    {}({}),\n", variant.name, types.join(", ")));
                    }
                    Some(VariantPayload::Struct(fields)) => {
                        def.push_str(&format!("    {} {{ ", variant.name));
                        let field_strs: Vec<String> = fields
                            .iter()
                            .map(|(n, t)| format!("{}: {}", n, t))
                            .collect();
                        def.push_str(&field_strs.join(", "));
                        def.push_str(" },\n");
                    }
                }
            }
            def.push('}');
            ("enum".to_string(), Some(def))
        }
        SymbolKind::Trait {
            supertraits,
            methods,
            associated_types,
        } => {
            let mut def = format!(
                "{}trait {}{}",
                symbol.visibility.prefix(),
                symbol.name,
                symbol.generics,
            );
            if !supertraits.is_empty() {
                def.push_str(&format!(": {}", supertraits.join(" + ")));
            }
            def.push_str(" {\n");
            for assoc_type in associated_types {
                if let Some(ref bounds) = assoc_type.bounds {
                    def.push_str(&format!("    type {}: {};\n", assoc_type.name, bounds));
                } else {
                    def.push_str(&format!("    type {};\n", assoc_type.name));
                }
            }
            for method in methods {
                let default_marker = if method.has_default { " { ... }" } else { ";" };
                def.push_str(&format!(
                    "    fn {}({}){}\n",
                    method.name, method.signature, default_marker
                ));
            }
            def.push('}');
            ("trait".to_string(), Some(def))
        }
        _ => {
            let kind = match &symbol.kind {
                SymbolKind::Function { .. } => "function",
                SymbolKind::Const { .. } => "const",
                SymbolKind::Static { .. } => "static",
                SymbolKind::TypeAlias { .. } => "type",
                SymbolKind::Mod => "mod",
                _ => "other",
            };
            (kind.to_string(), None)
        }
    }
}

fn matches_glob(path: &str, glob: &str) -> bool {
    glob_match(path.as_bytes(), glob.as_bytes())
}

fn glob_match(path: &[u8], pattern: &[u8]) -> bool {
    let mut px = 0;
    let mut gx = 0;
    let mut star_px = usize::MAX;
    let mut star_gx = 0;

    while px < path.len() {
        if gx < pattern.len() && pattern[gx] == b'*' {
            if gx + 1 < pattern.len() && pattern[gx + 1] == b'*' {
                gx += 2;
                if gx < pattern.len() && pattern[gx] == b'/' {
                    gx += 1;
                }
                star_px = px;
                star_gx = gx;
            } else {
                gx += 1;
                star_px = px;
                star_gx = gx;
            }
        } else if gx < pattern.len() && pattern[gx] == b'?' {
            if path[px] == b'/' {
                if star_px != usize::MAX {
                    star_px += 1;
                    px = star_px;
                    gx = star_gx;
                } else {
                    return false;
                }
            } else {
                px += 1;
                gx += 1;
            }
        } else if gx < pattern.len() && path[px] == pattern[gx] {
            px += 1;
            gx += 1;
        } else if star_px != usize::MAX {
            star_px += 1;
            px = star_px;
            gx = star_gx;
        } else {
            return false;
        }
    }

    while gx < pattern.len() && pattern[gx] == b'*' {
        gx += 1;
    }

    gx == pattern.len()
}

fn bare_name(qualified: &str) -> &str {
    qualified.rsplit("::").next().unwrap_or(qualified)
}

fn symbol_match_score(query: &str, name_lower: &str, bare_lower: &str) -> u32 {
    if bare_lower == query || name_lower == query {
        return 100;
    }
    if bare_lower.starts_with(query) {
        return 80;
    }
    let suffix = format!("::{}", query);
    if name_lower.ends_with(&suffix) {
        return 70;
    }
    if bare_lower.contains(query) {
        return 50;
    }
    if name_lower.contains(query) {
        return 30;
    }
    if fuzzy_match(query, bare_lower) {
        return 10;
    }
    0
}

fn fuzzy_match(query: &str, target: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if query.len() < 3 {
        return target.contains(query);
    }

    let query_chars: Vec<char> = query.chars().collect();
    let target_chars: Vec<char> = target.chars().collect();
    let mut query_index = 0;
    let mut consecutive_total = 0;
    let mut current_run = 0;

    for &target_char in &target_chars {
        if query_index < query_chars.len() && query_chars[query_index] == target_char {
            query_index += 1;
            current_run += 1;
            if current_run >= 2 {
                consecutive_total += 1;
            }
        } else {
            current_run = 0;
        }
    }

    query_index == query_chars.len() && consecutive_total >= query.len() / 2
}

fn split_pascal_case(name: &str) -> Vec<String> {
    let bare = bare_name(name);
    let mut words = Vec::new();
    let mut current = String::new();

    for character in bare.chars() {
        if character.is_ascii_uppercase() && !current.is_empty() {
            words.push(current.to_lowercase());
            current.clear();
        }
        current.push(character);
    }
    if !current.is_empty() {
        words.push(current.to_lowercase());
    }

    words
}

fn pascal_words_match(query_words: &[String], key_words: &[String]) -> bool {
    if query_words.is_empty() || key_words.len() < query_words.len() {
        return false;
    }
    let mut key_index = 0;
    for query_word in query_words {
        let mut found = false;
        while key_index < key_words.len() {
            if key_words[key_index] == *query_word {
                key_index += 1;
                found = true;
                break;
            }
            key_index += 1;
        }
        if !found {
            return false;
        }
    }
    true
}

fn strip_generics(name: &str) -> &str {
    match name.find('<') {
        Some(index) => &name[..index],
        None => name,
    }
}

#[tool_handler]
impl ServerHandler for CharterServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Charter codebase structural analysis server. Tools: search_symbols (fuzzy search, regex=true for pattern matching), find_symbol (exact/fuzzy lookup), find_references (type/symbol usage sites), find_implementations (includes derives), find_callers (with receiver type, falls back to references for types), find_dependencies (with receiver type), get_module_tree, get_type_hierarchy (includes derives), summarize, get_snippet (captured function bodies with end_line for read_source), get_imports (file imports or symbol import locations), read_source (any source range), search_text (regex text search with glob filtering and context lines), rescan. All return JSON.".to_string(),
            ),
        }
    }
}

pub async fn serve(root: &Path, external: bool) -> Result<()> {
    let workspace = detect_workspace(root).await?;

    let cache_path = root.join(".charter/cache.bin");
    let cache = Cache::load(&cache_path).await.unwrap_or_default();

    let walk_result = walk::walk_directory(root).await?;

    let result =
        pipeline::run_phase1_with_walk(root, &workspace, &cache, None, walk_result).await?;

    let symbol_table = pipeline::build_symbol_table(&result.files);
    let references = pipeline::run_phase2(&result.files, &symbol_table);

    let mut index = Index::new(result, symbol_table, references);

    if external {
        let direct_deps = external::parse_direct_deps(root);
        let crates = external::collect_external_crates(root, &direct_deps);
        let ext_symbols = external::extract_external_symbols(&crates);
        let mut ext_map = std::collections::HashMap::new();
        for sym in ext_symbols {
            ext_map
                .entry(sym.name.clone())
                .or_insert_with(Vec::new)
                .push(ExternalSymbolInfo {
                    name: sym.name,
                    kind: sym.kind,
                    crate_name: sym.crate_name,
                    file: sym.file,
                    line: sym.line,
                    signature: sym.signature,
                });
        }
        index.external_symbols = ext_map;
    }

    let index = Arc::new(RwLock::new(index));

    let server = CharterServer::new(index, root.to_path_buf());

    let transport = stdio();
    server.serve(transport).await?.waiting().await?;

    Ok(())
}
