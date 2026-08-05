// mavis_core/src/main.rs
// Entry point. Initializes subsystems, wires them together, runs until shutdown.

use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::mpsc;

mod context_engine;
mod event_bus;
mod executor;
mod memory;
mod models;
mod planner;
mod system;
mod ui;
mod worker_bridge;

use event_bus::EventBus;
use models::event::{Event, EventType};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("MAVIS starting...");

    let bus = Arc::new(EventBus::new());
    let (_shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Context Engine
    let bus_clone = Arc::clone(&bus);
    let mut ctx_engine = context_engine::ContextEngine::new();
    let ctx_handle = tokio::spawn(async move {
        let mut rx = bus_clone.subscribe();
        info!("ContextEngine: listening for events");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = ctx_engine.process_event(event).await {
                        warn!("ContextEngine error: {}", e);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("ContextEngine lagged by {} events", n);
                }
            }
        }
        info!("ContextEngine: shutting down");
    });

    // Executor
    let bus_clone = Arc::clone(&bus);
    let exec_handle = tokio::spawn(async move {
        let mut rx = bus_clone.subscribe();
        info!("Executor: listening for events");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    info!("Executor received: {:?}", event.event_type);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Executor lagged by {} events", n);
                }
            }
        }
        info!("Executor: shutting down");
    });

    // Worker Bridge
    let (worker_event_tx, mut worker_event_rx) = mpsc::channel::<Event>(64);
    let bus_clone = Arc::clone(&bus);
    let mut bridge = worker_bridge::WorkerBridge::new(worker_event_tx);
    let bridge_handle = tokio::spawn(async move {
        if let Err(e) = bridge.run(bus_clone).await {
            warn!("WorkerBridge error: {}", e);
        }
    });

    // Forward worker events back to bus
    let bus_clone = Arc::clone(&bus);
    let _forward_handle = tokio::spawn(async move {
        while let Some(event) = worker_event_rx.recv().await {
            bus_clone.publish(event);
        }
    });

    // Orb UI (stub)
    let _orb_handle = tokio::spawn(async move {
        let _orb = ui::Orb::new();
        info!("Orb: initialized (stub)");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    });

    // Startup event
    bus.publish(Event {
        id: uuid::Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        source: "mavis_core".to_string(),
        event_type: EventType::SystemWake,
        payload: serde_json::json!({ "status": "ready" }),
    });

    info!("MAVIS runtime ready. Press Ctrl+C to shutdown.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received.");
        }
        _ = shutdown_rx.recv() => {
            info!("Shutdown requested via event bus.");
        }
    }

    drop(bus);

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), ctx_handle).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), exec_handle).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), bridge_handle).await;

    info!("MAVIS shutdown complete.");
    Ok(())
}