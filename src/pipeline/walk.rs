use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct WalkResult {
    pub files: Vec<PathBuf>,
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

            if path.starts_with(root.join(".atlas")) {
                return ignore::WalkState::Continue;
            }

            if path.starts_with(root.join("target")) {
                return ignore::WalkState::Continue;
            }

            if path.extension().is_some_and(|e| e == "rs") {
                files.lock().unwrap().push(path.to_path_buf());
            }

            ignore::WalkState::Continue
        })
    });

    let files = files.into_inner().unwrap();

    Ok(WalkResult { files })
}
