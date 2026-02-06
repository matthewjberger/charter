pub mod python;
pub mod rust;

use anyhow::Result;

use crate::extract::attributes::{CfgInfo, DeriveInfo};
use crate::extract::calls::CallInfo;
use crate::extract::complexity::FunctionComplexity;
use crate::extract::errors::ErrorInfo;
use crate::extract::imports::{ImportInfo, ReExport};
use crate::extract::language::Language;
use crate::extract::safety::{
    AsyncInfo, DocInfo, FeatureFlagInfo, GenericConstraints, LifetimeInfo, PythonSafetyInfo,
    SafetyInfo, TestInfo,
};
use crate::extract::symbols::{FileSymbols, FunctionBody};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ParsedFile {
    pub symbols: FileSymbols,
    pub module_doc: Option<String>,
    pub derives: Vec<DeriveInfo>,
    pub cfgs: Vec<CfgInfo>,
    pub imports: Vec<ImportInfo>,
    pub re_exports: Vec<ReExport>,
    pub has_test_module: bool,
    pub test_functions: Vec<String>,
    pub identifier_locations: Vec<(String, usize)>,
    pub complexity: Vec<FunctionComplexity>,
    pub call_graph: Vec<CallInfo>,
    pub error_info: Vec<ErrorInfo>,
    pub captured_bodies: Vec<CapturedBody>,
    pub safety: SafetyInfo,
    pub python_safety: PythonSafetyInfo,
    pub lifetimes: LifetimeInfo,
    pub async_info: AsyncInfo,
    pub feature_flags: FeatureFlagInfo,
    pub doc_info: DocInfo,
    pub generic_constraints: GenericConstraints,
    pub test_info: TestInfo,
    pub language: Option<Language>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapturedBody {
    pub function_name: String,
    pub impl_type: Option<String>,
    pub line: usize,
    pub body: FunctionBody,
    pub importance_score: u32,
}

pub fn parse_file(content: &str, file_path: &str, language: Language) -> Result<ParsedFile> {
    let mut result = match language {
        Language::Rust => rust::parse_rust_file(content, file_path)?,
        Language::Python => python::parse_python_file(content, file_path)?,
    };
    result.language = Some(language);
    Ok(result)
}

pub fn is_pascal_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let first = match name.chars().next() {
        Some(c) => c,
        None => return false,
    };

    if !first.is_ascii_uppercase() {
        return false;
    }

    let has_lowercase = name.chars().any(|c| c.is_ascii_lowercase());
    let all_valid = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');

    has_lowercase && all_valid
}
