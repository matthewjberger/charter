use crate::Entity;

pub trait Serialize {
    fn to_json(&self) -> String;
    fn from_json(json: &str) -> Option<Self>
    where
        Self: Sized;
}

impl Serialize for Entity {
    fn to_json(&self) -> String {
        format!(
            r#"{{"id":{},"name":"{}","active":{}}}"#,
            self.id, self.name, self.active
        )
    }

    fn from_json(json: &str) -> Option<Self> {
        if json.contains("id") {
            Some(Entity::default())
        } else {
            None
        }
    }
}

impl Entity {
    pub fn serialize_compact(&self) -> Vec<u8> {
        self.to_json().into_bytes()
    }
}
