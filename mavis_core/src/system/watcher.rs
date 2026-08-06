// mavis_core/src/system/watcher.rs
// Watches Downloads and Desktop for changes. Emits ContextUpdate events.

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{info, warn};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;

pub struct FileWatcher {
    bus: Arc<EventBus>,
    paths: Vec<PathBuf>,
}

impl FileWatcher {
    pub fn new(bus: Arc<EventBus>) -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let paths = vec![home.join("Downloads"), home.join("Desktop")]
            .into_iter()
            .filter(|p| p.exists())
            .collect();

        Self { bus, paths }
    }

    pub async fn run(&mut self) {
        if self.paths.is_empty() {
            warn!("FileWatcher: no valid paths to watch");
            return;
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel::<notify::Event>(64);

        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.try_send(event);
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                warn!("FileWatcher: failed to create watcher: {}", e);
                return;
            }
        };

        for path in &self.paths {
            if let Err(e) = watcher.watch(path, RecursiveMode::NonRecursive) {
                warn!("FileWatcher: failed to watch {}: {}", path.display(), e);
            } else {
                info!("FileWatcher: watching {}", path.display());
            }
        }

        // Watcher must stay alive for the duration of the loop
        let _watcher = watcher;

        while let Some(event) = rx.recv().await {
            let paths: Vec<String> = event
                .paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let kind = format!("{:?}", event.kind);
            info!("FileWatcher: {} — {:?}", kind, paths);

            let bus_event = Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "file_watcher".to_string(),
                event_type: EventType::ContextUpdate,
                payload: serde_json::json!({
                    "kind": kind,
                    "paths": paths,
                }),
            };
            self.bus.publish(bus_event);
        }
    }
}