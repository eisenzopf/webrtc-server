use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde::Serialize;
use log::{debug, warn, error};
use tokio::time::sleep;

pub mod dashboard;
pub use dashboard::ConnectionMonitor;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStats {
    pub peer_id: String,
    pub connection_state: String,
    pub ice_connection_state: String,
    #[serde(skip_serializing)]
    pub connected_at: Option<Instant>,
    #[serde(skip_serializing)]
    pub last_activity: Instant,
    pub ice_candidates_received: u32,
    pub ice_candidates_sent: u32,
    pub data_channels_open: u32,
    pub error_count: u32,
}

// Background task to periodically clean up stale connections
pub async fn run_connection_monitor(monitor: Arc<ConnectionMonitor>) {
    loop {
        // Monitor connections periodically
        monitor.check_connections().await;
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    }
} 