pub struct RustType {
    pub value: i32,
}

impl RustType {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn double(&self) -> i32 {
        self.value * 2
    }
}

pub trait Calculator {
    fn calculate(&self, input: i32) -> i32;
}

impl Calculator for RustType {
    fn calculate(&self, input: i32) -> i32 {
        self.value + input
    }
}
