use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeriveInfo {
    pub target: String,
    pub traits: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgInfo {
    pub condition: String,
    pub line: usize,
}
