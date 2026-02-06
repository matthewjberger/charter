use crate::extract::symbols::Visibility;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub path: String,
    pub line: usize,
    pub kind: ImportKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ImportKind {
    #[default]
    RustUse,
    PythonImport {
        module: String,
    },
    PythonFromImport {
        module: String,
        names: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReExport {
    pub source_path: String,
    pub visibility: Visibility,
    pub line: usize,
}
