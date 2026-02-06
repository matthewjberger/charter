use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::extract::symbols::SymbolKind;
use crate::pipeline::PipelineResult;

pub async fn write_types(charter_dir: &Path, result: &PipelineResult, stamp: &str) -> Result<()> {
    let path = charter_dir.join("types.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(64 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    write_trait_definitions(&mut buffer, result)?;
    write_impl_map(&mut buffer, result)?;
    write_derive_map(&mut buffer, result)?;

    file.write_all(&buffer).await?;
    Ok(())
}

fn write_trait_definitions(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut has_traits = false;

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Trait {
                supertraits,
                methods,
                associated_types,
            } = &symbol.kind
            {
                if !has_traits {
                    writeln!(buffer, "Traits:")?;
                    has_traits = true;
                }

                write!(buffer, "  trait {}", symbol.name)?;
                if !symbol.generics.is_empty() {
                    write!(buffer, "{}", symbol.generics)?;
                }
                if !supertraits.is_empty() {
                    write!(buffer, ": {}", supertraits.join(" + "))?;
                }
                writeln!(buffer)?;

                if methods.is_empty() && associated_types.is_empty() {
                    writeln!(buffer, "    (marker trait)")?;
                }

                for assoc in associated_types {
                    write!(buffer, "    type {}", assoc.name)?;
                    if let Some(bounds) = &assoc.bounds {
                        write!(buffer, ": {}", bounds)?;
                    }
                    writeln!(buffer)?;
                }

                for method in methods {
                    let kind = if method.has_default {
                        "default"
                    } else {
                        "required"
                    };
                    writeln!(
                        buffer,
                        "    {} fn {}{}",
                        kind, method.name, method.signature
                    )?;
                }

                writeln!(buffer)?;
            }
        }
    }

    if has_traits {
        writeln!(buffer)?;
    }

    write_python_protocols(buffer, result)?;
    write_python_abcs(buffer, result)?;
    write_python_type_vars(buffer, result)?;
    write_python_dataclasses(buffer, result)?;
    write_class_hierarchy(buffer, result)?;

    Ok(())
}

fn write_python_protocols(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut protocols: Vec<(&str, &str, &[crate::extract::symbols::ClassMethod])> = Vec::new();

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Class {
                methods,
                is_protocol: true,
                ..
            } = &symbol.kind
            {
                protocols.push((&file_result.relative_path, &symbol.name, methods));
            }
        }
    }

    if protocols.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Protocols (Python):")?;
    for (path, name, methods) in protocols {
        writeln!(buffer, "  {} ({})", name, path)?;
        for method in methods {
            if method.is_abstract {
                writeln!(
                    buffer,
                    "    required def {}{}",
                    method.name, method.signature
                )?;
            } else {
                writeln!(
                    buffer,
                    "    default def {}{}",
                    method.name, method.signature
                )?;
            }
        }
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_python_abcs(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut abcs: Vec<(&str, &str, &[crate::extract::symbols::ClassMethod])> = Vec::new();

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Class {
                methods,
                is_abc: true,
                is_protocol: false,
                ..
            } = &symbol.kind
            {
                abcs.push((&file_result.relative_path, &symbol.name, methods));
            }
        }
    }

    if abcs.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Abstract Base Classes (Python):")?;
    for (path, name, methods) in abcs {
        writeln!(buffer, "  {} ({})", name, path)?;
        for method in methods {
            if method.is_abstract {
                writeln!(
                    buffer,
                    "    @abstractmethod def {}{}",
                    method.name, method.signature
                )?;
            }
        }
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_python_type_vars(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut type_vars: Vec<(String, String, String)> = Vec::new();

    for file_result in &result.files {
        if !file_result.relative_path.ends_with(".py")
            && !file_result.relative_path.ends_with(".pyi")
        {
            continue;
        }

        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Variable {
                value: Some(val), ..
            } = &symbol.kind
            {
                if val.contains("TypeVar(") {
                    type_vars.push((
                        file_result.relative_path.clone(),
                        symbol.name.clone(),
                        val.clone(),
                    ));
                }
            }
        }
    }

    if type_vars.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Type Variables (Python):")?;
    for (path, name, value) in type_vars.iter().take(50) {
        let short_value = if value.len() > 60 {
            format!("{}...", &value[..57])
        } else {
            value.clone()
        };
        writeln!(buffer, "  {} = {} ({})", name, short_value, path)?;
    }
    if type_vars.len() > 50 {
        writeln!(buffer, "  [+{} more]", type_vars.len() - 50)?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_python_dataclasses(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut dataclasses: Vec<(String, String, Vec<String>)> = Vec::new();

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Class {
                fields,
                is_dataclass: true,
                ..
            } = &symbol.kind
            {
                let field_names: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        if let Some(hint) = &f.type_hint {
                            format!("{}: {}", f.name, hint)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                dataclasses.push((
                    file_result.relative_path.clone(),
                    symbol.name.clone(),
                    field_names,
                ));
            }
        }
    }

    if dataclasses.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Dataclasses (Python):")?;
    for (path, name, fields) in dataclasses.iter().take(30) {
        writeln!(buffer, "  @dataclass {} ({})", name, path)?;
        for field in fields.iter().take(10) {
            writeln!(buffer, "    {}", field)?;
        }
        if fields.len() > 10 {
            writeln!(buffer, "    [+{} more fields]", fields.len() - 10)?;
        }
    }
    if dataclasses.len() > 30 {
        writeln!(buffer, "  [+{} more dataclasses]", dataclasses.len() - 30)?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_class_hierarchy(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();

    for file_result in &result.files {
        for symbol in &file_result.parsed.symbols.symbols {
            if let SymbolKind::Class { bases, .. } = &symbol.kind {
                if !bases.is_empty() {
                    class_bases.insert(symbol.name.clone(), bases.clone());
                }
            }
        }
    }

    if class_bases.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Class Hierarchy (Python):")?;
    let mut sorted: Vec<_> = class_bases.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    for (class_name, bases) in sorted {
        writeln!(buffer, "  {} extends {}", class_name, bases.join(", "))?;
    }
    writeln!(buffer)?;

    Ok(())
}

fn write_impl_map(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut impl_map: HashMap<String, Vec<String>> = HashMap::new();

    for file_result in &result.files {
        for (trait_name, type_name) in &file_result.parsed.symbols.impl_map {
            impl_map
                .entry(trait_name.clone())
                .or_default()
                .push(type_name.clone());
        }
    }

    if impl_map.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Impls:")?;

    let mut sorted: Vec<_> = impl_map.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    for (trait_name, mut types) in sorted {
        types.sort();
        types.dedup();
        writeln!(buffer, "  {} -> [{}]", trait_name, types.join(", "))?;
    }

    writeln!(buffer)?;
    Ok(())
}

fn write_derive_map(buffer: &mut Vec<u8>, result: &PipelineResult) -> Result<()> {
    let mut derive_map: HashMap<String, Vec<String>> = HashMap::new();

    for file_result in &result.files {
        for derive in &file_result.parsed.derives {
            derive_map
                .entry(derive.target.clone())
                .or_default()
                .extend(derive.traits.clone());
        }
    }

    if derive_map.is_empty() {
        return Ok(());
    }

    writeln!(buffer, "Derived:")?;

    let mut sorted: Vec<_> = derive_map.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    for (type_name, mut traits) in sorted {
        traits.sort();
        traits.dedup();
        writeln!(buffer, "  {} - {}", type_name, traits.join(", "))?;
    }

    writeln!(buffer)?;
    Ok(())
}
