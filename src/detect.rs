use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;

pub async fn find_project_root(path: Option<PathBuf>) -> Result<PathBuf> {
    let start = match path {
        Some(p) => {
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir()?.join(p)
            }
        }
        None => std::env::current_dir()?,
    };

    let start = fs::canonicalize(&start)
        .await
        .with_context(|| format!("Failed to canonicalize path: {}", start.display()))?;

    if let Some(cargo_root) = find_cargo_root(&start).await {
        return Ok(cargo_root);
    }

    if let Some(git_root) = find_git_root(&start).await {
        if has_cargo_toml(&git_root).await {
            return Ok(git_root);
        }
    }

    Ok(start)
}

async fn find_cargo_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    let mut deepest_cargo = None;

    loop {
        if has_cargo_toml(&current).await {
            let cargo_path = current.join("Cargo.toml");
            if let Ok(content) = fs::read_to_string(&cargo_path).await {
                if content.contains("[workspace]") {
                    return Some(current);
                }
                if deepest_cargo.is_none() {
                    deepest_cargo = Some(current.clone());
                }
            }
        }
        if !current.pop() {
            break;
        }
    }

    deepest_cargo
}

async fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let git_dir = current.join(".git");
        if fs::metadata(&git_dir).await.is_ok() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

async fn has_cargo_toml(path: &Path) -> bool {
    fs::metadata(path.join("Cargo.toml")).await.is_ok()
}

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub root: PathBuf,
    pub members: Vec<CrateInfo>,
    pub is_workspace: bool,
}

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    pub path: PathBuf,
    pub crate_type: CrateType,
    pub dependencies: Vec<String>,
    pub features: Vec<FeatureInfo>,
    pub targets: Vec<TargetInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateType {
    Lib,
    Bin,
    ProcMacro,
}

#[derive(Debug, Clone)]
pub struct FeatureInfo {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub name: String,
    pub kind: TargetKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Lib,
    Bin,
    Example,
    Bench,
}

impl std::fmt::Display for CrateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrateType::Lib => write!(f, "lib"),
            CrateType::Bin => write!(f, "bin"),
            CrateType::ProcMacro => write!(f, "proc-macro"),
        }
    }
}

pub async fn detect_workspace(root: &Path) -> Result<WorkspaceInfo> {
    let cargo_toml_path = root.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml_path)
        .await
        .with_context(|| format!("Failed to read {}", cargo_toml_path.display()))?;

    let cargo_toml: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", cargo_toml_path.display()))?;

    let is_workspace = cargo_toml.get("workspace").is_some();

    if is_workspace {
        let members = parse_workspace_members(root, &cargo_toml).await?;
        Ok(WorkspaceInfo {
            root: root.to_path_buf(),
            members,
            is_workspace: true,
        })
    } else {
        let crate_info = parse_single_crate(root, &cargo_toml).await?;
        Ok(WorkspaceInfo {
            root: root.to_path_buf(),
            members: vec![crate_info],
            is_workspace: false,
        })
    }
}

async fn parse_workspace_members(root: &Path, cargo_toml: &toml::Value) -> Result<Vec<CrateInfo>> {
    let mut members = Vec::new();

    let workspace = cargo_toml.get("workspace");
    let member_patterns: Vec<&str> = workspace
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    for pattern in member_patterns {
        let member_paths = expand_glob_pattern(root, pattern).await;
        for member_path in member_paths {
            let member_cargo_toml = member_path.join("Cargo.toml");
            if fs::metadata(&member_cargo_toml).await.is_ok() {
                let content = fs::read_to_string(&member_cargo_toml).await?;
                let member_toml: toml::Value = toml::from_str(&content)?;
                let crate_info = parse_single_crate(&member_path, &member_toml).await?;
                members.push(crate_info);
            }
        }
    }

    if members.is_empty() {
        let crate_info = parse_single_crate(root, cargo_toml).await?;
        if !crate_info.name.is_empty() {
            members.push(crate_info);
        }
    }

    Ok(members)
}

async fn expand_glob_pattern(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut results = Vec::new();

    if pattern.contains('*') {
        let base_path = root.join(pattern.split('*').next().unwrap_or(""));
        if let Ok(mut entries) = fs::read_dir(&base_path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_dir() && path.join("Cargo.toml").exists() {
                    results.push(path);
                }
            }
        }
    } else {
        let full_path = root.join(pattern);
        if fs::metadata(&full_path).await.is_ok() {
            results.push(full_path);
        }
    }

    results
}

async fn parse_single_crate(path: &Path, cargo_toml: &toml::Value) -> Result<CrateInfo> {
    let package = cargo_toml.get("package");

    let name = package
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    let crate_type = determine_crate_type(path, cargo_toml).await;

    let dependencies = extract_dependencies(cargo_toml);
    let features = extract_features(cargo_toml);
    let targets = extract_targets(path, cargo_toml).await;

    Ok(CrateInfo {
        name,
        path: path.to_path_buf(),
        crate_type,
        dependencies,
        features,
        targets,
    })
}

async fn determine_crate_type(path: &Path, cargo_toml: &toml::Value) -> CrateType {
    if let Some(lib) = cargo_toml.get("lib") {
        if let Some(proc_macro) = lib.get("proc-macro") {
            if proc_macro.as_bool().unwrap_or(false) {
                return CrateType::ProcMacro;
            }
        }
    }

    if fs::metadata(path.join("src/lib.rs")).await.is_ok() {
        return CrateType::Lib;
    }

    if fs::metadata(path.join("src/main.rs")).await.is_ok() {
        return CrateType::Bin;
    }

    CrateType::Lib
}

fn extract_dependencies(cargo_toml: &toml::Value) -> Vec<String> {
    let mut deps = Vec::new();

    if let Some(dependencies) = cargo_toml.get("dependencies") {
        if let Some(table) = dependencies.as_table() {
            for key in table.keys() {
                deps.push(key.clone());
            }
        }
    }

    deps
}

fn extract_features(cargo_toml: &toml::Value) -> Vec<FeatureInfo> {
    let mut features = Vec::new();

    if let Some(features_table) = cargo_toml.get("features") {
        if let Some(table) = features_table.as_table() {
            for (name, _value) in table {
                features.push(FeatureInfo { name: name.clone() });
            }
        }
    }

    features
}

async fn extract_targets(path: &Path, cargo_toml: &toml::Value) -> Vec<TargetInfo> {
    let mut targets = Vec::new();
    let mut seen_bin_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    let package_name = cargo_toml
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    if fs::metadata(path.join("src/lib.rs")).await.is_ok() {
        targets.push(TargetInfo {
            name: package_name.clone(),
            kind: TargetKind::Lib,
            path: path.join("src/lib.rs"),
        });
    }

    if fs::metadata(path.join("src/main.rs")).await.is_ok() {
        let name = if package_name.is_empty() {
            "main".to_string()
        } else {
            package_name.clone()
        };

        if !seen_bin_names.contains(&name) {
            seen_bin_names.insert(name.clone());
            targets.push(TargetInfo {
                name,
                kind: TargetKind::Bin,
                path: path.join("src/main.rs"),
            });
        }
    }

    if let Some(bins) = cargo_toml.get("bin") {
        if let Some(arr) = bins.as_array() {
            for bin in arr {
                let name = bin
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();

                if name.is_empty() || seen_bin_names.contains(&name) {
                    continue;
                }

                seen_bin_names.insert(name.clone());
                let bin_path = bin
                    .get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| path.join(p))
                    .unwrap_or_else(|| path.join(format!("src/bin/{}.rs", name)));

                targets.push(TargetInfo {
                    name,
                    kind: TargetKind::Bin,
                    path: bin_path,
                });
            }
        }
    }

    if let Some(examples) = cargo_toml.get("example") {
        if let Some(arr) = examples.as_array() {
            for example in arr {
                let name = example
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let example_path = example
                    .get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| path.join(p))
                    .unwrap_or_else(|| path.join(format!("examples/{}.rs", name)));

                targets.push(TargetInfo {
                    name,
                    kind: TargetKind::Example,
                    path: example_path,
                });
            }
        }
    }

    if let Some(benches) = cargo_toml.get("bench") {
        if let Some(arr) = benches.as_array() {
            for bench in arr {
                let name = bench
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let bench_path = bench
                    .get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| path.join(p))
                    .unwrap_or_else(|| path.join(format!("benches/{}.rs", name)));

                targets.push(TargetInfo {
                    name,
                    kind: TargetKind::Bench,
                    path: bench_path,
                });
            }
        }
    }

    targets
}
