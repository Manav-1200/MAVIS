// mavis_core/src/memory/permanent.rs
// Permanent Memory: user identity, core preferences. SQLite-backed.

pub struct PermanentStore;

impl PermanentStore {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }
}