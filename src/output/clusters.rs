use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::pipeline::PipelineResult;

struct FunctionInfo {
    name: String,
    impl_type: Option<String>,
    file: String,
    line: usize,
    return_type: Option<String>,
    param_types: Vec<String>,
}

struct Cluster {
    functions: Vec<usize>,
    label: String,
    dominant_file: Option<String>,
    dominant_impl: Option<String>,
}

pub async fn write_clusters(
    charter_dir: &Path,
    result: &PipelineResult,
    stamp: &str,
) -> Result<()> {
    let file = tokio::fs::File::create(charter_dir.join("clusters.md")).await?;
    let mut writer = BufWriter::new(file);

    writer.write_all(stamp.as_bytes()).await?;
    writer.write_all(b"\n\n").await?;
    writer.write_all(b"# Function Clusters\n\n").await?;
    writer
        .write_all(b"Functions grouped by semantic affinity (shared types, call relationships, impl membership).\n\n")
        .await?;

    let functions = collect_functions(result);
    if functions.is_empty() {
        writer.write_all(b"No functions detected.\n").await?;
        writer.flush().await?;
        return Ok(());
    }

    let call_graph = build_call_adjacency(result);
    let affinity = compute_affinity_matrix(&functions, &call_graph);
    let clusters = cluster_functions(&functions, &affinity);

    let mut significant_clusters: Vec<&Cluster> =
        clusters.iter().filter(|c| c.functions.len() >= 3).collect();

    significant_clusters.sort_by(|a, b| b.functions.len().cmp(&a.functions.len()));

    if significant_clusters.is_empty() {
        writer
            .write_all(b"No significant clusters detected (minimum 3 functions required).\n")
            .await?;
        writer.flush().await?;
        return Ok(());
    }

    for (index, cluster) in significant_clusters.iter().take(20).enumerate() {
        let header = format!(
            "## Cluster {}: {} ({} functions)\n\n",
            index + 1,
            cluster.label,
            cluster.functions.len()
        );
        writer.write_all(header.as_bytes()).await?;

        let mut by_file: HashMap<&str, Vec<&FunctionInfo>> = HashMap::new();
        for &func_idx in &cluster.functions {
            let func = &functions[func_idx];
            by_file.entry(&func.file).or_default().push(func);
        }

        let mut files: Vec<&str> = by_file.keys().copied().collect();
        files.sort();

        for file_path in files {
            let funcs = &by_file[file_path];
            let file_line = format!("{}:\n", file_path);
            writer.write_all(file_line.as_bytes()).await?;

            for func in funcs {
                let qualified = match &func.impl_type {
                    Some(t) => format!("{}::{}", t, func.name),
                    None => func.name.clone(),
                };
                let line = format!("  {} (line {})\n", qualified, func.line);
                writer.write_all(line.as_bytes()).await?;
            }
        }

        let internal_calls = count_internal_calls(cluster, &functions, &call_graph);
        let external_calls = count_external_calls(cluster, &functions, &call_graph);

        writer.write_all(b"\n").await?;
        let stats = format!(
            "Internal calls: {}, External calls: {}\n\n",
            internal_calls, external_calls
        );
        writer.write_all(stats.as_bytes()).await?;
    }

    writer.flush().await?;
    Ok(())
}

fn collect_functions(result: &PipelineResult) -> Vec<FunctionInfo> {
    let mut functions = Vec::new();

    for file_result in &result.files {
        for call_info in &file_result.parsed.call_graph {
            let (return_type, param_types) = extract_signature_types(
                &call_info.caller.name,
                call_info.caller.impl_type.as_deref(),
                &file_result.parsed,
            );

            functions.push(FunctionInfo {
                name: call_info.caller.name.clone(),
                impl_type: call_info.caller.impl_type.clone(),
                file: file_result.relative_path.clone(),
                line: call_info.line,
                return_type,
                param_types,
            });
        }
    }

    functions
}

fn extract_signature_types(
    name: &str,
    impl_type: Option<&str>,
    parsed: &crate::pipeline::ParsedFile,
) -> (Option<String>, Vec<String>) {
    for symbol in &parsed.symbols.symbols {
        if symbol.name == name {
            if let crate::extract::symbols::SymbolKind::Function { signature, .. } = &symbol.kind {
                return parse_signature_types(signature);
            }
        }
    }

    if let Some(type_name) = impl_type {
        for imp in &parsed.symbols.inherent_impls {
            if imp.type_name == type_name {
                for method in &imp.methods {
                    if method.name == name {
                        return parse_signature_types(&method.signature);
                    }
                }
            }
        }
    }

    (None, Vec::new())
}

fn parse_signature_types(signature: &str) -> (Option<String>, Vec<String>) {
    let return_type = if let Some(arrow_pos) = signature.rfind("->") {
        let ret = signature[arrow_pos + 2..].trim();
        if !ret.is_empty() && ret != "()" {
            Some(extract_base_type_from_str(ret))
        } else {
            None
        }
    } else {
        None
    };

    let mut param_types = Vec::new();
    if let Some(paren_start) = signature.find('(') {
        if let Some(paren_end) = signature.rfind(')') {
            let params_str = &signature[paren_start + 1..paren_end];
            for param in params_str.split(',') {
                let param = param.trim();
                if param.is_empty() || param == "self" || param == "&self" || param == "&mut self" {
                    continue;
                }
                if let Some(colon_pos) = param.find(':') {
                    let type_part = param[colon_pos + 1..].trim();
                    param_types.push(extract_base_type_from_str(type_part));
                }
            }
        }
    }

    (return_type, param_types)
}

fn extract_base_type_from_str(type_str: &str) -> String {
    let trimmed = type_str
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches("'static ")
        .trim_start_matches("'_ ");

    if let Some(generic_start) = trimmed.find('<') {
        trimmed[..generic_start].to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_call_adjacency(result: &PipelineResult) -> HashMap<String, HashSet<String>> {
    let mut adjacency: HashMap<String, HashSet<String>> = HashMap::new();

    for file_result in &result.files {
        for call_info in &file_result.parsed.call_graph {
            let caller = call_info.caller.qualified_name();

            for edge in &call_info.callees {
                let target = edge.qualified_target();
                adjacency.entry(caller.clone()).or_default().insert(target);
            }
        }
    }

    adjacency
}

fn extract_crate_module(file_path: &str) -> &str {
    let normalized = file_path.replace('\\', "/");
    let path = if normalized.starts_with("src/") {
        &file_path[4..]
    } else {
        file_path
    };

    if let Some(slash_pos) = path.find('/') {
        if let Some(second_slash) = path[slash_pos + 1..].find('/') {
            return &path[..slash_pos + 1 + second_slash];
        }
        return &path[..slash_pos];
    }

    path
}

fn compute_affinity_matrix(
    functions: &[FunctionInfo],
    call_graph: &HashMap<String, HashSet<String>>,
) -> Vec<Vec<i32>> {
    let len = functions.len();
    let mut affinity = vec![vec![0i32; len]; len];

    for index_a in 0..len {
        for index_b in (index_a + 1)..len {
            let func_a = &functions[index_a];
            let func_b = &functions[index_b];

            let mut score = 0i32;

            let same_crate =
                extract_crate_module(&func_a.file) == extract_crate_module(&func_b.file);
            let same_file = func_a.file == func_b.file;

            if func_a.impl_type.is_some() && func_a.impl_type == func_b.impl_type {
                if same_file {
                    score += 15;
                } else if same_crate {
                    score += 5;
                }
            }

            let name_a = qualified_name(func_a);
            let name_b = qualified_name(func_b);

            if let Some(targets) = call_graph.get(&name_a) {
                if targets.contains(&name_b) {
                    score += 5;
                }
            }
            if let Some(targets) = call_graph.get(&name_b) {
                if targets.contains(&name_a) {
                    score += 5;
                }
            }

            if same_file {
                score += 5;
            } else if same_crate {
                score += 2;
            } else {
                score -= 3;
            }

            let shared_types = count_shared_types(func_a, func_b);
            score += (shared_types * 2) as i32;

            affinity[index_a][index_b] = score;
            affinity[index_b][index_a] = score;
        }
    }

    affinity
}

fn qualified_name(func: &FunctionInfo) -> String {
    match &func.impl_type {
        Some(t) => format!("{}::{}", t, func.name),
        None => func.name.clone(),
    }
}

fn count_shared_types(func_a: &FunctionInfo, func_b: &FunctionInfo) -> usize {
    let mut count = 0;

    let types_a: HashSet<&str> = func_a
        .param_types
        .iter()
        .map(|s| s.as_str())
        .chain(func_a.return_type.as_deref())
        .collect();

    let types_b: HashSet<&str> = func_b
        .param_types
        .iter()
        .map(|s| s.as_str())
        .chain(func_b.return_type.as_deref())
        .collect();

    for type_a in &types_a {
        if !is_common_type(type_a) && types_b.contains(type_a) {
            count += 1;
        }
    }

    count
}

fn is_common_type(type_name: &str) -> bool {
    const COMMON: &[&str] = &[
        "bool",
        "char",
        "str",
        "&str",
        "String",
        "&String",
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "f32",
        "f64",
        "()",
        "Self",
        "&Self",
        "&mut Self",
        "Option",
        "Result",
        "Vec",
        "Box",
        "Arc",
        "Rc",
        "HashMap",
        "HashSet",
        "BTreeMap",
        "BTreeSet",
    ];

    let base = type_name
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .split('<')
        .next()
        .unwrap_or(type_name);

    COMMON.contains(&base)
}

fn cluster_functions(functions: &[FunctionInfo], affinity: &[Vec<i32>]) -> Vec<Cluster> {
    let len = functions.len();
    let mut cluster_id: Vec<Option<usize>> = vec![None; len];
    let mut clusters: Vec<Cluster> = Vec::new();

    const THRESHOLD: i32 = 10;
    const MAX_CLUSTER_SIZE: usize = 100;

    let mut pairs: Vec<(usize, usize, i32)> = Vec::new();
    for (index_a, row) in affinity.iter().enumerate() {
        for (index_b, &score) in row.iter().enumerate().skip(index_a + 1) {
            if score >= THRESHOLD {
                pairs.push((index_a, index_b, score));
            }
        }
    }

    pairs.sort_by(|a, b| b.2.cmp(&a.2));

    for (index_a, index_b, _score) in pairs {
        match (cluster_id[index_a], cluster_id[index_b]) {
            (None, None) => {
                let id = clusters.len();
                cluster_id[index_a] = Some(id);
                cluster_id[index_b] = Some(id);
                clusters.push(Cluster {
                    functions: vec![index_a, index_b],
                    label: String::new(),
                    dominant_file: None,
                    dominant_impl: None,
                });
            }
            (Some(id), None) => {
                if clusters[id].functions.len() < MAX_CLUSTER_SIZE {
                    cluster_id[index_b] = Some(id);
                    clusters[id].functions.push(index_b);
                }
            }
            (None, Some(id)) => {
                if clusters[id].functions.len() < MAX_CLUSTER_SIZE {
                    cluster_id[index_a] = Some(id);
                    clusters[id].functions.push(index_a);
                }
            }
            (Some(id_a), Some(id_b)) if id_a != id_b => {
                let combined_size = clusters[id_a].functions.len() + clusters[id_b].functions.len();
                if combined_size <= MAX_CLUSTER_SIZE {
                    if clusters[id_a].functions.len() >= clusters[id_b].functions.len() {
                        for &func_idx in &clusters[id_b].functions.clone() {
                            cluster_id[func_idx] = Some(id_a);
                            clusters[id_a].functions.push(func_idx);
                        }
                        clusters[id_b].functions.clear();
                    } else {
                        for &func_idx in &clusters[id_a].functions.clone() {
                            cluster_id[func_idx] = Some(id_b);
                            clusters[id_b].functions.push(func_idx);
                        }
                        clusters[id_a].functions.clear();
                    }
                }
            }
            _ => {}
        }
    }

    for cluster in &mut clusters {
        if cluster.functions.is_empty() {
            continue;
        }

        let (dominant_file, dominant_impl) = find_dominant_attributes(functions, cluster);
        cluster.dominant_file = dominant_file.clone();
        cluster.dominant_impl = dominant_impl.clone();
        cluster.label = generate_cluster_label(functions, cluster, &dominant_file, &dominant_impl);
    }

    clusters.retain(|c| !c.functions.is_empty());
    clusters
}

fn find_dominant_attributes(
    functions: &[FunctionInfo],
    cluster: &Cluster,
) -> (Option<String>, Option<String>) {
    let mut file_counts: HashMap<&str, usize> = HashMap::new();
    let mut impl_counts: HashMap<&str, usize> = HashMap::new();

    for &func_idx in &cluster.functions {
        let func = &functions[func_idx];
        *file_counts.entry(&func.file).or_insert(0) += 1;
        if let Some(ref impl_type) = func.impl_type {
            *impl_counts.entry(impl_type).or_insert(0) += 1;
        }
    }

    let dominant_file = file_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(file, _)| file.to_string());

    let dominant_impl = impl_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(impl_type, _)| impl_type.to_string());

    (dominant_file, dominant_impl)
}

fn generate_cluster_label(
    functions: &[FunctionInfo],
    cluster: &Cluster,
    dominant_file: &Option<String>,
    dominant_impl: &Option<String>,
) -> String {
    if let Some(impl_type) = dominant_impl {
        let impl_count = cluster
            .functions
            .iter()
            .filter(|&&idx| functions[idx].impl_type.as_deref() == Some(impl_type))
            .count();

        if impl_count >= cluster.functions.len() / 2 {
            return format!("{} methods", impl_type);
        }
    }

    if let Some(file) = dominant_file {
        let file_count = cluster
            .functions
            .iter()
            .filter(|&&idx| functions[idx].file == *file)
            .count();

        if file_count >= cluster.functions.len() / 2 {
            return generate_file_based_label(file, functions, cluster);
        }
    }

    infer_label_from_function_names(functions, cluster)
}

fn generate_file_based_label(file: &str, functions: &[FunctionInfo], cluster: &Cluster) -> String {
    let file_name = file.rsplit('/').next().unwrap_or(file);
    let module_name = file_name.trim_end_matches(".rs");

    let parent_dir = file
        .rsplit_once('/')
        .and_then(|(parent, _)| parent.rsplit('/').next())
        .unwrap_or("");

    if !parent_dir.is_empty() && parent_dir != "src" {
        return format!("{}/{}", parent_dir, module_name);
    }

    let common_prefix = find_common_function_prefix(functions, cluster);
    if !common_prefix.is_empty() && common_prefix.len() >= 3 {
        return common_prefix;
    }

    module_name.to_string()
}

fn find_common_function_prefix(functions: &[FunctionInfo], cluster: &Cluster) -> String {
    let names: Vec<&str> = cluster
        .functions
        .iter()
        .map(|&idx| functions[idx].name.as_str())
        .collect();

    if names.is_empty() {
        return String::new();
    }

    let first = names[0];
    let mut prefix_len = 0;

    for char_idx in 0..first.len() {
        let char_at_idx = first.chars().nth(char_idx);
        let all_match = names
            .iter()
            .all(|name| name.chars().nth(char_idx) == char_at_idx);

        if all_match {
            prefix_len = char_idx + 1;
        } else {
            break;
        }
    }

    let prefix = &first[..prefix_len];
    prefix.trim_end_matches('_').to_string()
}

fn infer_label_from_function_names(functions: &[FunctionInfo], cluster: &Cluster) -> String {
    let mut keyword_counts: HashMap<&str, usize> = HashMap::new();

    for &func_idx in &cluster.functions {
        let name = &functions[func_idx].name;
        for keyword in extract_keywords(name) {
            *keyword_counts.entry(keyword).or_insert(0) += 1;
        }
    }

    keyword_counts
        .into_iter()
        .filter(|(_, count)| *count >= cluster.functions.len() / 3)
        .max_by_key(|(_, count)| *count)
        .map(|(keyword, _)| keyword.to_string())
        .unwrap_or_else(|| "Related functions".to_string())
}

fn extract_keywords(name: &str) -> Vec<&'static str> {
    let lower = name.to_lowercase();
    let mut keywords = Vec::new();

    const KEYWORDS: &[&str] = &[
        "parse", "extract", "write", "read", "build", "create", "find", "get", "set", "check",
        "validate", "process", "handle", "format", "collect", "generate", "load", "save", "init",
        "new", "update", "delete", "insert", "remove",
    ];

    for &kw in KEYWORDS {
        if lower.contains(kw) {
            keywords.push(kw);
        }
    }

    keywords
}

fn count_internal_calls(
    cluster: &Cluster,
    functions: &[FunctionInfo],
    call_graph: &HashMap<String, HashSet<String>>,
) -> usize {
    let cluster_names: HashSet<String> = cluster
        .functions
        .iter()
        .map(|&idx| qualified_name(&functions[idx]))
        .collect();

    let mut count = 0;
    for &func_idx in &cluster.functions {
        let name = qualified_name(&functions[func_idx]);
        if let Some(targets) = call_graph.get(&name) {
            for target in targets {
                if cluster_names.contains(target) {
                    count += 1;
                }
            }
        }
    }

    count
}

fn count_external_calls(
    cluster: &Cluster,
    functions: &[FunctionInfo],
    call_graph: &HashMap<String, HashSet<String>>,
) -> usize {
    let cluster_names: HashSet<String> = cluster
        .functions
        .iter()
        .map(|&idx| qualified_name(&functions[idx]))
        .collect();

    let mut count = 0;
    for &func_idx in &cluster.functions {
        let name = qualified_name(&functions[func_idx]);
        if let Some(targets) = call_graph.get(&name) {
            for target in targets {
                if !cluster_names.contains(target) {
                    count += 1;
                }
            }
        }
    }

    count
}
