use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

const MAX_FILES_PER_SYMBOL: usize = 6;
const MAX_SYMBOLS: usize = 300;
const MIN_FILES_THRESHOLD: usize = 2;

pub async fn write_refs(
    charter_dir: &Path,
    references: &HashMap<String, Vec<(String, usize)>>,
    stamp: &str,
) -> Result<()> {
    let path = charter_dir.join("refs.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(64 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    if references.is_empty() {
        writeln!(buffer, "(no cross-references found)")?;
        file.write_all(&buffer).await?;
        return Ok(());
    }

    let mut processed: Vec<ProcessedSymbol> = references
        .iter()
        .filter_map(|(name, locations)| process_symbol(name, locations))
        .collect();

    processed.sort_by(|a, b| {
        b.total_refs
            .cmp(&a.total_refs)
            .then_with(|| a.name.cmp(&b.name))
    });

    processed.truncate(MAX_SYMBOLS);

    for symbol in &processed {
        write_symbol_line(&mut buffer, symbol)?;
    }

    file.write_all(&buffer).await?;
    Ok(())
}

struct ProcessedSymbol {
    name: String,
    total_refs: usize,
    file_refs: Vec<FileRef>,
    total_files: usize,
}

struct FileRef {
    path: String,
    first_line: usize,
}

fn process_symbol(name: &str, locations: &[(String, usize)]) -> Option<ProcessedSymbol> {
    if locations.is_empty() {
        return None;
    }

    let mut file_map: HashMap<&str, Vec<usize>> = HashMap::new();
    for (file, line) in locations {
        file_map.entry(file.as_str()).or_default().push(*line);
    }

    if file_map.len() < MIN_FILES_THRESHOLD {
        return None;
    }

    let total_refs = locations.len();
    let total_files = file_map.len();

    let mut file_sorted: Vec<_> = file_map.into_iter().collect();
    file_sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));

    let file_refs: Vec<FileRef> = file_sorted
        .into_iter()
        .take(MAX_FILES_PER_SYMBOL)
        .map(|(path, lines)| {
            let first_line = lines.into_iter().min().unwrap_or(1);
            FileRef {
                path: path.to_string(),
                first_line,
            }
        })
        .collect();

    Some(ProcessedSymbol {
        name: name.to_string(),
        total_refs,
        file_refs,
        total_files,
    })
}

fn write_symbol_line(buffer: &mut Vec<u8>, symbol: &ProcessedSymbol) -> Result<()> {
    let refs_shown = symbol.file_refs.len();
    let files_omitted = symbol.total_files.saturating_sub(refs_shown);
    let sites_omitted = symbol.total_refs.saturating_sub(refs_shown);

    let ref_strings: Vec<String> = symbol
        .file_refs
        .iter()
        .map(|r| format!("{}:{}", r.path, r.first_line))
        .collect();

    let refs_part = ref_strings.join(", ");

    if files_omitted > 0 {
        writeln!(
            buffer,
            "{} [{}] — {} [+{} sites in {} files]",
            symbol.name, symbol.total_refs, refs_part, sites_omitted, files_omitted
        )?;
    } else {
        writeln!(
            buffer,
            "{} [{}] — {}",
            symbol.name, symbol.total_refs, refs_part
        )?;
    }

    Ok(())
}
