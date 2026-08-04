// mavis_core/src/context_engine.rs
// Central nervous system. Owns Working Memory and context window.

use crate::models::event::Event;

pub struct ContextEngine;

impl ContextEngine {
    pub fn new() -> Self {
        Self
    }

    pub async fn process_event(&mut self, _event: Event) -> anyhow::Result<()> {
        // Phase 2: route event, update working memory, decide if planner runs
        Ok(())
    }
}
