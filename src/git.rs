use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub commit_short: String,
}

pub async fn get_git_info(root: &Path) -> Result<GitInfo> {
    let commit_short = get_commit_short(root).await?;
    Ok(GitInfo { commit_short })
}

async fn get_commit_short(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--short")
        .arg("HEAD")
        .current_dir(root)
        .output()
        .await?;

    if !output.status.success() {
        return Err(anyhow!("git rev-parse failed"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn get_churn_data(root: &Path) -> Result<HashMap<PathBuf, u32>> {
    let output = Command::new("git")
        .arg("log")
        .arg("--format=")
        .arg("--name-only")
        .arg("--since=90 days ago")
        .current_dir(root)
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Ok(HashMap::new()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut churn: HashMap<PathBuf, u32> = HashMap::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let path = root.join(line);
        *churn.entry(path).or_insert(0) += 1;
    }

    Ok(churn)
}
