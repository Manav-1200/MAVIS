// mavis_core/src/memory/manager.rs

use crate::memory::episodic::EpisodicStore;
use crate::memory::permanent::PermanentStore;
use crate::memory::working::WorkingMemory;
use log::info;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct MemoryManager {
    pub working: Arc<RwLock<WorkingMemory>>,
    pub permanent: Arc<Mutex<PermanentStore>>,
    pub episodic: Arc<Mutex<EpisodicStore>>,
    working_file: std::path::PathBuf,
}

impl MemoryManager {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let permanent_db = data_dir.join("permanent.db");
        let episodic_db = data_dir.join("episodic.db");
        let working_file = data_dir.join("working_memory.json");

        let working = if working_file.exists() {
            match std::fs::read_to_string(&working_file) {
                Ok(json) => match WorkingMemory::from_json(&json) {
                    Ok(wm) => {
                        info!(
                            "Memory: restored working memory from snapshot ({} events, user={:?})",
                            wm.events.len(),
                            wm.user_name
                        );
                        wm
                    }
                    Err(e) => {
                        log::warn!("Memory: failed to parse working memory snapshot: {}", e);
                        WorkingMemory::new()
                    }
                },
                Err(e) => {
                    log::warn!("Memory: failed to read working memory snapshot: {}", e);
                    WorkingMemory::new()
                }
            }
        } else {
            WorkingMemory::new()
        };

        Ok(Self {
            working: Arc::new(RwLock::new(working)),
            permanent: Arc::new(Mutex::new(PermanentStore::new(&permanent_db)?)),
            episodic: Arc::new(Mutex::new(EpisodicStore::new(&episodic_db)?)),
            working_file,
        })
    }

    /// Serialize working memory to disk for session recovery.
    pub async fn save_working(&self) -> anyhow::Result<()> {
        let wm = self.working.read().await;
        let json = wm.to_json()?;
        drop(wm);
        tokio::fs::write(&self.working_file, json).await?;
        Ok(())
    }
}