use anyhow::Result;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone)]
pub struct ExternalCrate {
    pub name: String,
    pub version: String,
    pub source_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalSymbol {
    pub name: String,
    pub kind: String,
    pub crate_name: String,
    pub file: String,
    pub line: usize,
    pub signature: Option<String>,
}

thread_local! {
    static EXT_PARSER: RefCell<Parser> = RefCell::new({
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).expect("Rust grammar");
        parser.set_timeout_micros(5_000_000);
        parser
    });
}

fn find_cargo_lock(root: &Path) -> Result<PathBuf> {
    let mut dir = root.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.lock");
        if candidate.exists() {
            return Ok(candidate);
        }
        if !dir.pop() {
            return Err(anyhow::anyhow!("Cargo.lock not found"));
        }
    }
}

pub fn parse_cargo_lock(root: &Path) -> Result<Vec<(String, String)>> {
    let lock_path = find_cargo_lock(root)?;
    let content = std::fs::read_to_string(&lock_path)?;
    let mut packages = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut is_registry = false;

    for line in content.lines() {
        if line == "[[package]]" {
            if let (Some(name), Some(version)) = (current_name.take(), current_version.take()) {
                if is_registry {
                    packages.push((name, version));
                }
            }
            current_name = None;
            current_version = None;
            is_registry = false;
            continue;
        }

        if let Some(rest) = line.strip_prefix("name = \"") {
            current_name = rest.strip_suffix('"').map(String::from);
        } else if let Some(rest) = line.strip_prefix("version = \"") {
            current_version = rest.strip_suffix('"').map(String::from);
        } else if let Some(rest) = line.strip_prefix("source = \"") {
            is_registry = rest.contains("registry+");
        }
    }

    if let (Some(name), Some(version)) = (current_name, current_version) {
        if is_registry {
            packages.push((name, version));
        }
    }

    Ok(packages)
}

pub fn locate_registry_source(name: &str, version: &str) -> Option<PathBuf> {
    let cargo_home = std::env::var("CARGO_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs_fallback)?;

    let registry_src = cargo_home.join("registry").join("src");
    if !registry_src.exists() {
        return None;
    }

    let dir_name = format!("{}-{}", name, version);

    if let Ok(entries) = std::fs::read_dir(&registry_src) {
        for entry in entries.flatten() {
            let index_dir = entry.path();
            let crate_dir = index_dir.join(&dir_name);
            if crate_dir.is_dir() {
                return Some(crate_dir);
            }
        }
    }

    None
}

fn dirs_fallback() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(|profile| PathBuf::from(profile).join(".cargo"))
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".cargo"))
    }
}

pub fn collect_external_crates(
    root: &Path,
    direct_deps: &HashSet<String>,
) -> Vec<ExternalCrate> {
    let lock_packages = match parse_cargo_lock(root) {
        Ok(packages) => packages,
        Err(_) => return Vec::new(),
    };

    lock_packages
        .into_iter()
        .filter(|(name, _)| direct_deps.contains(name))
        .filter_map(|(name, version)| {
            let source_dir = locate_registry_source(&name, &version)?;
            Some(ExternalCrate {
                name,
                version,
                source_dir,
            })
        })
        .collect()
}

pub fn parse_direct_deps(root: &Path) -> HashSet<String> {
    let mut deps = HashSet::new();
    extract_deps_from_toml(&root.join("Cargo.toml"), &mut deps);
    let mut dir = root.to_path_buf();
    while dir.pop() {
        let parent_toml = dir.join("Cargo.toml");
        if parent_toml.exists() {
            extract_deps_from_toml(&parent_toml, &mut deps);
        } else {
            break;
        }
    }
    deps
}

fn extract_deps_from_toml(path: &Path, deps: &mut HashSet<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut in_deps_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps_section = trimmed == "[dependencies]"
                || trimmed.starts_with("[dependencies.")
                || trimmed == "[dev-dependencies]"
                || trimmed.starts_with("[dev-dependencies.");
            continue;
        }
        if in_deps_section {
            if let Some(name) = trimmed.split('=').next() {
                let name = name.trim();
                if !name.is_empty() && !name.starts_with('#') {
                    deps.insert(name.replace('-', "_"));
                    deps.insert(name.to_string());
                }
            }
        }
    }
}

const MAX_EXTERNAL_SOURCE_FILES: usize = 200;

pub fn extract_external_symbols(crates: &[ExternalCrate]) -> Vec<ExternalSymbol> {
    let mut symbols = Vec::new();

    for ext_crate in crates {
        let source_files = collect_rs_files(&ext_crate.source_dir);
        if source_files.len() > MAX_EXTERNAL_SOURCE_FILES {
            continue;
        }

        for file_path in &source_files {
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let relative = file_path
                .strip_prefix(&ext_crate.source_dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .replace('\\', "/");

            let file_symbols = parse_external_file(&content, &ext_crate.name, &relative);
            symbols.extend(file_symbols);
        }
    }

    symbols
}

fn collect_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rs_files_recursive(dir, &mut files);
    files
}

fn collect_rs_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files_recursive(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn parse_external_file(
    content: &str,
    crate_name: &str,
    relative_path: &str,
) -> Vec<ExternalSymbol> {
    EXT_PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        let tree = match parser.parse(content, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut symbols = Vec::new();
        let root = tree.root_node();
        let source = content.as_bytes();
        extract_pub_symbols(&root, source, crate_name, relative_path, &mut symbols);
        symbols
    })
}

fn extract_pub_symbols(
    node: &Node,
    source: &[u8],
    crate_name: &str,
    file: &str,
    symbols: &mut Vec<ExternalSymbol>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_item" | "enum_item" | "trait_item" | "type_item" | "const_item"
            | "static_item" | "function_item" => {
                if !is_pub(&child, source) {
                    continue;
                }
                let (kind, name, signature) = extract_item_info(&child, source);
                if let Some(name) = name {
                    symbols.push(ExternalSymbol {
                        name,
                        kind,
                        crate_name: crate_name.to_string(),
                        file: file.to_string(),
                        line: child.start_position().row + 1,
                        signature,
                    });
                }
            }
            "impl_item" => {
                extract_pub_impl_methods(&child, source, crate_name, file, symbols);
            }
            _ => {}
        }
    }
}

fn is_pub(node: &Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text_str(&child, source);
            return text.starts_with("pub");
        }
    }
    false
}

fn extract_item_info(node: &Node, source: &[u8]) -> (String, Option<String>, Option<String>) {
    let kind = match node.kind() {
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        "type_item" => "type",
        "const_item" => "const",
        "static_item" => "static",
        "function_item" => "function",
        other => other,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text_str(&n, source).to_string());

    let signature = if kind == "function" {
        let full = node_text_str(node, source);
        full.find('{')
            .map(|brace_pos| full[..brace_pos].trim().to_string())
    } else if kind == "type" {
        Some(
            node_text_str(node, source)
                .trim_end_matches(';')
                .trim()
                .to_string(),
        )
    } else {
        None
    };

    (kind.to_string(), name, signature)
}

fn extract_pub_impl_methods(
    impl_node: &Node,
    source: &[u8],
    crate_name: &str,
    file: &str,
    symbols: &mut Vec<ExternalSymbol>,
) {
    let type_name = impl_node
        .child_by_field_name("type")
        .map(|n| node_text_str(&n, source).to_string());

    let body = match impl_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" && is_pub(&child, source) {
            let method_name = child
                .child_by_field_name("name")
                .map(|n| node_text_str(&n, source).to_string());

            if let Some(method_name) = method_name {
                let full = node_text_str(&child, source);
                let signature = full.find('{').map(|pos| full[..pos].trim().to_string());

                let qualified = match &type_name {
                    Some(t) => format!("{}::{}", t, method_name),
                    None => method_name,
                };

                symbols.push(ExternalSymbol {
                    name: qualified,
                    kind: "method".to_string(),
                    crate_name: crate_name.to_string(),
                    file: file.to_string(),
                    line: child.start_position().row + 1,
                    signature,
                });
            }
        }
    }
}

fn node_text_str<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}
