use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio::time::sleep;
use tracing::{debug, error, warn};

use crate::transport::{Transport, TransportType};

#[derive(Debug, Clone)]
pub struct PingConfig {
    /// Interval between pings in milliseconds
    pub interval_ms: u64,
    /// Timeout for ping response in milliseconds
    pub timeout_ms: u64,
    /// Number of consecutive failures before connection is considered dead
    pub failure_threshold: u32,
}

impl Default for PingConfig {
    fn default() -> Self {
        Self {
            interval_ms: 30000,   // 30 seconds
            timeout_ms: 5000,     // 5 seconds
            failure_threshold: 3, // 3 consecutive failures
        }
    }
}

#[derive(Debug)]
pub struct ConnectionHealth {
    last_pong: Arc<RwLock<Instant>>,
    consecutive_failures: Arc<RwLock<u32>>,
    config: PingConfig,
}

impl ConnectionHealth {
    pub fn new(config: PingConfig) -> Self {
        Self {
            last_pong: Arc::new(RwLock::new(Instant::now())),
            consecutive_failures: Arc::new(RwLock::new(0)),
            config,
        }
    }

    pub async fn start_monitor(&self, mut transport: TransportType, reconnect_tx: mpsc::Sender<()>) -> Result<()> {
        let last_pong = self.last_pong.clone();
        let consecutive_failures = self.consecutive_failures.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            loop {
                sleep(Duration::from_millis(config.interval_ms)).await;

                match Self::send_ping(&mut transport).await {
                    Ok(_) => {
                        // Wait for pong with timeout
                        if !Self::wait_for_pong(last_pong.clone(), config.timeout_ms).await {
                            let mut failures = consecutive_failures.write().await;
                            *failures += 1;

                            if *failures >= config.failure_threshold {
                                warn!("Connection appears stale after {} consecutive failures", failures);
                                if let Err(e) = reconnect_tx.send(()).await {
                                    error!("Failed to send reconnection signal: {}", e);
                                }
                                *failures = 0;
                            }
                        } else {
                            // Reset failures on successful pong
                            *consecutive_failures.write().await = 0;
                        }
                    }
                    Err(e) => {
                        error!("Failed to send ping: {}", e);
                        let mut failures = consecutive_failures.write().await;
                        *failures += 1;
                    }
                }
            }
        });

        Ok(())
    }

    async fn send_ping(transport: &mut TransportType) -> Result<()> {
        let ping_request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "ping",
            "params": {},
            "id": chrono::Utc::now().timestamp_millis()
        });

        Transport::send(transport, serde_json::to_string(&ping_request)?).await.context("Failed to send ping request")
    }

    async fn wait_for_pong(last_pong: Arc<RwLock<Instant>>, timeout_ms: u64) -> bool {
        let timeout = Duration::from_millis(timeout_ms);
        let start = Instant::now();

        while start.elapsed() < timeout {
            let last = *last_pong.read().await;
            if last > start {
                debug!("Received pong response");
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }

        warn!("Ping timeout after {}ms", timeout_ms);
        false
    }

    pub async fn handle_pong(&self) {
        *self.last_pong.write().await = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use crate::transport::stdio;

    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_connection_health_monitoring() {
        let config = PingConfig { interval_ms: 100, timeout_ms: 50, failure_threshold: 2 };

        let health = ConnectionHealth::new(config);
        let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);

        // Create mock transport configured to fail
        let mock_transport = stdio::StdioTransport::new_server();
        let transport = TransportType::Stdio(mock_transport);

        health.start_monitor(transport, reconnect_tx).await.unwrap();

        // Should receive reconnection signal after failure_threshold * interval_ms
        match timeout(Duration::from_millis(300), reconnect_rx.recv()).await {
            Ok(Some(())) => (),
            _ => panic!("Should have received reconnection signal"),
        }
    }
}
