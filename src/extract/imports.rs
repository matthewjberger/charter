use crate::extract::symbols::Visibility;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReExport {
    pub source_path: String,
    pub visibility: Visibility,
    pub line: usize,
}
