use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn deps(root: &Path, crate_filter: Option<&str>) -> Result<()> {
    let atlas_dir = root.join(".atlas");

    if !atlas_dir.exists() {
        eprintln!("No .atlas/ directory found. Run 'atlas' first.");
        std::process::exit(1);
    }

    let cargo_deps = parse_cargo_toml(root).await;
    let import_usage = analyze_imports(&atlas_dir).await?;

    if let Some(krate) = crate_filter {
        show_crate_usage(&import_usage, krate, &cargo_deps);
    } else {
        show_all_deps(&import_usage, &cargo_deps);
    }

    Ok(())
}

async fn parse_cargo_toml(root: &Path) -> HashMap<String, String> {
    let mut deps = HashMap::new();

    let cargo_path = root.join("Cargo.toml");
    let content = match fs::read_to_string(&cargo_path).await {
        Ok(c) => c,
        Err(_) => return deps,
    };

    let parsed: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return deps,
    };

    if let Some(dependencies) = parsed.get("dependencies").and_then(|d| d.as_table()) {
        for (name, value) in dependencies {
            let version = match value {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string(),
                _ => "*".to_string(),
            };
            deps.insert(name.clone(), version);
        }
    }

    if let Some(dev_deps) = parsed.get("dev-dependencies").and_then(|d| d.as_table()) {
        for (name, value) in dev_deps {
            let version = match value {
                toml::Value::String(s) => format!("{} (dev)", s),
                toml::Value::Table(t) => {
                    let ver = t.get("version").and_then(|v| v.as_str()).unwrap_or("*");
                    format!("{} (dev)", ver)
                }
                _ => "* (dev)".to_string(),
            };
            deps.insert(name.clone(), version);
        }
    }

    deps
}

struct CrateUsage {
    file_count: usize,
    import_count: usize,
    files: Vec<String>,
    items: Vec<String>,
}

async fn analyze_imports(atlas_dir: &Path) -> Result<HashMap<String, CrateUsage>> {
    let mut usage: HashMap<String, CrateUsage> = HashMap::new();

    let _symbols_content = fs::read_to_string(atlas_dir.join("symbols.md"))
        .await
        .unwrap_or_default();

    let overview_content = fs::read_to_string(atlas_dir.join("overview.md"))
        .await
        .unwrap_or_default();

    let mut in_imports = false;
    for line in overview_content.lines() {
        if line == "External crates:" || line == "Dependencies:" {
            in_imports = true;
            continue;
        }

        if in_imports && !line.starts_with(' ') && !line.is_empty() {
            break;
        }

        if in_imports && line.starts_with("  ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                let crate_name = parts[0].to_string();
                let entry = usage.entry(crate_name).or_insert(CrateUsage {
                    file_count: 0,
                    import_count: 0,
                    files: Vec::new(),
                    items: Vec::new(),
                });
                entry.import_count += 1;
            }
        }
    }

    let cache_path = atlas_dir.join("cache.bin");
    if cache_path.exists() {
        if let Ok(cache_data) = fs::read(&cache_path).await {
            if let Ok(cache) = bincode::deserialize::<crate::cache::Cache>(&cache_data) {
                for (file_path, entry) in &cache.entries {
                    for import in &entry.data.parsed.imports {
                        if let Some(crate_name) = extract_crate_name(&import.path) {
                            let entry = usage.entry(crate_name).or_insert(CrateUsage {
                                file_count: 0,
                                import_count: 0,
                                files: Vec::new(),
                                items: Vec::new(),
                            });
                            entry.import_count += 1;
                            if !entry.files.contains(file_path) {
                                entry.files.push(file_path.clone());
                                entry.file_count += 1;
                            }
                            let item = import.path.clone();
                            if !entry.items.contains(&item) && entry.items.len() < 20 {
                                entry.items.push(item);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(usage)
}

fn extract_crate_name(import_path: &str) -> Option<String> {
    let path = import_path.trim();

    if path.starts_with("crate::") || path.starts_with("self::") || path.starts_with("super::") {
        return None;
    }

    if path.starts_with("std::") || path.starts_with("core::") || path.starts_with("alloc::") {
        return None;
    }

    let first_segment = path.split("::").next()?;

    if first_segment.is_empty() {
        return None;
    }

    Some(first_segment.replace('_', "-"))
}

fn show_crate_usage(
    usage: &HashMap<String, CrateUsage>,
    krate: &str,
    cargo_deps: &HashMap<String, String>,
) {
    let crate_lower = krate.to_lowercase();
    let normalized = krate.replace('-', "_");

    let matching: Vec<_> = usage
        .iter()
        .filter(|(name, _)| {
            let name_lower = name.to_lowercase();
            name_lower == crate_lower || name.replace('-', "_") == normalized
        })
        .collect();

    if matching.is_empty() {
        println!("No usage found for crate '{}'", krate);

        let similar: Vec<_> = usage
            .keys()
            .filter(|name| name.to_lowercase().contains(&crate_lower))
            .collect();

        if !similar.is_empty() {
            println!("\nDid you mean one of these?");
            for name in similar.iter().take(5) {
                println!("  {}", name);
            }
        }
        return;
    }

    for (name, crate_usage) in matching {
        println!("Crate: {}", name);

        if let Some(version) = cargo_deps.get(name) {
            println!("Version: {}", version);
        }

        println!("Files using: {}", crate_usage.file_count);
        println!("Import count: {}", crate_usage.import_count);
        println!();

        if !crate_usage.files.is_empty() {
            println!("Files:");
            for file in crate_usage.files.iter().take(20) {
                println!("  {}", file);
            }
            if crate_usage.files.len() > 20 {
                println!("  ... and {} more", crate_usage.files.len() - 20);
            }
        }

        if !crate_usage.items.is_empty() {
            println!("\nImported items:");
            for item in &crate_usage.items {
                println!("  {}", item);
            }
        }
    }
}

fn show_all_deps(usage: &HashMap<String, CrateUsage>, cargo_deps: &HashMap<String, String>) {
    println!("External Dependencies");
    println!("====================");
    println!();

    let mut sorted: Vec<_> = usage.iter().collect();
    sorted.sort_by(|a, b| b.1.import_count.cmp(&a.1.import_count));

    let categories = categorize_deps(&sorted);

    for (category, deps) in &categories {
        if deps.is_empty() {
            continue;
        }

        println!("{}:", category);
        for (name, crate_usage) in deps {
            let version = cargo_deps
                .get(*name)
                .map(|v| format!(" ({})", v))
                .unwrap_or_default();
            println!(
                "  {}{} — {} files, {} imports",
                name, version, crate_usage.file_count, crate_usage.import_count
            );
        }
        println!();
    }

    let unused: Vec<_> = cargo_deps
        .iter()
        .filter(|(name, _)| {
            !usage.contains_key(name.as_str()) && !usage.contains_key(&name.replace('-', "_"))
        })
        .collect();

    if !unused.is_empty() {
        println!("Potentially unused (in Cargo.toml but no imports found):");
        for (name, version) in &unused {
            println!("  {} ({})", name, version);
        }
    }
}

fn categorize_deps<'a>(
    deps: &'a [(&'a String, &'a CrateUsage)],
) -> Vec<(&'static str, Vec<(&'a String, &'a CrateUsage)>)> {
    let mut categories: HashMap<&'static str, Vec<(&String, &CrateUsage)>> = HashMap::new();

    for (name, usage) in deps {
        let category = match name.as_str() {
            "serde" | "serde_json" | "serde_yaml" | "bincode" | "toml" | "ron" => "Serialization",
            "tokio" | "async-std" | "futures" | "async-trait" => "Async",
            "clap" | "structopt" | "argh" => "CLI",
            "anyhow" | "thiserror" | "eyre" | "color-eyre" => "Error Handling",
            "tracing" | "log" | "env_logger" | "pretty_env_logger" => "Logging",
            "reqwest" | "hyper" | "actix-web" | "axum" | "warp" | "rocket" => "HTTP",
            "sqlx" | "diesel" | "rusqlite" | "postgres" | "mongodb" => "Database",
            "regex" | "lazy_static" | "once_cell" => "Utilities",
            "rand" | "uuid" | "chrono" | "time" => "Data Types",
            _ => "Other",
        };

        categories.entry(category).or_default().push((name, usage));
    }

    let order = [
        "Async",
        "Error Handling",
        "Serialization",
        "CLI",
        "HTTP",
        "Database",
        "Logging",
        "Data Types",
        "Utilities",
        "Other",
    ];

    let mut result = Vec::new();
    for category in order {
        if let Some(deps) = categories.remove(category) {
            result.push((category, deps));
        }
    }

    result
}
