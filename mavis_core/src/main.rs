// mavis_core/src/main.rs
// Entry point. Initializes subsystems and runs until Ctrl+C.

use anyhow::Result;
use log::info;

mod context_engine;
mod event_bus;
mod executor;
mod memory;
mod models;
mod planner;
mod system;
mod ui;
mod worker_bridge;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("MAVIS starting...");

    let _bus = event_bus::EventBus::new();
    let _ctx = context_engine::ContextEngine::new();
    let _orb = ui::Orb::new();

    info!("MAVIS runtime ready.");

    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received. Exiting.");
    Ok(())
}