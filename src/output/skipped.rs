use anyhow::Result;
use std::io::Write;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::pipeline::SkippedFile;

pub async fn write_skipped(charter_dir: &Path, skipped: &[SkippedFile], stamp: &str) -> Result<()> {
    let path = charter_dir.join("skipped.md");
    let mut file = File::create(&path).await?;

    let mut buffer = Vec::with_capacity(16 * 1024);

    writeln!(buffer, "{}", stamp)?;
    writeln!(buffer)?;

    for skipped_file in skipped {
        let path_str = skipped_file.path.to_string_lossy().replace('\\', "/");
        writeln!(buffer, "{} - {}", path_str, skipped_file.reason)?;
    }

    file.write_all(&buffer).await?;
    Ok(())
}
