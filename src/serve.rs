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
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::Cache;
use crate::detect::detect_workspace;
use crate::extract::symbols::{Symbol, SymbolKind, Visibility};
use crate::pipeline::{self, FileResult, PipelineResult, walk};

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
    pub body: String,
    pub importance_score: u32,
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
                    symbols_by_name.entry(qualified).or_default().push(info);
                }
            }

            for call_info in &file.parsed.call_graph {
                let caller = call_info.caller.qualified_name();
                let caller_impl_type = call_info.caller.impl_type.clone();
                let caller_line = call_info.line;
                for callee in &call_info.callees {
                    let callee_name = callee.qualified_target();
                    call_graph
                        .entry(caller.clone())
                        .or_default()
                        .push(CallTarget {
                            name: callee_name.clone(),
                            receiver_type: callee.target_type.clone(),
                            file: file.relative_path.clone(),
                            line: callee.line,
                        });
                    reverse_calls
                        .entry(callee_name)
                        .or_default()
                        .push(CallerInfo {
                            name: caller.clone(),
                            impl_type: caller_impl_type.clone(),
                            file: file.relative_path.clone(),
                            line: caller_line,
                        });
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
                let body_text = captured
                    .body
                    .full_text
                    .clone()
                    .unwrap_or_else(|| "[body not captured]".to_string());
                snippets_by_name
                    .entry(key.clone())
                    .or_default()
                    .push(SnippetInfo {
                        function_name: captured.function_name.clone(),
                        impl_type: captured.impl_type.clone(),
                        file: file.relative_path.clone(),
                        line: captured.line,
                        body: body_text,
                        importance_score: captured.importance_score,
                    });
                if captured.impl_type.is_some() {
                    snippets_by_name
                        .entry(captured.function_name.clone())
                        .or_default()
                        .push(SnippetInfo {
                            function_name: captured.function_name.clone(),
                            impl_type: captured.impl_type.clone(),
                            file: file.relative_path.clone(),
                            line: captured.line,
                            body: captured
                                .body
                                .full_text
                                .clone()
                                .unwrap_or_else(|| "[body not captured]".to_string()),
                            importance_score: captured.importance_score,
                        });
                }
            }
        }

        for traits in derive_map.values_mut() {
            traits.sort();
            traits.dedup();
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
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FindDependenciesParams {
    pub symbol: String,
    pub direction: String,
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
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetSnippetParams {
    pub name: String,
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
    pub callers: Vec<CallerInfo>,
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
        description = "Search for symbols by name (fuzzy/partial matching supported). Filter by kind and limit results."
    )]
    async fn search_symbols(
        &self,
        params: Parameters<SearchSymbolsParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let query = params.0.query.to_lowercase();
        let limit = params.0.limit.unwrap_or(50);

        let mut symbols = Vec::new();
        let mut traits = Vec::new();
        let mut calls = Vec::new();

        for (name, syms) in &index.symbols_by_name {
            let name_lower = name.to_lowercase();
            if name_lower.contains(&query) || fuzzy_match(&query, &name_lower) {
                for sym in syms {
                    if let Some(ref kind_filter) = params.0.kind {
                        if &sym.kind != kind_filter {
                            continue;
                        }
                    }
                    symbols.push(SymbolResult {
                        name: name.clone(),
                        kind: sym.kind.clone(),
                        file: sym.file.clone(),
                        line: sym.line,
                        signature: sym.signature.clone(),
                        visibility: sym.visibility.clone(),
                    });
                }
            }
        }

        for (trait_name, impls) in &index.impl_map {
            let trait_lower = trait_name.to_lowercase();
            if trait_lower.contains(&query) {
                traits.push(TraitResult {
                    trait_name: trait_name.clone(),
                    implementors: impls.iter().map(|i| i.type_name.clone()).collect(),
                });
            }
        }

        for (caller, targets) in &index.call_graph {
            let caller_lower = caller.to_lowercase();
            if caller_lower.contains(&query) {
                calls.push(CallResult {
                    caller: caller.clone(),
                    callees: targets.iter().map(|t| t.name.clone()).collect(),
                });
            }
        }

        symbols.truncate(limit);
        traits.truncate(limit / 2);
        calls.truncate(limit / 2);

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
        let mut results = Vec::new();
        let query_lower = params.0.name.to_lowercase();

        if let Some(symbols) = index.symbols_by_name.get(&params.0.name) {
            for sym in symbols {
                if let Some(ref kind_filter) = params.0.kind {
                    if &sym.kind != kind_filter {
                        continue;
                    }
                }
                results.push(SymbolResult {
                    name: sym.name.clone(),
                    kind: sym.kind.clone(),
                    file: sym.file.clone(),
                    line: sym.line,
                    signature: sym.signature.clone(),
                    visibility: sym.visibility.clone(),
                });
            }
        }

        for (qualified_name, symbols) in &index.symbols_by_name {
            if qualified_name.ends_with(&format!("::{}", params.0.name)) {
                for sym in symbols {
                    if let Some(ref kind_filter) = params.0.kind {
                        if &sym.kind != kind_filter {
                            continue;
                        }
                    }
                    results.push(SymbolResult {
                        name: qualified_name.clone(),
                        kind: sym.kind.clone(),
                        file: sym.file.clone(),
                        line: sym.line,
                        signature: sym.signature.clone(),
                        visibility: sym.visibility.clone(),
                    });
                }
            }
        }

        if results.is_empty() {
            for (name, symbols) in &index.symbols_by_name {
                let name_lower = name.to_lowercase();
                if name_lower.contains(&query_lower) || fuzzy_match(&query_lower, &name_lower) {
                    for sym in symbols {
                        if let Some(ref kind_filter) = params.0.kind {
                            if &sym.kind != kind_filter {
                                continue;
                            }
                        }
                        results.push(SymbolResult {
                            name: name.clone(),
                            kind: sym.kind.clone(),
                            file: sym.file.clone(),
                            line: sym.line,
                            signature: sym.signature.clone(),
                            visibility: sym.visibility.clone(),
                        });
                    }
                }
            }
            results.truncate(20);
        }

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

        let trait_implementors = index
            .impl_map
            .get(&params.0.symbol)
            .cloned()
            .unwrap_or_default();

        let type_implements = index
            .reverse_impl_map
            .get(&params.0.symbol)
            .cloned()
            .unwrap_or_default();

        let derived_traits = index
            .derive_map
            .get(&params.0.symbol)
            .cloned()
            .unwrap_or_default();

        let mut methods = Vec::new();
        for file in &index.result.files {
            for inherent_impl in &file.parsed.symbols.inherent_impls {
                if inherent_impl.type_name == params.0.symbol {
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

        let result = ImplementationsResult {
            trait_implementors,
            type_implements,
            methods,
            derived_traits,
        };

        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
    }

    #[tool(
        description = "Find all call sites of a function or method. Returns caller name, file, and line."
    )]
    async fn find_callers(
        &self,
        params: Parameters<FindCallersParams>,
    ) -> Result<String, McpError> {
        let index = self.index.read().await;
        let mut callers = Vec::new();

        if let Some(caller_list) = index.reverse_calls.get(&params.0.symbol) {
            callers.extend(caller_list.clone());
        }

        for (qualified_name, caller_list) in &index.reverse_calls {
            if qualified_name.ends_with(&format!("::{}", params.0.symbol))
                && qualified_name != &params.0.symbol
            {
                callers.extend(caller_list.clone());
            }
        }

        let result = CallersResult { callers };

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

        let upstream = if direction == "upstream" || direction == "both" {
            index
                .call_graph
                .get(&params.0.symbol)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let downstream = if direction == "downstream" || direction == "both" {
            index
                .reverse_calls
                .get(&params.0.symbol)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let references: Vec<ReferenceInfo> = index
            .references
            .get(&params.0.symbol)
            .map(|refs| {
                refs.iter()
                    .take(50)
                    .map(|(file, line)| ReferenceInfo {
                        file: file.clone(),
                        line: *line,
                    })
                    .collect()
            })
            .unwrap_or_default();

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

        let implementors = index
            .impl_map
            .get(&params.0.symbol)
            .cloned()
            .unwrap_or_default();

        let implements = index
            .reverse_impl_map
            .get(&params.0.symbol)
            .cloned()
            .unwrap_or_default();

        let derived_traits = index
            .derive_map
            .get(&params.0.symbol)
            .cloned()
            .unwrap_or_default();

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
            for (name, snips) in &index.snippets_by_name {
                let name_lower = name.to_lowercase();
                if name_lower.contains(&query_lower) {
                    snippets.extend(snips.clone());
                }
            }
            snippets.truncate(10);
        }

        snippets.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));

        let result = SnippetResult { snippets };

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

fn matches_glob(path: &str, glob: &str) -> bool {
    if let Some(suffix) = glob.strip_prefix('*') {
        path.ends_with(suffix)
    } else {
        path.contains(glob)
    }
}

fn fuzzy_match(query: &str, target: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let mut query_chars = query.chars().peekable();
    for target_char in target.chars() {
        if let Some(&query_char) = query_chars.peek() {
            if query_char == target_char {
                query_chars.next();
            }
        }
        if query_chars.peek().is_none() {
            return true;
        }
    }

    query_chars.peek().is_none()
}

#[tool_handler]
impl ServerHandler for CharterServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Charter codebase structural analysis server. Tools: search_symbols (fuzzy search), find_symbol (exact/fuzzy lookup), find_implementations (includes derives), find_callers (with receiver type), find_dependencies (with receiver type), get_module_tree, get_type_hierarchy (includes derives), summarize, get_snippet (captured function bodies), read_source (any source range), search_text (regex text search with glob filtering and context lines), rescan. All return JSON.".to_string(),
            ),
        }
    }
}

pub async fn serve(root: &Path) -> Result<()> {
    let workspace = detect_workspace(root).await?;

    let cache_path = root.join(".charter/cache.bin");
    let cache = Cache::load(&cache_path).await.unwrap_or_default();

    let walk_result = walk::walk_directory(root).await?;

    let result =
        pipeline::run_phase1_with_walk(root, &workspace, &cache, None, walk_result).await?;

    let symbol_table = pipeline::build_symbol_table(&result.files);
    let references = pipeline::run_phase2(&result.files, &symbol_table);

    let index = Arc::new(RwLock::new(Index::new(result, symbol_table, references)));

    let server = CharterServer::new(index, root.to_path_buf());

    let transport = stdio();
    server.serve(transport).await?.waiting().await?;

    Ok(())
}
