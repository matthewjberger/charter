pub mod calls;
pub mod clusters;
pub mod dataflow;
pub mod dependents;
pub mod errors;
pub mod hotspots;
pub mod manifest;
pub mod overview;
pub mod preamble;
pub mod refs;
pub mod safety;
pub mod skipped;
pub mod snippets;
pub mod symbols;
pub mod type_map;

use std::path::Path;

pub(crate) fn format_qualifiers(is_async: bool, is_unsafe: bool, is_const: bool) -> String {
    let mut parts = Vec::new();
    if is_const {
        parts.push("const");
    }
    if is_async {
        parts.push("async");
    }
    if is_unsafe {
        parts.push("unsafe");
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(" ") + " "
    }
}

pub(crate) fn file_role(path: &Path) -> &'static str {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if file_name == "Cargo.toml" || file_name == "Cargo.lock" {
        return "[build]";
    }

    if file_name == "build.rs" {
        return "[build]";
    }

    if file_name == "pyproject.toml" || file_name == "setup.py" || file_name == "setup.cfg" {
        return "[build]";
    }

    if file_name == "conftest.py" {
        return "[test-config]";
    }

    if file_name.ends_with("_test.rs")
        || file_name.ends_with("_tests.rs")
        || file_name.starts_with("test_")
        || file_name.ends_with("_test.py")
    {
        return "[test]";
    }

    if file_name.ends_with(".pyi") {
        return "[stub]";
    }

    let path_str = path.to_string_lossy();
    if path_str.contains("/tests/") || path_str.contains("\\tests\\") {
        return "[test]";
    }

    if path_str.contains("/benches/") || path_str.contains("\\benches\\") {
        return "[bench]";
    }

    if path_str.contains("/examples/") || path_str.contains("\\examples\\") {
        return "[example]";
    }

    if path_str.contains("/__pycache__/") {
        return "[cache]";
    }

    match extension {
        "rs" => "[source]",
        "py" => "[source]",
        "pyi" => "[stub]",
        "md" => "[docs]",
        "toml" => "[config]",
        "json" => "[config]",
        "yaml" | "yml" => "[config]",
        _ => "[other]",
    }
}

pub(crate) fn churn_label(count: u32, high_threshold: u32, med_threshold: u32) -> &'static str {
    if count >= high_threshold {
        "[churn:high]"
    } else if count >= med_threshold {
        "[churn:med]"
    } else {
        "[stable]"
    }
}
