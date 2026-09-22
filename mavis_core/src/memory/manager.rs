// mavis_core/src/memory/manager.rs

use crate::memory::episodic::EpisodicStore;
use crate::memory::entities::EntityStore;
use crate::memory::long_term::LongTermMemory;
use crate::memory::recall::RecallStore;
use crate::memory::working::WorkingMemory;
use log::info;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct MemoryManager {
    pub working: Arc<RwLock<WorkingMemory>>,
    pub episodic: Arc<Mutex<EpisodicStore>>,
    /// Searchable memory — what the user said, and what MAVIS replied.
    pub recall: Arc<Mutex<RecallStore>>,
    /// Daily summaries, consolidated from recall before decay removes it.
    pub long_term: Arc<Mutex<LongTermMemory>>,
    /// Projects, apps and files the user works with, and what co-occurs.
    pub entities: Arc<Mutex<EntityStore>>,
    working_file: std::path::PathBuf,
}

impl MemoryManager {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let episodic_db = data_dir.join("episodic.db");
        let recall_db = data_dir.join("recall.db");
        let long_term_db = data_dir.join("long_term.db");
        let entities_db = data_dir.join("entities.db");
        let working_file = data_dir.join("working_memory.json");

        let working = if working_file.exists() {
            match std::fs::read_to_string(&working_file) {
                Ok(json) => match WorkingMemory::from_json(&json) {
                    Ok(mut wm) => {
                        if let Some(bad) = wm.sanitize() {
                            log::warn!(
                                "Memory: discarded stored user name {:?} — learned by an older build's rules; say \"my name is …\" again",
                                bad
                            );
                        }
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

        // Prune expired memories once at startup — cheap, and keeps old
        // chatter from crowding out real matches in search results.
        let recall_store = RecallStore::new(&recall_db)?;
        if let Err(e) = recall_store.purge_expired() {
            log::warn!("Memory: purge failed: {}", e);
        }

        Ok(Self {
            working: Arc::new(RwLock::new(working)),
            episodic: Arc::new(Mutex::new(EpisodicStore::new(&episodic_db)?)),
            recall: Arc::new(Mutex::new(recall_store)),
            long_term: Arc::new(Mutex::new(LongTermMemory::new(&long_term_db)?)),
            entities: Arc::new(Mutex::new(EntityStore::new(&entities_db)?)),
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