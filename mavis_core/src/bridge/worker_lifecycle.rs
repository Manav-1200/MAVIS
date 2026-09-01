//! Manages the Python worker process: lazy spawn, health checks, crash recovery, idle kill.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use log::{error, info, warn};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::time::timeout;

const SOCKET_PATH: &str = "/tmp/mavis_worker.sock";
const SPAWN_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CRASH_WINDOW: Duration = Duration::from_secs(60);
const MAX_RESTARTS_IN_WINDOW: u32 = 2;

pub struct WorkerLifecycle {
    child: Option<Child>,
    python_path: String,
    worker_module: String,
    last_request: Instant,
    restart_count: u32,
    first_crash: Option<Instant>,
    available: bool,
}

impl WorkerLifecycle {
    pub fn new(python_path: impl Into<String>, worker_module: impl Into<String>) -> Self {
        Self {
            child: None,
            python_path: python_path.into(),
            worker_module: worker_module.into(),
            last_request: Instant::now(),
            restart_count: 0,
            first_crash: None,
            available: true,
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub async fn ensure_running(&mut self) -> Result<()> {
        // If we have a child handle, verify it's actually still alive
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(None) => return Ok(()), // Still running
                Ok(Some(status)) => {
                    warn!("Worker process exited with status: {:?}", status);
                    self.record_crash();
                }
                Err(e) => {
                    warn!("Failed to poll worker status: {}", e);
                    self.record_crash();
                }
            }
        }

        if !self.available {
            anyhow::bail!("Worker unavailable due to repeated crashes");
        }

        if let Some(first) = self.first_crash {
            if self.restart_count >= MAX_RESTARTS_IN_WINDOW && first.elapsed() < CRASH_WINDOW {
                self.available = false;
                anyhow::bail!("Worker crashed too many times; marked unavailable");
            }
            if first.elapsed() >= CRASH_WINDOW {
                self.restart_count = 0;
                self.first_crash = None;
            }
        }

        info!("Spawning Python worker ({})...", self.worker_module);

        if Path::new(SOCKET_PATH).exists() {
            let _ = tokio::fs::remove_file(SOCKET_PATH).await;
        }

        let mut child = Command::new(&self.python_path)
            .arg("-m")
            .arg(&self.worker_module)
            .env("PYTHONUNBUFFERED", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to spawn Python worker")?;

        // Forward worker stdout/stderr to Rust logs
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::info!("[worker] {}", line);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::warn!("[worker] {}", line);
                }
            });
        }

        // Wait for socket file to appear
        let socket_ready = timeout(SPAWN_TIMEOUT, async {
            while !Path::new(SOCKET_PATH).exists() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;

        if socket_ready.is_err() {
            let _ = child.kill().await;
            anyhow::bail!("Worker did not create socket within {:?}", SPAWN_TIMEOUT);
        }

        self.child = Some(child);
        info!("Worker spawned and socket ready");
        Ok(())
    }

    pub async fn send_request(&mut self, request: &str) -> Result<String> {
        for attempt in 1..=2 {
            self.ensure_running().await?;

            let stream = match UnixStream::connect(SOCKET_PATH).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("Worker socket connection failed (attempt {}): {}", attempt, e);
                    self.record_crash();
                    if attempt == 2 {
                        return Err(anyhow::anyhow!("Failed to connect to worker socket: {}", e));
                    }
                    continue;
                }
            };

            let (mut reader, mut writer) = stream.into_split();

            // Length-prefixed write
            let req_bytes = request.as_bytes();
            writer
                .write_all(&(req_bytes.len() as u32).to_le_bytes())
                .await?;
            writer.write_all(req_bytes).await?;
            writer.flush().await?;

            self.last_request = Instant::now();

            // Length-prefixed read
            let read_fut = async {
                let mut len_bytes = [0u8; 4];
                reader.read_exact(&mut len_bytes).await?;
                let resp_len = u32::from_le_bytes(len_bytes) as usize;
                let mut buf = vec![0u8; resp_len];
                reader.read_exact(&mut buf).await?;
                Ok::<_, std::io::Error>(String::from_utf8_lossy(&buf).to_string())
            };

            match timeout(REQUEST_TIMEOUT, read_fut).await {
                Ok(Ok(resp)) => return Ok(resp),
                Ok(Err(e)) => {
                    error!("IO error reading worker response (attempt {}): {}", attempt, e);
                    self.record_crash();
                    if attempt == 2 {
                        return Err(e.into());
                    }
                }
                Err(_) => {
                    error!("Worker request timed out (attempt {})", attempt);
                    self.record_crash();
                    if attempt == 2 {
                        anyhow::bail!("Worker request timed out");
                    }
                }
            }
        }
        anyhow::bail!("Worker request failed after retries")
    }

    pub async fn health_check(&mut self) -> bool {
        match self.send_request(r#"{"type":"health"}"#).await {
            Ok(resp) => {
                let ok = serde_json::from_str::<serde_json::Value>(&resp)
                    .ok()
                    .and_then(|v| {
                        v.get("payload")
                            .and_then(|p| p.get("status"))
                            .and_then(|s| s.as_str())
                            .map(|s| s == "ok")
                    })
                    .unwrap_or(false);
                if !ok {
                    warn!("Health check unexpected response: {}", resp);
                }
                ok
            }
            Err(e) => {
                warn!("Health check failed: {}", e);
                false
            }
        }
    }

    pub fn is_idle(&self, idle_duration: Duration) -> bool {
        self.last_request.elapsed() > idle_duration
    }

    pub fn record_crash(&mut self) {
        self.restart_count += 1;
        if self.first_crash.is_none() {
            self.first_crash = Some(Instant::now());
        }
        self.child = None;
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            info!("Killing worker process...");
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        if Path::new(SOCKET_PATH).exists() {
            let _ = tokio::fs::remove_file(SOCKET_PATH).await;
        }
    }
}