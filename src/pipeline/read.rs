use anyhow::Result;
use std::path::Path;

const MMAP_THRESHOLD: u64 = 64 * 1024;

pub async fn read_file(path: &Path, size: u64) -> Result<Vec<u8>> {
    if size > MMAP_THRESHOLD {
        read_mmap(path).await
    } else {
        read_direct(path).await
    }
}

async fn read_direct(path: &Path) -> Result<Vec<u8>> {
    let content = tokio::fs::read(path).await?;
    Ok(content)
}

async fn read_mmap(path: &Path) -> Result<Vec<u8>> {
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok(mmap.to_vec())
    })
    .await?
}
