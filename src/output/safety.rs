use anyhow::Result;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::pipeline::PipelineResult;

pub async fn write_safety(charter_dir: &Path, result: &PipelineResult, stamp: &str) -> Result<()> {
    let path = charter_dir.join("safety.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(64 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    writeln!(buffer, "# Safety Analysis")?;
    writeln!(buffer)?;

    write_panic_points(&mut buffer, result)?;
    write_unsafe_blocks(&mut buffer, result)?;
    write_unsafe_traits(&mut buffer, result)?;
    write_lifetime_summary(&mut buffer, result)?;
    write_async_summary(&mut buffer, result)?;
    write_feature_flags(&mut buffer, result)?;
    write_generic_constraints(&mut buffer, result)?;
    write_test_coverage(&mut buffer, result)?;
    write_doc_coverage(&mut buffer, result)?;

    file.write_all(&buffer).await?;
    Ok(())
}

use std::io::Write;

fn write_panic_points(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut all_panics: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .safety
                .panic_points
                .iter()
                .map(move |p| (f.relative_path.as_str(), p))
        })
        .collect();

    if all_panics.is_empty() {
        return Ok(());
    }

    all_panics.sort_by_key(|(path, p)| (*path, p.line));

    writeln!(buffer, "## Panic Points")?;
    writeln!(buffer)?;
    writeln!(buffer, "Locations that may panic at runtime.")?;
    writeln!(buffer)?;

    let unwrap_count = all_panics
        .iter()
        .filter(|(_, p)| matches!(p.kind, crate::extract::safety::PanicKind::Unwrap))
        .count();
    let expect_count = all_panics
        .iter()
        .filter(|(_, p)| matches!(p.kind, crate::extract::safety::PanicKind::Expect(_)))
        .count();
    let index_count = all_panics
        .iter()
        .filter(|(_, p)| matches!(p.kind, crate::extract::safety::PanicKind::IndexAccess))
        .count();
    let macro_count = all_panics
        .iter()
        .filter(|(_, p)| {
            matches!(
                p.kind,
                crate::extract::safety::PanicKind::PanicMacro
                    | crate::extract::safety::PanicKind::UnreachableMacro
                    | crate::extract::safety::PanicKind::TodoMacro
                    | crate::extract::safety::PanicKind::UnimplementedMacro
            )
        })
        .count();
    let assert_count = all_panics
        .iter()
        .filter(|(_, p)| matches!(p.kind, crate::extract::safety::PanicKind::Assert))
        .count();

    writeln!(buffer, "Summary: {} total panic points", all_panics.len())?;
    writeln!(buffer, "  .unwrap(): {}", unwrap_count)?;
    writeln!(buffer, "  .expect(): {}", expect_count)?;
    writeln!(buffer, "  index access: {}", index_count)?;
    writeln!(buffer, "  panic!/unreachable!/todo!: {}", macro_count)?;
    writeln!(buffer, "  assert!: {}", assert_count)?;
    writeln!(buffer)?;

    let mut current_file = "";
    for (path, panic) in all_panics.iter().take(100) {
        if *path != current_file {
            writeln!(buffer, "{}:", path)?;
            current_file = path;
        }
        let fn_context = panic
            .containing_function
            .as_deref()
            .unwrap_or("(top-level)");
        writeln!(
            buffer,
            "  L{} in {} — {}",
            panic.line, fn_context, panic.kind
        )?;
    }

    if all_panics.len() > 100 {
        writeln!(buffer, "[+{} more panic points]", all_panics.len() - 100)?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_unsafe_blocks(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut all_unsafe: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .safety
                .unsafe_blocks
                .iter()
                .map(move |u| (f.relative_path.as_str(), u))
        })
        .collect();

    if all_unsafe.is_empty() {
        writeln!(buffer, "## Unsafe Code")?;
        writeln!(buffer)?;
        writeln!(buffer, "No unsafe blocks found.")?;
        writeln!(buffer)?;
        return Ok(());
    }

    writeln!(buffer, "## Unsafe Blocks")?;
    writeln!(buffer)?;
    writeln!(buffer, "{} unsafe blocks found.", all_unsafe.len())?;
    writeln!(buffer)?;

    all_unsafe.sort_by_key(|(path, u)| (*path, u.line));

    for (path, unsafe_block) in &all_unsafe {
        let fn_context = unsafe_block
            .containing_function
            .as_deref()
            .unwrap_or("(top-level)");
        write!(buffer, "{}:{} in {}", path, unsafe_block.line, fn_context)?;

        if !unsafe_block.operations.is_empty() {
            write!(buffer, " — ")?;
            let ops: Vec<_> = unsafe_block
                .operations
                .iter()
                .map(|o| o.to_string())
                .collect();
            write!(buffer, "{}", ops.join(", "))?;
        }
        writeln!(buffer)?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_unsafe_traits(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut all_unsafe_traits: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .safety
                .unsafe_traits
                .iter()
                .map(move |t| (f.relative_path.as_str(), t.as_str()))
        })
        .collect();

    let mut all_unsafe_impls: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .safety
                .unsafe_impls
                .iter()
                .map(move |i| (f.relative_path.as_str(), i))
        })
        .collect();

    if all_unsafe_traits.is_empty() && all_unsafe_impls.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "## Unsafe Traits & Impls")?;
    writeln!(buffer)?;

    if !all_unsafe_traits.is_empty() {
        writeln!(buffer, "Unsafe traits:")?;
        all_unsafe_traits.sort();
        for (path, trait_name) in &all_unsafe_traits {
            writeln!(buffer, "  {} ({})", trait_name, path)?;
        }
        writeln!(buffer)?;
    }

    if !all_unsafe_impls.is_empty() {
        writeln!(buffer, "Unsafe impl blocks:")?;
        all_unsafe_impls.sort_by_key(|(path, i)| (*path, i.line));
        for (path, impl_info) in &all_unsafe_impls {
            writeln!(
                buffer,
                "  unsafe impl {} for {} ({}:{})",
                impl_info.trait_name, impl_info.type_name, path, impl_info.line
            )?;
        }
        writeln!(buffer)?;
    }

    Ok(())
}

fn write_lifetime_summary(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut structs_with_lifetimes: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .lifetimes
                .struct_lifetimes
                .iter()
                .map(move |s| (f.relative_path.as_str(), s))
        })
        .collect();

    let mut functions_with_lifetimes: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .lifetimes
                .function_lifetimes
                .iter()
                .map(move |fl| (f.relative_path.as_str(), fl))
        })
        .filter(|(_, fl)| !fl.lifetimes.is_empty() || fl.has_static)
        .collect();

    let mut complex_bounds: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .lifetimes
                .complex_bounds
                .iter()
                .map(move |b| (f.relative_path.as_str(), b))
        })
        .collect();

    if structs_with_lifetimes.is_empty()
        && functions_with_lifetimes.is_empty()
        && complex_bounds.is_empty()
    {
        return Ok(());
    }

    writeln!(buffer, "## Lifetime Analysis")?;
    writeln!(buffer)?;

    if !structs_with_lifetimes.is_empty() {
        writeln!(buffer, "Types with lifetimes:")?;
        structs_with_lifetimes.sort_by_key(|(_, s)| &s.type_name);
        for (path, sl) in &structs_with_lifetimes {
            let lifetimes_str = if sl.lifetimes.is_empty() {
                "'static".to_string()
            } else {
                sl.lifetimes.join(", ")
            };
            writeln!(
                buffer,
                "  {}<{}> ({}:{})",
                sl.type_name, lifetimes_str, path, sl.line
            )?;
        }
        writeln!(buffer)?;
    }

    if !functions_with_lifetimes.is_empty() {
        writeln!(
            buffer,
            "Functions with explicit lifetimes ({}):",
            functions_with_lifetimes.len()
        )?;
        functions_with_lifetimes.sort_by_key(|(path, _)| *path);
        for (path, fl) in functions_with_lifetimes.iter().take(30) {
            let qualified = if let Some(ref impl_type) = fl.impl_type {
                format!("{}::{}", impl_type, fl.function_name)
            } else {
                fl.function_name.clone()
            };
            let lifetimes_str = fl.lifetimes.join(", ");
            writeln!(
                buffer,
                "  {}:{} {} — <{}>",
                path, fl.line, qualified, lifetimes_str
            )?;
        }
        if functions_with_lifetimes.len() > 30 {
            writeln!(buffer, "  [+{} more]", functions_with_lifetimes.len() - 30)?;
        }
        writeln!(buffer)?;
    }

    if !complex_bounds.is_empty() {
        writeln!(buffer, "Complex where clauses ({}):", complex_bounds.len())?;
        complex_bounds.sort_by_key(|(path, _)| *path);
        for (path, bound) in complex_bounds.iter().take(20) {
            writeln!(
                buffer,
                "  {}:{} {} where {}",
                path, bound.line, bound.item_name, bound.bounds
            )?;
        }
        if complex_bounds.len() > 20 {
            writeln!(buffer, "  [+{} more]", complex_bounds.len() - 20)?;
        }
        writeln!(buffer)?;
    }

    Ok(())
}

fn write_async_summary(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let async_functions: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .async_info
                .async_functions
                .iter()
                .map(move |af| (f.relative_path.as_str(), af))
        })
        .collect();

    let blocking_in_async: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .async_info
                .blocking_calls
                .iter()
                .map(move |bc| (f.relative_path.as_str(), bc))
        })
        .filter(|(_, bc)| bc.in_async_context)
        .collect();

    if async_functions.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "## Async Analysis")?;
    writeln!(buffer)?;
    writeln!(buffer, "{} async functions found.", async_functions.len())?;
    writeln!(buffer)?;

    let total_awaits: usize = async_functions.iter().map(|(_, af)| af.awaits.len()).sum();
    let total_spawns: usize = async_functions.iter().map(|(_, af)| af.spawns.len()).sum();

    writeln!(buffer, "Total .await points: {}", total_awaits)?;
    writeln!(buffer, "Total spawn points: {}", total_spawns)?;
    writeln!(buffer)?;

    if !blocking_in_async.is_empty() {
        writeln!(
            buffer,
            "⚠ Blocking calls in async context ({}):",
            blocking_in_async.len()
        )?;
        for (path, bc) in blocking_in_async.iter().take(20) {
            let fn_name = bc.containing_function.as_deref().unwrap_or("unknown");
            writeln!(
                buffer,
                "  {}:{} in {} — {}",
                path, bc.line, fn_name, bc.call
            )?;
        }
        if blocking_in_async.len() > 20 {
            writeln!(buffer, "  [+{} more]", blocking_in_async.len() - 20)?;
        }
        writeln!(buffer)?;
    }

    let complex_async: Vec<_> = async_functions
        .iter()
        .filter(|(_, af)| af.awaits.len() > 5 || !af.spawns.is_empty())
        .collect();

    if !complex_async.is_empty() {
        writeln!(buffer, "Complex async functions:")?;
        for (path, af) in complex_async.iter().take(20) {
            let qualified = if let Some(ref impl_type) = af.impl_type {
                format!("{}::{}", impl_type, af.name)
            } else {
                af.name.clone()
            };
            write!(
                buffer,
                "  {}:{} {} — {} awaits",
                path,
                af.line,
                qualified,
                af.awaits.len()
            )?;
            if !af.spawns.is_empty() {
                write!(buffer, ", {} spawns", af.spawns.len())?;
            }
            writeln!(buffer)?;
        }
        writeln!(buffer)?;
    }

    Ok(())
}

fn write_feature_flags(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut all_features: std::collections::HashMap<
        String,
        Vec<(String, crate::extract::safety::GatedSymbol)>,
    > = std::collections::HashMap::new();

    for file in &result.files {
        for gate in &file.parsed.feature_flags.feature_gates {
            for symbol in &gate.symbols {
                all_features
                    .entry(gate.feature_name.clone())
                    .or_default()
                    .push((file.relative_path.clone(), symbol.clone()));
            }
        }
    }

    let cfg_blocks: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .feature_flags
                .cfg_blocks
                .iter()
                .map(move |c| (f.relative_path.as_str(), c))
        })
        .filter(|(_, c)| !c.condition.starts_with("feature"))
        .collect();

    if all_features.is_empty() && cfg_blocks.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "## Feature Flags")?;
    writeln!(buffer)?;

    if !all_features.is_empty() {
        writeln!(buffer, "Feature-gated code:")?;
        let mut features: Vec<_> = all_features.into_iter().collect();
        features.sort_by_key(|(name, _)| name.clone());

        for (feature, symbols) in &features {
            writeln!(
                buffer,
                "  feature = \"{}\" ({} items):",
                feature,
                symbols.len()
            )?;
            for (path, symbol) in symbols.iter().take(5) {
                writeln!(
                    buffer,
                    "    {} {} ({}:{})",
                    symbol.kind, symbol.name, path, symbol.line
                )?;
            }
            if symbols.len() > 5 {
                writeln!(buffer, "    [+{} more]", symbols.len() - 5)?;
            }
        }
        writeln!(buffer)?;
    }

    if !cfg_blocks.is_empty() {
        writeln!(buffer, "Other #[cfg] conditions ({}):", cfg_blocks.len())?;
        for (path, cfg) in cfg_blocks.iter().take(20) {
            writeln!(
                buffer,
                "  {}:{} — #[cfg({})]",
                path, cfg.line, cfg.condition
            )?;
        }
        if cfg_blocks.len() > 20 {
            writeln!(buffer, "  [+{} more]", cfg_blocks.len() - 20)?;
        }
        writeln!(buffer)?;
    }

    Ok(())
}

fn write_generic_constraints(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let constraints: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .generic_constraints
                .constraints
                .iter()
                .map(move |c| (f.relative_path.as_str(), c))
        })
        .filter(|(_, c)| {
            c.type_params.iter().any(|p| !p.bounds.is_empty()) || c.where_clause.is_some()
        })
        .collect();

    if constraints.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "## Generic Constraints")?;
    writeln!(buffer)?;
    writeln!(buffer, "Items with trait bounds ({}):", constraints.len())?;
    writeln!(buffer)?;

    for (path, constraint) in constraints.iter().take(50) {
        write!(
            buffer,
            "{}:{} {} {}",
            path, constraint.line, constraint.item_kind, constraint.item_name
        )?;

        let bounds_str: Vec<_> = constraint
            .type_params
            .iter()
            .filter(|p| !p.bounds.is_empty())
            .map(|p| format!("{}: {}", p.name, p.bounds.join(" + ")))
            .collect();

        if !bounds_str.is_empty() {
            write!(buffer, "<{}>", bounds_str.join(", "))?;
        }

        if let Some(ref where_clause) = constraint.where_clause {
            write!(buffer, " where {}", where_clause)?;
        }

        writeln!(buffer)?;
    }

    if constraints.len() > 50 {
        writeln!(buffer, "[+{} more]", constraints.len() - 50)?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_test_coverage(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let test_functions: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .test_info
                .test_functions
                .iter()
                .map(move |t| (f.relative_path.as_str(), t))
        })
        .collect();

    let test_modules: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .test_info
                .test_modules
                .iter()
                .map(move |m| (f.relative_path.as_str(), m))
        })
        .collect();

    let tested_items: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .test_info
                .tested_items
                .iter()
                .map(move |t| (f.relative_path.as_str(), t))
        })
        .collect();

    if test_functions.is_empty() && test_modules.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "## Test Coverage")?;
    writeln!(buffer)?;

    let total_tests = test_functions.len();
    let async_tests = test_functions.iter().filter(|(_, t)| t.is_async).count();
    let ignored_tests = test_functions.iter().filter(|(_, t)| t.is_ignored).count();
    let should_panic_tests = test_functions
        .iter()
        .filter(|(_, t)| t.should_panic)
        .count();

    writeln!(buffer, "Summary:")?;
    writeln!(buffer, "  Total test functions: {}", total_tests)?;
    writeln!(buffer, "  Async tests: {}", async_tests)?;
    writeln!(buffer, "  Ignored tests: {}", ignored_tests)?;
    writeln!(buffer, "  #[should_panic] tests: {}", should_panic_tests)?;
    writeln!(buffer, "  Test modules: {}", test_modules.len())?;
    writeln!(buffer)?;

    if !tested_items.is_empty() {
        writeln!(buffer, "Inferred test coverage:")?;
        for (_, item) in tested_items.iter().take(30) {
            write!(
                buffer,
                "  {} — tested by: {}",
                item.item_name,
                item.test_names.join(", ")
            )?;
            if !item.coverage_hints.is_empty() {
                write!(buffer, " ({})", item.coverage_hints.join(", "))?;
            }
            writeln!(buffer)?;
        }
        if tested_items.len() > 30 {
            writeln!(buffer, "  [+{} more]", tested_items.len() - 30)?;
        }
        writeln!(buffer)?;
    }

    Ok(())
}

fn write_doc_coverage(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let docs: Vec<_> = result
        .files
        .iter()
        .flat_map(|f| {
            f.parsed
                .doc_info
                .item_docs
                .iter()
                .map(move |d| (f.relative_path.as_str(), d))
        })
        .collect();

    let total_public_items: usize = result
        .files
        .iter()
        .map(|f| {
            f.parsed
                .symbols
                .symbols
                .iter()
                .filter(|s| matches!(s.visibility, crate::extract::symbols::Visibility::Public))
                .count()
        })
        .sum();

    if docs.is_empty() && total_public_items == 0 {
        return Ok(());
    }

    writeln!(buffer, "## Documentation Coverage")?;
    writeln!(buffer)?;

    let documented_count = docs.len();
    let with_examples = docs.iter().filter(|(_, d)| d.has_examples).count();
    let with_panics = docs.iter().filter(|(_, d)| d.has_panics_section).count();
    let with_safety = docs.iter().filter(|(_, d)| d.has_safety_section).count();
    let with_errors = docs.iter().filter(|(_, d)| d.has_errors_section).count();

    writeln!(buffer, "Summary:")?;
    writeln!(buffer, "  Public items: {}", total_public_items)?;
    writeln!(buffer, "  Documented items: {}", documented_count)?;
    writeln!(buffer, "  With examples: {}", with_examples)?;
    writeln!(buffer, "  With # Panics section: {}", with_panics)?;
    writeln!(buffer, "  With # Safety section: {}", with_safety)?;
    writeln!(buffer, "  With # Errors section: {}", with_errors)?;
    writeln!(buffer)?;

    Ok(())
}
