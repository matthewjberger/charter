use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct FunctionId {
    pub file: String,
    pub name: String,
    pub impl_type: Option<String>,
}

impl FunctionId {
    pub fn qualified_name(&self) -> String {
        match &self.impl_type {
            Some(type_name) => format!("{}::{}", type_name, self.name),
            None => self.name.clone(),
        }
    }
}

impl std::fmt::Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.qualified_name())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub target: String,
    pub target_type: Option<String>,
    pub line: usize,
    pub is_async_call: bool,
    pub is_try_call: bool,
}

impl CallEdge {
    pub fn qualified_target(&self) -> String {
        match &self.target_type {
            Some(type_name) => format!("{}::{}", type_name, self.target),
            None => self.target.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub caller: FunctionId,
    pub callees: Vec<CallEdge>,
    pub line: usize,
}

impl CallInfo {
    pub fn new(file: String, name: String, impl_type: Option<String>, line: usize) -> Self {
        Self {
            caller: FunctionId {
                file,
                name,
                impl_type,
            },
            callees: Vec::new(),
            line,
        }
    }
}
