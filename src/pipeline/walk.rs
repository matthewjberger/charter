use anyhow::Result;
use ignore::WalkBuilder;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::extract::language::Language;

pub struct WalkResult {
    pub files: Vec<PathBuf>,
    #[allow(dead_code)]
    pub language_counts: HashMap<Language, usize>,
}

pub async fn walk_directory(root: &Path) -> Result<WalkResult> {
    let root = root.to_path_buf();

    tokio::task::spawn_blocking(move || walk_directory_sync(&root)).await?
}

fn walk_directory_sync(root: &Path) -> Result<WalkResult> {
    let files = Mutex::new(Vec::new());

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .threads(num_cpus::get())
        .build_parallel();

    walker.run(|| {
        let files = &files;

        Box::new(move |entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };

            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                return ignore::WalkState::Continue;
            }

            let path = entry.path();

            if path.starts_with(root.join(".charter")) {
                return ignore::WalkState::Continue;
            }

            if path.starts_with(root.join("target")) {
                return ignore::WalkState::Continue;
            }

            if path.starts_with(root.join("__pycache__")) {
                return ignore::WalkState::Continue;
            }

            if path.starts_with(root.join(".venv")) || path.starts_with(root.join("venv")) {
                return ignore::WalkState::Continue;
            }

            if path.starts_with(root.join(".git")) {
                return ignore::WalkState::Continue;
            }

            if let Some(ext) = path.extension() {
                let ext_str = ext.to_str().unwrap_or("");
                if ext_str == "rs" || ext_str == "py" || ext_str == "pyi" {
                    files
                        .lock()
                        .expect("lock poisoned")
                        .push(path.to_path_buf());
                }
            }

            ignore::WalkState::Continue
        })
    });

    let files = files.into_inner().expect("lock poisoned");

    let mut language_counts = HashMap::new();
    for file in &files {
        if let Some(lang) = Language::from_path(file) {
            *language_counts.entry(lang).or_insert(0) += 1;
        }
    }

    Ok(WalkResult {
        files,
        language_counts,
    })
}
