pub struct Config {
    pub name: String,
    pub enabled: bool,
    pub max_retries: u32,
}

pub enum Status {
    Pending,
    Running { progress: f32 },
    Complete(String),
    Failed { error: String, retries: u32 },
}

pub trait Processor {
    fn process(&self, input: &str) -> Result<String, ProcessError>;
    fn validate(&self, input: &str) -> bool {
        !input.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ProcessError {
    pub message: String,
    pub code: i32,
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProcessError {}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProcessError {}
