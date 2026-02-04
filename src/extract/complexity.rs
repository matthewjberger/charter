use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportanceTier {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for ImportanceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportanceTier::High => write!(f, "high"),
            ImportanceTier::Medium => write!(f, "medium"),
            ImportanceTier::Low => write!(f, "low"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    pub cyclomatic: u32,
    pub line_count: u32,
    pub nesting_depth: u32,
    pub call_sites: u32,
    pub churn_score: u32,
    pub is_public: bool,
    pub is_test: bool,
}

impl ComplexityMetrics {
    pub fn importance_score(&self) -> u32 {
        if self.is_test {
            return 0;
        }
        (self.cyclomatic * 2)
            + (self.line_count / 10)
            + (self.call_sites * 3)
            + (self.churn_score * 2)
            + if self.is_public { 10 } else { 0 }
    }

    pub fn tier(&self) -> ImportanceTier {
        match self.importance_score() {
            score if score >= 30 => ImportanceTier::High,
            score if score >= 15 => ImportanceTier::Medium,
            _ => ImportanceTier::Low,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionComplexity {
    pub name: String,
    pub impl_type: Option<String>,
    pub line: usize,
    pub metrics: ComplexityMetrics,
}

impl FunctionComplexity {
    pub fn qualified_name(&self) -> String {
        match &self.impl_type {
            Some(type_name) => format!("{}::{}", type_name, self.name),
            None => self.name.clone(),
        }
    }
}
