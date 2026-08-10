#![allow(dead_code)]

use crate::memory::episodic::EpisodicStore;
use crate::memory::permanent::PermanentStore;
use crate::memory::working::WorkingMemory;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub struct MemoryManager {
    pub working: Arc<RwLock<WorkingMemory>>,
    pub permanent: Arc<Mutex<PermanentStore>>,
    pub episodic: Arc<Mutex<EpisodicStore>>,
}

impl MemoryManager {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let permanent_db = data_dir.join("permanent.db");
        let episodic_db = data_dir.join("episodic.db");

        Ok(Self {
            working: Arc::new(RwLock::new(WorkingMemory::new())),
            permanent: Arc::new(Mutex::new(PermanentStore::new(&permanent_db)?)),
            episodic: Arc::new(Mutex::new(EpisodicStore::new(&episodic_db)?)),
        })
    }
}
