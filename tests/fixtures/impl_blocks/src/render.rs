use crate::Entity;

pub trait Render {
    fn render(&self) -> String;
}

impl Render for Entity {
    fn render(&self) -> String {
        format!("Entity({}, {})", self.id, self.name)
    }
}

impl Entity {
    pub fn render_debug(&self) -> String {
        format!("Debug: {:?}", self.id)
    }
}
