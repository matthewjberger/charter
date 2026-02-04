pub struct Entity {
    pub id: u64,
    pub name: String,
    pub active: bool,
}

impl Entity {
    pub fn new(id: u64, name: String) -> Self {
        Self {
            id,
            name,
            active: true,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            active: false,
        }
    }
}
