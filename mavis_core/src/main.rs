// Entry point. Initializes subsystems, wires them together, runs until shutdown.

use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::mpsc;

mod bridge;
mod context_engine;
mod event_bus;
mod executor;
mod memory;
mod models;
mod planner;
mod system;
mod ui;
mod tts;

use event_bus::EventBus;
use models::event::{Event, EventType};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("MAVIS starting...");

    let bus = Arc::new(EventBus::new());
    let (_shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Context Engine
    let data_dir = std::path::Path::new("../memory");
    let bus_pub = Arc::clone(&bus);
    let bus_sub = Arc::clone(&bus);
    let mut ctx_engine = context_engine::ContextEngine::new(bus_pub, data_dir)?;
    let ctx_handle = tokio::spawn(async move {
        let mut rx = bus_sub.subscribe();
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

    // Planner
    let bus_clone = Arc::clone(&bus);
    let mut planner = planner::Planner::new(bus_clone);
    let planner_handle = tokio::spawn(async move {
        planner.run().await;
    });

    // Executor
    let bus_clone = Arc::clone(&bus);
    let mut executor = executor::Executor::new(bus_clone);
    let exec_handle = tokio::spawn(async move {
        executor.run().await;
    });

    // DBus Integration
    let bus_clone = Arc::clone(&bus);
    let mut dbus = system::dbus::DbusIntegration::new(bus_clone);
    let dbus_handle = tokio::spawn(async move {
        dbus.run().await;
    });

    // Hotkey Manager
    let bus_clone = Arc::clone(&bus);
    let mut hotkeys = system::hotkeys::HotkeyManager::new(bus_clone);
    let hotkeys_handle = tokio::spawn(async move {
        hotkeys.run().await;
    });

    // File Watcher
    let bus_clone = Arc::clone(&bus);
    let mut watcher = system::watcher::FileWatcher::new(bus_clone);
    let watcher_handle = tokio::spawn(async move {
        watcher.run().await;
    });

    // Worker Bridge
    let bus_clone = Arc::clone(&bus);
    let bridge_handle = tokio::spawn(async move {
        let bridge = bridge::worker_bridge::WorkerBridge::new();
        if let Err(e) = bridge.run(bus_clone).await {
            warn!("WorkerBridge error: {}", e);
        }
    });

    // Orb UI
    let bus_clone = Arc::clone(&bus);
    let orb_handle = tokio::spawn(async move {
        let orb = ui::Orb::new();
        info!("Orb: initialized");

        let mut rx = bus_clone.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let EventType::UiStateChange = event.event_type {
                        if let Some(state_str) =
                            event.payload.get("state").and_then(|v| v.as_str())
                        {
                            let state = match state_str {
                                "idle" => ui::OrbState::Idle,
                                "listening" => ui::OrbState::Listening,
                                "thinking" => ui::OrbState::Thinking,
                                "speaking" => ui::OrbState::Speaking,
                                "working" => ui::OrbState::Working,
                                "error" => ui::OrbState::Error,
                                "asleep" => ui::OrbState::Asleep,
                                _ => ui::OrbState::Idle,
                            };
                            orb.set_state(state);
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Orb lagged by {} events", n);
                }
            }
        }
        orb.shutdown();
        info!("Orb: shutting down");
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

    // CRITICAL: close the bus first. This wakes up every rx.recv().await immediately.
    bus.close();
    info!("EventBus closed — signaling all subsystems to shut down");

    // Join everything concurrently so total shutdown is capped at 5s, not 5s × N.
    let timeout = std::time::Duration::from_secs(5);
    let _ = tokio::join!(
        tokio::time::timeout(timeout, ctx_handle),
        tokio::time::timeout(timeout, planner_handle),
        tokio::time::timeout(timeout, exec_handle),
        tokio::time::timeout(timeout, dbus_handle),
        tokio::time::timeout(timeout, hotkeys_handle),
        tokio::time::timeout(timeout, watcher_handle),
        tokio::time::timeout(timeout, bridge_handle),
        tokio::time::timeout(timeout, orb_handle),
    );

    info!("MAVIS shutdown complete.");
    Ok(())
}