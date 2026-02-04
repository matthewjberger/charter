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
