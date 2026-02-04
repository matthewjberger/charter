use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::extract::symbols::SymbolKind;
use crate::pipeline::PipelineResult;

struct TypeFlow {
    type_name: String,
    producers: Vec<ProducerInfo>,
    consumers: Vec<ConsumerInfo>,
}

struct ProducerInfo {
    function: String,
    file: String,
    line: usize,
}

struct ConsumerInfo {
    function: String,
    file: String,
    line: usize,
}

struct CrossModuleFlow {
    from_module: String,
    to_module: String,
    types: Vec<String>,
}

pub async fn write_dataflow(
    charter_dir: &Path,
    result: &PipelineResult,
    stamp: &str,
) -> Result<()> {
    let file = tokio::fs::File::create(charter_dir.join("dataflow.md")).await?;
    let mut writer = BufWriter::new(file);

    writer.write_all(stamp.as_bytes()).await?;
    writer.write_all(b"\n\n").await?;
    writer.write_all(b"# Data Flow\n\n").await?;
    writer
        .write_all(b"Type flows and cross-module connections.\n\n")
        .await?;

    let type_flows = build_type_flows(result);
    let cross_module_flows = build_cross_module_flows(result);

    if !type_flows.is_empty() {
        writer.write_all(b"## Type Flows\n\n").await?;
        writer
            .write_all(b"Types produced and consumed by functions.\n\n")
            .await?;

        let mut flows: Vec<&TypeFlow> = type_flows.values().collect();
        flows.sort_by(|a, b| {
            let a_score = a.producers.len() + a.consumers.len();
            let b_score = b.producers.len() + b.consumers.len();
            b_score
                .cmp(&a_score)
                .then_with(|| a.type_name.cmp(&b.type_name))
        });

        for flow in flows.iter().take(30) {
            if flow.producers.is_empty() && flow.consumers.is_empty() {
                continue;
            }
            if flow.producers.len() < 2 && flow.consumers.len() < 2 {
                continue;
            }

            let header = format!("{}\n", flow.type_name);
            writer.write_all(header.as_bytes()).await?;

            if !flow.producers.is_empty() {
                let producers: Vec<String> = flow
                    .producers
                    .iter()
                    .take(5)
                    .map(|p| format!("{} ({}:{})", p.function, p.file, p.line))
                    .collect();

                let more = if flow.producers.len() > 5 {
                    format!(" [+{} more]", flow.producers.len() - 5)
                } else {
                    String::new()
                };

                let line = format!("  produced by: {}{}\n", producers.join(", "), more);
                writer.write_all(line.as_bytes()).await?;
            }

            if !flow.consumers.is_empty() {
                let consumers: Vec<String> = flow
                    .consumers
                    .iter()
                    .take(5)
                    .map(|c| format!("{} ({}:{})", c.function, c.file, c.line))
                    .collect();

                let more = if flow.consumers.len() > 5 {
                    format!(" [+{} more]", flow.consumers.len() - 5)
                } else {
                    String::new()
                };

                let line = format!("  consumed by: {}{}\n", consumers.join(", "), more);
                writer.write_all(line.as_bytes()).await?;
            }

            writer.write_all(b"\n").await?;
        }
    }

    if !cross_module_flows.is_empty() {
        writer.write_all(b"## Cross-Module Type Flows\n\n").await?;
        writer
            .write_all(b"Types flowing between modules (shared types suggest coupling).\n\n")
            .await?;

        for flow in cross_module_flows.iter().take(25) {
            let header = format!("{} → {}\n", flow.from_module, flow.to_module);
            writer.write_all(header.as_bytes()).await?;

            let types_str = if flow.types.len() <= 4 {
                flow.types.join(", ")
            } else {
                let shown: Vec<&str> = flow.types.iter().take(3).map(|s| s.as_str()).collect();
                format!("{} [+{} more]", shown.join(", "), flow.types.len() - 3)
            };

            let line = format!("  via: {}\n\n", types_str);
            writer.write_all(line.as_bytes()).await?;
        }
    }

    if type_flows.is_empty() && cross_module_flows.is_empty() {
        writer
            .write_all(b"No significant data flow patterns detected.\n")
            .await?;
    }

    writer.flush().await?;
    Ok(())
}

fn build_type_flows(result: &PipelineResult) -> HashMap<String, TypeFlow> {
    let mut flows: HashMap<String, TypeFlow> = HashMap::new();

    let defined_types: HashSet<String> = result
        .files
        .iter()
        .flat_map(|f| &f.parsed.symbols.symbols)
        .filter_map(|s| match &s.kind {
            SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => Some(s.name.clone()),
            _ => None,
        })
        .collect();

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Function { signature, .. } = &symbol.kind {
                let (return_type, param_types) = parse_signature_types(signature);

                if let Some(ref ret_type) = return_type {
                    let base_type = extract_base_type(ret_type);
                    if defined_types.contains(&base_type) && !is_common_type(&base_type) {
                        flows.entry(base_type.clone()).or_insert_with(|| TypeFlow {
                            type_name: base_type.clone(),
                            producers: Vec::new(),
                            consumers: Vec::new(),
                        });

                        flows
                            .get_mut(&base_type)
                            .unwrap()
                            .producers
                            .push(ProducerInfo {
                                function: symbol.name.clone(),
                                file: file_result.relative_path.clone(),
                                line: symbol.line,
                            });
                    }
                }

                for param_type in &param_types {
                    let base_type = extract_base_type(param_type);
                    if defined_types.contains(&base_type) && !is_common_type(&base_type) {
                        flows.entry(base_type.clone()).or_insert_with(|| TypeFlow {
                            type_name: base_type.clone(),
                            producers: Vec::new(),
                            consumers: Vec::new(),
                        });

                        flows
                            .get_mut(&base_type)
                            .unwrap()
                            .consumers
                            .push(ConsumerInfo {
                                function: symbol.name.clone(),
                                file: file_result.relative_path.clone(),
                                line: symbol.line,
                            });
                    }
                }
            }
        }

        for imp in &file_result.parsed.symbols.inherent_impls {
            for method in &imp.methods {
                let (return_type, param_types) = parse_signature_types(&method.signature);

                if let Some(ref ret_type) = return_type {
                    let base_type = extract_base_type(ret_type);
                    if defined_types.contains(&base_type) && !is_common_type(&base_type) {
                        flows.entry(base_type.clone()).or_insert_with(|| TypeFlow {
                            type_name: base_type.clone(),
                            producers: Vec::new(),
                            consumers: Vec::new(),
                        });

                        let qualified = format!("{}::{}", imp.type_name, method.name);
                        flows
                            .get_mut(&base_type)
                            .unwrap()
                            .producers
                            .push(ProducerInfo {
                                function: qualified,
                                file: file_result.relative_path.clone(),
                                line: method.line,
                            });
                    }
                }

                for param_type in &param_types {
                    let base_type = extract_base_type(param_type);
                    if defined_types.contains(&base_type) && !is_common_type(&base_type) {
                        flows.entry(base_type.clone()).or_insert_with(|| TypeFlow {
                            type_name: base_type.clone(),
                            producers: Vec::new(),
                            consumers: Vec::new(),
                        });

                        let qualified = format!("{}::{}", imp.type_name, method.name);
                        flows
                            .get_mut(&base_type)
                            .unwrap()
                            .consumers
                            .push(ConsumerInfo {
                                function: qualified,
                                file: file_result.relative_path.clone(),
                                line: method.line,
                            });
                    }
                }
            }
        }
    }

    for flow in flows.values_mut() {
        flow.producers
            .sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
        flow.consumers
            .sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
        flow.producers
            .dedup_by(|a, b| a.function == b.function && a.file == b.file);
        flow.consumers
            .dedup_by(|a, b| a.function == b.function && a.file == b.file);
    }

    flows
}

fn parse_signature_types(signature: &str) -> (Option<String>, Vec<String>) {
    let return_type = if let Some(arrow_pos) = signature.rfind("->") {
        let ret = signature[arrow_pos + 2..].trim();
        if !ret.is_empty() && ret != "()" {
            Some(ret.to_string())
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
                    param_types.push(type_part.to_string());
                }
            }
        }
    }

    (return_type, param_types)
}

fn build_cross_module_flows(result: &PipelineResult) -> Vec<CrossModuleFlow> {
    let mut flows: Vec<CrossModuleFlow> = Vec::new();

    let type_to_file: HashMap<String, String> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .symbols
                .symbols
                .iter()
                .filter_map(|s| match &s.kind {
                    SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => {
                        Some((s.name.clone(), f.relative_path.clone()))
                    }
                    _ => None,
                })
        })
        .collect();

    let mut module_connections: HashMap<(String, String), HashSet<String>> = HashMap::new();

    for file_result in &result.files {
        let source_module = extract_module_name(&file_result.relative_path);

        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Function { signature, .. } = &symbol.kind {
                let (return_type, param_types) = parse_signature_types(signature);

                if let Some(ref ret) = return_type {
                    let base = extract_base_type(ret);
                    if let Some(type_file) = type_to_file.get(&base) {
                        let type_module = extract_module_name(type_file);
                        if type_module != source_module && !is_common_type(&base) {
                            let key = (source_module.clone(), type_module.clone());
                            module_connections.entry(key).or_default().insert(base);
                        }
                    }
                }

                for param in &param_types {
                    let base = extract_base_type(param);
                    if let Some(type_file) = type_to_file.get(&base) {
                        let type_module = extract_module_name(type_file);
                        if type_module != source_module && !is_common_type(&base) {
                            let key = (type_module.clone(), source_module.clone());
                            module_connections.entry(key).or_default().insert(base);
                        }
                    }
                }
            }
        }
    }

    for ((from_module, to_module), types) in module_connections {
        if types.len() >= 2 {
            flows.push(CrossModuleFlow {
                from_module,
                to_module,
                types: types.into_iter().collect(),
            });
        }
    }

    flows.sort_by(|a, b| {
        b.types
            .len()
            .cmp(&a.types.len())
            .then_with(|| a.from_module.cmp(&b.from_module))
    });

    flows
}

fn extract_module_name(file_path: &str) -> String {
    file_path
        .trim_start_matches("src/")
        .trim_end_matches(".rs")
        .replace('/', "::")
}

fn extract_base_type(type_str: &str) -> String {
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

fn is_common_type(type_name: &str) -> bool {
    const COMMON: &[&str] = &[
        "bool", "char", "str", "String", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
        "u32", "u64", "u128", "usize", "f32", "f64", "Self", "Option", "Result", "Vec", "Box",
        "Arc", "Rc", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Path", "PathBuf", "Error",
        "Cow", "Pin", "Future",
    ];

    COMMON.contains(&type_name)
}
