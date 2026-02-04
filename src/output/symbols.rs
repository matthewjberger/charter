use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::extract::symbols::{
    EnumVariant, ImplMethod, InherentImpl, Symbol, SymbolKind, VariantPayload, Visibility,
};
use crate::pipeline::{FileResult, PipelineResult};

const CHAR_BUDGET: usize = 50_000;
const MIN_COMPRESSION_DEPTH: usize = 2;

struct SymbolWriteContext<'a> {
    churn_data: &'a HashMap<PathBuf, u32>,
    high_threshold: u32,
    med_threshold: u32,
    all_inherent_impls: HashMap<String, Vec<(String, InherentImpl)>>,
    type_locations: HashMap<String, String>,
}

impl<'a> SymbolWriteContext<'a> {
    fn new(result: &PipelineResult, churn_data: &'a HashMap<PathBuf, u32>) -> Self {
        let (high_threshold, med_threshold) = calculate_churn_thresholds(churn_data);
        let all_inherent_impls = collect_all_inherent_impls(result);
        let type_locations = build_type_locations(result);

        Self {
            churn_data,
            high_threshold,
            med_threshold,
            all_inherent_impls,
            type_locations,
        }
    }

    fn get_type_location(&self, type_name: &str) -> Option<&str> {
        self.type_locations.get(type_name).map(|s| s.as_str())
    }
}

fn build_type_locations(result: &PipelineResult) -> HashMap<String, String> {
    let mut type_to_file = HashMap::new();

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            match &symbol.kind {
                SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => {
                    type_to_file
                        .entry(symbol.name.clone())
                        .or_insert_with(|| file_result.relative_path.clone());
                }
                _ => {}
            }
        }
    }

    type_to_file
}

pub async fn write_symbols(
    charter_dir: &Path,
    result: &PipelineResult,
    churn_data: &HashMap<PathBuf, u32>,
    stamp: &str,
) -> Result<()> {
    let path = charter_dir.join("symbols.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(256 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    let context = SymbolWriteContext::new(result, churn_data);

    write_public_api_summary(&mut buffer, result)?;

    write_macros_summary(&mut buffer, result)?;

    let full_output = generate_full_symbols(result, &context)?;

    if full_output.len() <= CHAR_BUDGET {
        buffer.extend_from_slice(&full_output);
    } else {
        let budget = CHAR_BUDGET.saturating_sub(buffer.len());
        write_compressed_symbols(&mut buffer, result, &context, budget)?;
    }

    file.write_all(&buffer).await?;
    Ok(())
}

fn collect_all_inherent_impls(
    result: &PipelineResult,
) -> HashMap<String, Vec<(String, InherentImpl)>> {
    let mut map: HashMap<String, Vec<(String, InherentImpl)>> = HashMap::new();

    for file_result in &result.files {
        for inherent_impl in &file_result.parsed.symbols.inherent_impls {
            map.entry(inherent_impl.type_name.clone())
                .or_default()
                .push((file_result.relative_path.clone(), inherent_impl.clone()));
        }
    }

    map
}

fn write_public_api_summary(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut public_symbols: Vec<String> = Vec::new();

    for file_result in &result.files {
        if !file_result.relative_path.ends_with("lib.rs") {
            continue;
        }

        for re_export in &file_result.parsed.re_exports {
            if re_export.visibility == Visibility::Public {
                let path = &re_export.source_path;
                if let Some(name) = extract_exported_name(path) {
                    public_symbols.push(name);
                }
            }
        }
    }

    if public_symbols.is_empty() {
        return Ok(());
    }

    public_symbols.sort();
    public_symbols.dedup();

    writeln!(buffer, "Public API (re-exported from lib.rs):")?;

    let mut line = String::from("  ");
    for (index, symbol) in public_symbols.iter().enumerate() {
        if index > 0 {
            line.push_str(", ");
        }
        if line.len() + symbol.len() > 100 {
            writeln!(buffer, "{}", line)?;
            line = format!("  {}", symbol);
        } else {
            line.push_str(symbol);
        }
    }
    if line.len() > 2 {
        writeln!(buffer, "{}", line)?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn extract_exported_name(path: &str) -> Option<String> {
    let path = path.trim();

    if path.contains('{') {
        let start = path.find('{')? + 1;
        let end = path.find('}')?;
        let inner = &path[start..end];
        return Some(
            inner
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && !s.starts_with("self"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    let last_segment = path.rsplit("::").next()?;

    if last_segment == "*" || last_segment == "self" {
        let segments: Vec<&str> = path.split("::").collect();
        if segments.len() >= 2 {
            return Some(segments[segments.len() - 2].to_string());
        }
        return None;
    }

    Some(last_segment.to_string())
}

fn write_macros_summary(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut exported_macros: Vec<(String, String, usize)> = Vec::new();
    let mut local_macros: Vec<(String, String, usize)> = Vec::new();

    for file_result in &result.files {
        for macro_info in &file_result.parsed.symbols.macros {
            let entry = (
                macro_info.name.clone(),
                file_result.relative_path.clone(),
                macro_info.line,
            );
            if macro_info.is_exported {
                exported_macros.push(entry);
            } else {
                local_macros.push(entry);
            }
        }
    }

    if exported_macros.is_empty() && local_macros.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Macros:")?;

    for (name, file, line) in &exported_macros {
        writeln!(
            buffer,
            "  #[macro_export] macro_rules! {} [{}:{}]",
            name, file, line
        )?;
    }

    for (name, file, line) in &local_macros {
        writeln!(
            buffer,
            "  macro_rules! {} [{}:{}] (crate-local)",
            name, file, line
        )?;
    }

    writeln!(buffer)?;

    Ok(())
}

fn generate_full_symbols(result: &PipelineResult, context: &SymbolWriteContext) -> Result<Vec<u8>> {
    let mut buffer = Vec::with_capacity(256 * 1024);

    for file_result in &result.files {
        if !file_result.relative_path.ends_with(".rs") {
            continue;
        }

        write_file_symbols(&mut buffer, file_result, context)?;
    }

    Ok(buffer)
}

struct CompressedDir {
    files: usize,
    lines: usize,
    type_names: Vec<String>,
}

fn write_compressed_symbols(
    buffer: &mut Vec<u8>,
    result: &PipelineResult,
    context: &SymbolWriteContext,
    budget: usize,
) -> Result<()> {
    let mut current_size = 0;
    let mut compressed_dirs: HashMap<String, CompressedDir> = HashMap::new();

    for file_result in &result.files {
        if !file_result.relative_path.ends_with(".rs") {
            continue;
        }

        if !has_symbols(file_result, &context.all_inherent_impls) {
            continue;
        }

        let depth = file_result.relative_path.matches('/').count();

        if depth >= MIN_COMPRESSION_DEPTH && current_size > budget / 2 {
            let dir_path = get_parent_dir(&file_result.relative_path);
            let entry = compressed_dirs
                .entry(dir_path)
                .or_insert_with(|| CompressedDir {
                    files: 0,
                    lines: 0,
                    type_names: Vec::new(),
                });
            entry.files += 1;
            entry.lines += file_result.lines;
            collect_type_names(file_result, &mut entry.type_names);
            continue;
        }

        let mut file_buffer = Vec::new();
        write_file_symbols(&mut file_buffer, file_result, context)?;

        if current_size + file_buffer.len() <= budget || depth < MIN_COMPRESSION_DEPTH {
            buffer.extend_from_slice(&file_buffer);
            current_size += file_buffer.len();
        } else {
            let dir_path = get_parent_dir(&file_result.relative_path);
            let entry = compressed_dirs
                .entry(dir_path)
                .or_insert_with(|| CompressedDir {
                    files: 0,
                    lines: 0,
                    type_names: Vec::new(),
                });
            entry.files += 1;
            entry.lines += file_result.lines;
            collect_type_names(file_result, &mut entry.type_names);
        }
    }

    if !compressed_dirs.is_empty() {
        writeln!(buffer)?;
        writeln!(
            buffer,
            "# Compressed modules (depth >= {})",
            MIN_COMPRESSION_DEPTH
        )?;
        writeln!(buffer)?;

        let mut dirs: Vec<_> = compressed_dirs.into_iter().collect();
        dirs.sort_by(|a, b| a.0.cmp(&b.0));

        for (dir, info) in dirs {
            write!(
                buffer,
                "{}/ [{} files, {} lines]",
                dir, info.files, info.lines
            )?;
            if !info.type_names.is_empty() {
                let types_str = info.type_names.join(", ");
                if types_str.len() <= 80 {
                    writeln!(buffer, " — {}", types_str)?;
                } else {
                    writeln!(buffer)?;
                    writeln!(buffer, "  {}", types_str)?;
                }
            } else {
                writeln!(buffer)?;
            }
        }
    }

    Ok(())
}

fn collect_type_names(file_result: &FileResult, type_names: &mut Vec<String>) {
    for symbol in &file_result.parsed.symbols.symbols {
        let is_type = matches!(
            &symbol.kind,
            SymbolKind::Struct { .. } | SymbolKind::Enum { .. } | SymbolKind::Trait { .. }
        );
        let is_public =
            symbol.visibility == Visibility::Public || symbol.visibility == Visibility::PubCrate;
        if is_type && is_public && !type_names.contains(&symbol.name) {
            type_names.push(symbol.name.clone());
        }
    }
}

fn has_symbols(
    file_result: &FileResult,
    all_inherent_impls: &HashMap<String, Vec<(String, InherentImpl)>>,
) -> bool {
    if !file_result.parsed.symbols.symbols.is_empty() {
        return true;
    }
    if !file_result.parsed.symbols.macros.is_empty() {
        return true;
    }
    for impls in all_inherent_impls.values() {
        if impls
            .iter()
            .any(|(path, _)| path == &file_result.relative_path)
        {
            return true;
        }
    }
    false
}

fn write_file_symbols(
    buffer: &mut Vec<u8>,
    file_result: &FileResult,
    context: &SymbolWriteContext,
) -> Result<()> {
    if !has_symbols(file_result, &context.all_inherent_impls) {
        return Ok(());
    }

    let types_defined_here: std::collections::HashSet<&str> = file_result
        .parsed
        .symbols
        .symbols
        .iter()
        .filter_map(|s| match &s.kind {
            SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    let external_impls: Vec<&InherentImpl> = file_result
        .parsed
        .symbols
        .inherent_impls
        .iter()
        .filter(|imp| !types_defined_here.contains(imp.type_name.as_str()))
        .collect();

    let important_symbols: Vec<&Symbol> = file_result
        .parsed
        .symbols
        .symbols
        .iter()
        .filter(|s| is_important_symbol(s))
        .collect();

    let has_important_symbols = !important_symbols.is_empty();
    let has_macros = !file_result.parsed.symbols.macros.is_empty();
    let has_external_impls = !external_impls.is_empty();

    if !has_important_symbols && !has_macros && !has_external_impls {
        return Ok(());
    }

    let churn_count = context
        .churn_data
        .get(&file_result.path)
        .copied()
        .unwrap_or(0);
    let churn_label =
        super::churn_label(churn_count, context.high_threshold, context.med_threshold);
    let role = super::file_role(&file_result.path);

    writeln!(
        buffer,
        "{} [{} lines] {} {}",
        file_result.relative_path, file_result.lines, role, churn_label
    )?;

    for symbol in important_symbols {
        write_symbol(
            buffer,
            symbol,
            &file_result.relative_path,
            &context.all_inherent_impls,
        )?;
    }

    for inherent_impl in external_impls {
        write_external_impl_summary(buffer, inherent_impl, context)?;
    }

    let imports: Vec<_> = file_result
        .parsed
        .imports
        .iter()
        .filter(|i| i.path.starts_with("crate::") || i.path.starts_with("super::"))
        .map(|i| &i.path)
        .collect();

    if !imports.is_empty() {
        let imports_str: Vec<_> = imports.iter().map(|s| s.as_str()).collect();
        writeln!(buffer, "  uses: {}", imports_str.join(", "))?;
    }

    writeln!(buffer)?;
    Ok(())
}

fn is_important_symbol(symbol: &Symbol) -> bool {
    match &symbol.kind {
        SymbolKind::Struct { .. }
        | SymbolKind::Enum { .. }
        | SymbolKind::Trait { .. }
        | SymbolKind::TypeAlias { .. } => true,
        SymbolKind::Function { .. } => {
            symbol.visibility == Visibility::Public || symbol.visibility == Visibility::PubCrate
        }
        SymbolKind::Const { .. } | SymbolKind::Static { .. } => {
            symbol.visibility == Visibility::Public
        }
        SymbolKind::Mod => symbol.visibility == Visibility::Public,
    }
}

fn write_external_impl_summary(
    buffer: &mut Vec<u8>,
    inherent_impl: &InherentImpl,
    context: &SymbolWriteContext,
) -> Result<()> {
    let method_count = inherent_impl.methods.len();
    let method_word = if method_count == 1 {
        "method"
    } else {
        "methods"
    };

    let location_info = if let Some(loc) = context.get_type_location(&inherent_impl.type_name) {
        format!(" (defined at {})", loc)
    } else {
        String::new()
    };

    let qualifier = if !inherent_impl.generics.is_empty() || inherent_impl.where_clause.is_some() {
        let mut parts = Vec::new();
        if !inherent_impl.generics.is_empty() {
            parts.push(inherent_impl.generics.clone());
        }
        if let Some(wc) = &inherent_impl.where_clause {
            parts.push(format!("where {}", wc));
        }
        format!(" [{}]", parts.join(" "))
    } else {
        String::new()
    };

    writeln!(
        buffer,
        "  impl {}: {} {}{}{}",
        inherent_impl.type_name, method_count, method_word, qualifier, location_info
    )?;

    Ok(())
}

fn write_symbol(
    buffer: &mut Vec<u8>,
    symbol: &Symbol,
    current_file: &str,
    all_inherent_impls: &HashMap<String, Vec<(String, InherentImpl)>>,
) -> Result<()> {
    let vis = symbol.visibility.prefix();
    let qualifiers = super::format_qualifiers(symbol.is_async, symbol.is_unsafe, symbol.is_const);

    match &symbol.kind {
        SymbolKind::Struct { fields } => {
            write!(buffer, "  {}struct {}", vis, symbol.name)?;
            if !symbol.generics.is_empty() {
                write!(buffer, "{}", symbol.generics)?;
            }

            if fields.is_empty() {
                writeln!(buffer)?;
            } else if fields.len() <= 5 {
                write!(buffer, " {{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(buffer, ", ")?;
                    }
                    write!(buffer, "{}: {}", field.name, field.field_type)?;
                }
                writeln!(buffer, " }}")?;
            } else {
                writeln!(buffer, " {{")?;
                for field in fields {
                    let field_vis = if field.visibility != Visibility::Private {
                        format!("{} ", field.visibility)
                    } else {
                        String::new()
                    };
                    writeln!(
                        buffer,
                        "    {}{}: {},",
                        field_vis, field.name, field.field_type
                    )?;
                }
                writeln!(buffer, "  }}")?;
            }

            write_inherent_impls_for_type(buffer, &symbol.name, current_file, all_inherent_impls)?;
        }
        SymbolKind::Enum { variants } => {
            write!(buffer, "  {}enum {}", vis, symbol.name)?;
            if !symbol.generics.is_empty() {
                write!(buffer, "{}", symbol.generics)?;
            }

            if variants.is_empty() {
                writeln!(buffer)?;
            } else if is_simple_enum(variants) && variants.len() <= 6 {
                write!(buffer, " {{ ")?;
                for (index, variant) in variants.iter().enumerate() {
                    if index > 0 {
                        write!(buffer, ", ")?;
                    }
                    write_variant_inline(buffer, variant)?;
                }
                writeln!(buffer, " }}")?;
            } else {
                writeln!(buffer, " {{")?;
                for variant in variants {
                    write!(buffer, "    ")?;
                    write_variant_inline(buffer, variant)?;
                    writeln!(buffer, ",")?;
                }
                writeln!(buffer, "  }}")?;
            }

            write_inherent_impls_for_type(buffer, &symbol.name, current_file, all_inherent_impls)?;
        }
        SymbolKind::Trait {
            supertraits,
            methods,
            associated_types,
        } => {
            write!(buffer, "  {}trait {}", vis, symbol.name)?;
            if !symbol.generics.is_empty() {
                write!(buffer, "{}", symbol.generics)?;
            }
            if !supertraits.is_empty() {
                write!(buffer, ": {}", supertraits.join(" + "))?;
            }
            writeln!(buffer)?;

            for assoc in associated_types {
                write!(buffer, "    type {}", assoc.name)?;
                if let Some(bounds) = &assoc.bounds {
                    write!(buffer, ": {}", bounds)?;
                }
                writeln!(buffer)?;
            }

            for method in methods {
                let default_marker = if method.has_default {
                    "default "
                } else {
                    "required "
                };
                writeln!(
                    buffer,
                    "    {}fn {}{}",
                    default_marker, method.name, method.signature
                )?;
            }
        }
        SymbolKind::Function { signature, .. } => {
            writeln!(
                buffer,
                "  {}{}fn {}{}",
                vis, qualifiers, symbol.name, signature
            )?;
        }
        SymbolKind::Const { const_type, value } => {
            write!(buffer, "  {}const {}: {}", vis, symbol.name, const_type)?;
            if let Some(val) = value {
                writeln!(buffer, " = {}", val)?;
            } else {
                writeln!(buffer)?;
            }
        }
        SymbolKind::Static {
            static_type,
            is_mutable,
            value,
        } => {
            let mut_kw = if *is_mutable { "mut " } else { "" };
            write!(
                buffer,
                "  {}static {}{}: {}",
                vis, mut_kw, symbol.name, static_type
            )?;
            if let Some(val) = value {
                writeln!(buffer, " = {}", val)?;
            } else {
                writeln!(buffer)?;
            }
        }
        SymbolKind::TypeAlias { aliased_type } => {
            write!(buffer, "  {}type {}", vis, symbol.name)?;
            if !symbol.generics.is_empty() {
                write!(buffer, "{}", symbol.generics)?;
            }
            writeln!(buffer, " = {}", aliased_type)?;
        }
        SymbolKind::Mod => {
            writeln!(buffer, "  {}mod {}", vis, symbol.name)?;
        }
    }

    if let Some(re_export) = &symbol.re_exported_as {
        writeln!(buffer, "    [re-exported as {}]", re_export)?;
    }

    Ok(())
}

fn write_inherent_impls_for_type(
    buffer: &mut Vec<u8>,
    type_name: &str,
    current_file: &str,
    all_inherent_impls: &HashMap<String, Vec<(String, InherentImpl)>>,
) -> Result<()> {
    if let Some(impls) = all_inherent_impls.get(type_name) {
        for (file, inherent_impl) in impls {
            write!(buffer, "    impl")?;
            if !inherent_impl.generics.is_empty() {
                write!(buffer, "{}", inherent_impl.generics)?;
            }
            write!(buffer, " {}", type_name)?;
            if let Some(where_clause) = &inherent_impl.where_clause {
                write!(buffer, " where {}", where_clause)?;
            }
            if file != current_file {
                writeln!(buffer, ": [from {}]", file)?;
            } else {
                writeln!(buffer, ":")?;
            }

            for method in &inherent_impl.methods {
                write_impl_method(buffer, method)?;
            }
        }
    }

    Ok(())
}

fn write_impl_method(buffer: &mut Vec<u8>, method: &ImplMethod) -> Result<()> {
    let vis = method.visibility.prefix();
    let qualifiers = super::format_qualifiers(method.is_async, method.is_unsafe, method.is_const);

    writeln!(
        buffer,
        "      {}{}fn {}{}",
        vis, qualifiers, method.name, method.signature
    )?;
    Ok(())
}

fn write_variant_inline(buffer: &mut Vec<u8>, variant: &EnumVariant) -> Result<()> {
    write!(buffer, "{}", variant.name)?;

    match &variant.payload {
        None => {}
        Some(VariantPayload::Tuple(types)) => {
            write!(buffer, "({})", types.join(", "))?;
        }
        Some(VariantPayload::Struct(fields)) => {
            write!(buffer, " {{ ")?;
            for (index, (name, typ)) in fields.iter().enumerate() {
                if index > 0 {
                    write!(buffer, ", ")?;
                }
                write!(buffer, "{}: {}", name, typ)?;
            }
            write!(buffer, " }}")?;
        }
    }

    Ok(())
}

fn is_simple_enum(variants: &[EnumVariant]) -> bool {
    variants.iter().all(|v| match &v.payload {
        None => true,
        Some(VariantPayload::Tuple(types)) => types.len() <= 2,
        Some(VariantPayload::Struct(fields)) => fields.len() <= 2,
    })
}

fn get_parent_dir(path: &str) -> String {
    let parts: Vec<_> = path.rsplitn(2, '/').collect();
    if parts.len() > 1 {
        parts[1].to_string()
    } else {
        String::new()
    }
}

fn calculate_churn_thresholds(churn_data: &HashMap<PathBuf, u32>) -> (u32, u32) {
    if churn_data.is_empty() {
        return (10, 5);
    }

    let mut counts: Vec<u32> = churn_data.values().copied().collect();
    counts.sort_unstable();

    let len = counts.len();
    let high = counts.get(len * 2 / 3).copied().unwrap_or(10);
    let med = counts.get(len / 3).copied().unwrap_or(5);

    (high.max(1), med.max(1))
}
