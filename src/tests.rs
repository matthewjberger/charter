use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn tests(root: &Path, file_filter: Option<&str>) -> Result<()> {
    let charter_dir = root.join(".charter");

    if !charter_dir.exists() {
        eprintln!("No .charter/ directory found. Run 'charter' first.");
        std::process::exit(1);
    }

    let mapping = build_test_mapping(&charter_dir).await?;

    if let Some(file) = file_filter {
        show_tests_for_file(&mapping, file);
    } else {
        show_all_mappings(&mapping);
    }

    Ok(())
}

struct TestMapping {
    source_file: String,
    test_files: Vec<String>,
    test_functions: Vec<String>,
    coverage_estimate: CoverageLevel,
}

#[derive(Clone, Copy)]
enum CoverageLevel {
    High,
    Medium,
    Low,
    None,
}

impl std::fmt::Display for CoverageLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoverageLevel::High => write!(f, "high"),
            CoverageLevel::Medium => write!(f, "medium"),
            CoverageLevel::Low => write!(f, "low"),
            CoverageLevel::None => write!(f, "none"),
        }
    }
}

async fn build_test_mapping(charter_dir: &Path) -> Result<HashMap<String, TestMapping>> {
    let mut mappings: HashMap<String, TestMapping> = HashMap::new();

    let mut source_files: Vec<String> = Vec::new();
    let mut test_files: Vec<String> = Vec::new();
    let mut test_functions: HashMap<String, Vec<String>> = HashMap::new();

    let cache_path = charter_dir.join("cache.bin");
    if cache_path.exists() {
        if let Ok(cache_data) = fs::read(&cache_path).await {
            if let Ok(cache) = bincode::deserialize::<crate::cache::Cache>(&cache_data) {
                for (file_path, entry) in &cache.entries {
                    let is_test_file = file_path.contains("/tests/")
                        || file_path.contains("\\tests\\")
                        || file_path.ends_with("_test.rs")
                        || file_path.ends_with("_tests.rs");

                    if is_test_file {
                        test_files.push(file_path.clone());
                        test_functions
                            .insert(file_path.clone(), entry.data.parsed.test_functions.clone());
                    } else if file_path.ends_with(".rs") {
                        source_files.push(file_path.clone());
                    }

                    if entry.data.parsed.has_test_module {
                        let inline_tests: Vec<String> = entry.data.parsed.test_functions.to_vec();
                        if !inline_tests.is_empty() {
                            let mapping =
                                mappings.entry(file_path.clone()).or_insert(TestMapping {
                                    source_file: file_path.clone(),
                                    test_files: Vec::new(),
                                    test_functions: Vec::new(),
                                    coverage_estimate: CoverageLevel::None,
                                });
                            mapping.test_functions.extend(inline_tests);
                        }
                    }
                }
            }
        }
    }

    for source_file in &source_files {
        let mapping = mappings.entry(source_file.clone()).or_insert(TestMapping {
            source_file: source_file.clone(),
            test_files: Vec::new(),
            test_functions: Vec::new(),
            coverage_estimate: CoverageLevel::None,
        });

        let source_stem = extract_stem(source_file);

        for test_file in &test_files {
            if test_matches_source(test_file, source_file, &source_stem) {
                if !mapping.test_files.contains(test_file) {
                    mapping.test_files.push(test_file.clone());
                }

                if let Some(funcs) = test_functions.get(test_file) {
                    for func in funcs {
                        if !mapping.test_functions.contains(func) {
                            mapping.test_functions.push(func.clone());
                        }
                    }
                }
            }
        }

        mapping.coverage_estimate = estimate_coverage(mapping);
    }

    Ok(mappings)
}

fn extract_stem(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let file_name = normalized.rsplit('/').next().unwrap_or("");
    file_name.trim_end_matches(".rs").to_string()
}

fn test_matches_source(test_file: &str, source_file: &str, source_stem: &str) -> bool {
    let test_normalized = test_file.replace('\\', "/").to_lowercase();
    let source_normalized = source_file.replace('\\', "/").to_lowercase();
    let stem_lower = source_stem.to_lowercase();

    if test_normalized.contains(&format!("{}_test", stem_lower))
        || test_normalized.contains(&format!("{}_tests", stem_lower))
        || test_normalized.contains(&format!("test_{}", stem_lower))
    {
        return true;
    }

    let _source_module = source_normalized
        .trim_start_matches("src/")
        .trim_end_matches(".rs")
        .replace('/', "::");

    let test_content_check = test_normalized.contains(&stem_lower);

    if test_content_check {
        let in_same_module = {
            let source_dir = source_normalized
                .rsplit_once('/')
                .map(|(d, _)| d)
                .unwrap_or("");
            let test_dir = test_normalized
                .rsplit_once('/')
                .map(|(d, _)| d)
                .unwrap_or("");
            source_dir == test_dir || test_dir.ends_with("/tests")
        };
        if in_same_module {
            return true;
        }
    }

    false
}

fn estimate_coverage(mapping: &TestMapping) -> CoverageLevel {
    let test_count = mapping.test_functions.len();
    let has_dedicated_file = !mapping.test_files.is_empty();

    if test_count >= 5 || (has_dedicated_file && test_count >= 3) {
        CoverageLevel::High
    } else if test_count >= 2 || has_dedicated_file {
        CoverageLevel::Medium
    } else if test_count >= 1 {
        CoverageLevel::Low
    } else {
        CoverageLevel::None
    }
}

fn show_tests_for_file(mappings: &HashMap<String, TestMapping>, file: &str) {
    let file_lower = file.to_lowercase();
    let file_normalized = file.replace('\\', "/");

    let matching: Vec<_> = mappings
        .iter()
        .filter(|(path, _)| {
            let path_lower = path.to_lowercase();
            path_lower.contains(&file_lower) || path_lower.ends_with(&file_normalized)
        })
        .collect();

    if matching.is_empty() {
        println!("No source file found matching '{}'", file);

        let similar: Vec<_> = mappings
            .keys()
            .filter(|path| {
                let path_lower = path.to_lowercase();
                let file_stem = file_lower.trim_end_matches(".rs");
                path_lower.contains(file_stem)
            })
            .take(5)
            .collect();

        if !similar.is_empty() {
            println!("\nDid you mean one of these?");
            for path in similar {
                println!("  {}", path);
            }
        }
        return;
    }

    for (path, mapping) in matching {
        println!("Tests for: {}", path);
        println!("Coverage estimate: {}", mapping.coverage_estimate);
        println!();

        if !mapping.test_files.is_empty() {
            println!("Test files:");
            for test_file in &mapping.test_files {
                println!("  {}", test_file);
            }
            println!();
        }

        if !mapping.test_functions.is_empty() {
            println!("Test functions ({}):", mapping.test_functions.len());
            for func in mapping.test_functions.iter().take(20) {
                println!("  {}", func);
            }
            if mapping.test_functions.len() > 20 {
                println!("  ... and {} more", mapping.test_functions.len() - 20);
            }
        } else {
            println!("No test functions found");
        }

        println!();
    }
}

fn show_all_mappings(mappings: &HashMap<String, TestMapping>) {
    println!("Test Coverage Mapping");
    println!("====================");
    println!();

    let mut by_coverage: HashMap<&'static str, Vec<&TestMapping>> = HashMap::new();
    by_coverage.insert("high", Vec::new());
    by_coverage.insert("medium", Vec::new());
    by_coverage.insert("low", Vec::new());
    by_coverage.insert("none", Vec::new());

    for mapping in mappings.values() {
        let key = match mapping.coverage_estimate {
            CoverageLevel::High => "high",
            CoverageLevel::Medium => "medium",
            CoverageLevel::Low => "low",
            CoverageLevel::None => "none",
        };
        by_coverage.get_mut(key).unwrap().push(mapping);
    }

    for (level, files) in [
        ("High coverage", by_coverage.get("high").unwrap()),
        ("Medium coverage", by_coverage.get("medium").unwrap()),
        ("Low coverage", by_coverage.get("low").unwrap()),
    ] {
        if files.is_empty() {
            continue;
        }

        println!("{} ({} files):", level, files.len());
        for mapping in files.iter().take(20) {
            let test_count = mapping.test_functions.len();
            let file_count = mapping.test_files.len();
            println!(
                "  {} — {} tests, {} test files",
                mapping.source_file, test_count, file_count
            );
        }
        if files.len() > 20 {
            println!("  ... and {} more", files.len() - 20);
        }
        println!();
    }

    let untested = by_coverage.get("none").unwrap();
    if !untested.is_empty() {
        println!("No tests found ({} files):", untested.len());
        for mapping in untested.iter().take(30) {
            println!("  {}", mapping.source_file);
        }
        if untested.len() > 30 {
            println!("  ... and {} more", untested.len() - 30);
        }
    }

    println!();
    println!("Summary:");
    println!(
        "  High coverage: {} files",
        by_coverage.get("high").unwrap().len()
    );
    println!(
        "  Medium coverage: {} files",
        by_coverage.get("medium").unwrap().len()
    );
    println!(
        "  Low coverage: {} files",
        by_coverage.get("low").unwrap().len()
    );
    println!(
        "  No tests: {} files",
        by_coverage.get("none").unwrap().len()
    );
}
