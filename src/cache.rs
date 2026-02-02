mod types;

pub use types::{CacheEntry, FileData};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    pub entries: HashMap<String, CacheEntry>,
}

impl Cache {
    pub async fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let bytes = tokio::fs::read(path).await?;
        let cache: Cache = bincode::deserialize(&bytes)?;
        Ok(cache)
    }

    pub async fn save(&self, path: &Path) -> Result<()> {
        let bytes = bincode::serialize(self)?;
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    pub fn get(&self, path: &str) -> Option<&CacheEntry> {
        self.entries.get(path)
    }
}
