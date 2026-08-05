// mavis_core/src/worker_bridge.rs
// IPC bridge between Rust core and Python AI worker via Unix domain socket.

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use anyhow::{Context, Result};
use log::{error, info, warn};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

const SOCKET_PATH: &str = "/tmp/mavis_worker.sock";

pub struct WorkerBridge {
    socket_path: PathBuf,
    event_tx: mpsc::Sender<Event>,
}

impl WorkerBridge {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self {
            socket_path: PathBuf::from(SOCKET_PATH),
            event_tx,
        }
    }

    /// Attempt to connect to the Python worker. Retry with backoff.
    pub async fn run(&mut self, bus: Arc<EventBus>) -> Result<()> {
        // Wait for socket to exist (Python worker may still be starting)
        let stream = Self::wait_for_socket(&self.socket_path).await?;
        info!("WorkerBridge: connected to {}", self.socket_path.display());

        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        // Channel for outbound requests from the bus to the worker
        let (req_tx, mut req_rx) = mpsc::channel::<Event>(64);

        // Subscribe to bus events and forward WorkerRequests to the worker
        let bus_clone = Arc::clone(&bus);
        let event_tx = self.event_tx.clone();
        let _bus_handle = tokio::spawn(async move {
            let mut rx = bus_clone.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if event.event_type == EventType::WorkerRequest {
                            if let Err(e) = req_tx.send(event).await {
                                error!("WorkerBridge: failed to queue request: {}", e);
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WorkerBridge lagged by {} events", n);
                    }
                }
            }
        });

        // Read responses from worker and publish them back to the bus
        let bus_clone = Arc::clone(&bus);
        let read_handle = tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        info!("WorkerBridge: received response {:?}", event.event_type);
                        bus_clone.publish(event);
                    }
                    Err(e) => {
                        warn!("WorkerBridge: failed to parse response: {}", e);
                    }
                }
            }
            info!("WorkerBridge: read loop ended");
        });

        // Write requests to worker
        while let Some(event) = req_rx.recv().await {
            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(e) => {
                    error!("WorkerBridge: failed to serialize event: {}", e);
                    continue;
                }
            };
            if let Err(e) = writer.write_all(json.as_bytes()).await {
                error!("WorkerBridge: write error: {}", e);
                break;
            }
            if let Err(e) = writer.write_all(b"\n").await {
                error!("WorkerBridge: write error: {}", e);
                break;
            }
            if let Err(e) = writer.flush().await {
                error!("WorkerBridge: flush error: {}", e);
                break;
            }
        }

        let _ = read_handle.await;
        info!("WorkerBridge: disconnected");
        Ok(())
    }

    async fn wait_for_socket(path: &PathBuf) -> Result<UnixStream> {
        let max_retries = 30;
        for i in 0..max_retries {
            if path.exists() {
                return UnixStream::connect(path)
                    .await
                    .with_context(|| format!("failed to connect to {}", path.display()));
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if i % 5 == 0 {
                info!("WorkerBridge: waiting for worker socket... ({}/{})", i, max_retries);
            }
        }
        anyhow::bail!("worker socket did not appear after {} retries", max_retries)
    }
}