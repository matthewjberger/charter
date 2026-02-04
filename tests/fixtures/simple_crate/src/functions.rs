use crate::types::{Config, ProcessError, Processor, Status};

pub fn process(config: &Config, input: &str) -> Result<Status, ProcessError> {
    if !validate_input(input) {
        return Err(ProcessError {
            message: "Invalid input".to_string(),
            code: 1,
        });
    }

    let result = transform(input);

    if config.enabled {
        Ok(Status::Complete(result))
    } else {
        Ok(Status::Pending)
    }
}

fn validate_input(input: &str) -> bool {
    !input.is_empty() && input.len() < 1000
}

fn transform(input: &str) -> String {
    input.to_uppercase()
}

pub struct SimpleProcessor {
    config: Config,
}

impl SimpleProcessor {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Processor for SimpleProcessor {
    fn process(&self, input: &str) -> Result<String, ProcessError> {
        if self.config.enabled {
            Ok(transform(input))
        } else {
            Err(ProcessError {
                message: "Processor disabled".to_string(),
                code: 2,
            })
        }
    }
}

pub async fn async_process(input: &str) -> Result<String, ProcessError> {
    Ok(transform(input))
}

pub fn complex_function(a: i32, b: i32, c: i32) -> i32 {
    if a > 0 {
        if b > 0 {
            if c > 0 {
                a + b + c
            } else if c < -10 {
                a + b - c
            } else {
                a + b
            }
        } else if b < -10 {
            if c > 0 {
                a - b + c
            } else {
                a - b - c
            }
        } else {
            a
        }
    } else if a < -10 {
        match b {
            0..=10 => b + c,
            _ => b - c,
        }
    } else {
        0
    }
}
