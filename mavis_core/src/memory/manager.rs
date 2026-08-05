// Memory Manager: central coordinator for all memory layers.
// Currently owns WorkingMemory; future iterations add SQLite-backed stores.

use crate::memory::working::WorkingMemory;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MemoryManager {
    pub working: Arc<RwLock<WorkingMemory>>,
}

impl MemoryManager {
    pub fn new() -> Self {
        Self {
            working: Arc::new(RwLock::new(WorkingMemory::new())),
        }
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
    }
}