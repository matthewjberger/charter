use serde::{Deserialize, Serialize};

use crate::pipeline::ParsedFile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub hash: String,
    pub mtime: u64,
    pub size: u64,
    pub lines: usize,
    pub data: FileData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileData {
    pub parsed: ParsedFile,
}
