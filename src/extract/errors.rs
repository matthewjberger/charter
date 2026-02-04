use serde::{Deserialize, Serialize};

use super::calls::FunctionId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorReturnType {
    Result { ok_type: String, err_type: String },
    Option { some_type: String },
    Neither,
}

impl ErrorReturnType {
    pub fn is_fallible(&self) -> bool {
        !matches!(self, ErrorReturnType::Neither)
    }
}

impl std::fmt::Display for ErrorReturnType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorReturnType::Result { ok_type, err_type } => {
                write!(f, "Result<{}, {}>", ok_type, err_type)
            }
            ErrorReturnType::Option { some_type } => {
                write!(f, "Option<{}>", some_type)
            }
            ErrorReturnType::Neither => write!(f, "()"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationPoint {
    pub line: usize,
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorOriginKind {
    ErrConstructor,
    AnyhowMacro,
    BailMacro,
    NoneReturn,
    CustomError,
}

impl std::fmt::Display for ErrorOriginKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorOriginKind::ErrConstructor => write!(f, "Err()"),
            ErrorOriginKind::AnyhowMacro => write!(f, "anyhow!()"),
            ErrorOriginKind::BailMacro => write!(f, "bail!()"),
            ErrorOriginKind::NoneReturn => write!(f, "None"),
            ErrorOriginKind::CustomError => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorOrigin {
    pub line: usize,
    pub kind: ErrorOriginKind,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub function_id: FunctionId,
    pub return_type: ErrorReturnType,
    pub propagation_points: Vec<PropagationPoint>,
    pub error_origins: Vec<ErrorOrigin>,
    pub line: usize,
}

impl ErrorInfo {
    pub fn new(
        file: String,
        name: String,
        impl_type: Option<String>,
        return_type: ErrorReturnType,
        line: usize,
    ) -> Self {
        Self {
            function_id: FunctionId {
                file,
                name,
                impl_type,
            },
            return_type,
            propagation_points: Vec::new(),
            error_origins: Vec::new(),
            line,
        }
    }

    pub fn is_error_source(&self) -> bool {
        !self.error_origins.is_empty()
    }

    pub fn propagation_count(&self) -> usize {
        self.propagation_points.len()
    }
}
