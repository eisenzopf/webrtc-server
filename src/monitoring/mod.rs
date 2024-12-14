use log::{info, warn, debug};
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use serde::{Serialize, Serializer};
use crate::signaling::connection_state::ConnectionState;
use tokio::sync::broadcast;
use chrono;
use warp::ws::Message as WarpMessage;

#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub connection_state: String,
    pub last_activity: Instant,
    pub ice_candidates_received: u32,
}

impl Serialize for ConnectionStats {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ConnectionStats", 3)?;
        state.serialize_field("connection_state", &self.connection_state)?;
        state.serialize_field("last_activity_secs", &self.last_activity.elapsed().as_secs())?;
        state.serialize_field("ice_candidates_received", &self.ice_candidates_received)?;
        state.end()
    }
}

impl ConnectionStats {
    fn new() -> Self {
        Self {
            connection_state: "new".to_string(),
            last_activity: Instant::now(),
            ice_candidates_received: 0,
        }
    }
}

#[derive(Debug)]
pub struct ConnectionMonitor {
    connections: Arc<Mutex<HashMap<String, ConnectionStats>>>,
}

impl ConnectionMonitor {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn handle_disconnect(&self, peer_id: &str, room_id: &str) {
        let mut connections = self.connections.lock().await;
        connections.remove(peer_id);
        info!("Removed connection state tracking for peer {} in room {}", peer_id, room_id);
    }

    pub async fn update_connection_state(&self, peer_id: &str, state: &str) {
        let mut connections = self.connections.lock().await;
        let stats = connections.entry(peer_id.to_string()).or_insert_with(ConnectionStats::new);
        stats.connection_state = state.to_string();
        stats.last_activity = Instant::now();
    }

    pub async fn record_ice_candidate(&self, peer_id: &str, _received: bool) {
        let mut connections = self.connections.lock().await;
        if let Some(stats) = connections.get_mut(peer_id) {
            stats.ice_candidates_received += 1;
            stats.last_activity = Instant::now();
        }
    }

    pub async fn get_connection_stats(&self) -> HashMap<String, ConnectionStats> {
        self.connections.lock().await.clone()
    }

    pub async fn register_connection(&self, peer_id: &str) {
        let mut connections = self.connections.lock().await;
        connections.insert(peer_id.to_string(), ConnectionStats::new());
        debug!("Registered new connection for peer: {}", peer_id);
    }

    pub async fn get_metrics(&self) -> ConnectionMetrics {
        let connections = self.connections.lock().await;
        
        let total = connections.len();
        let active = connections.values()
            .filter(|stats| stats.connection_state == "connected")
            .count();
        let failed = connections.values()
            .filter(|stats| stats.connection_state == "failed")
            .count();
            
        let success_rate = if total > 0 {
            (active as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        
        let state_distribution = connections.values()
            .fold(HashMap::new(), |mut acc, stats| {
                *acc.entry(stats.connection_state.clone()).or_insert(0) += 1;
                acc
            });
            
        ConnectionMetrics {
            total_connections: total,
            active_connections: active,
            failed_connections: failed,
            success_rate,
            average_connection_time: None, // TODO: Implement if needed
            state_distribution,
        }
    }

    pub async fn check_for_alerts(&self) -> Vec<Alert> {
        let connections = self.connections.lock().await;
        let mut alerts = Vec::new();
        
        // Check for high failure rate
        let failed = connections.values()
            .filter(|stats| stats.connection_state == "failed")
            .count();
        let total = connections.len();
        
        if total > 0 {
            let failure_rate = (failed as f64 / total as f64) * 100.0;
            if failure_rate > 20.0 {
                alerts.push(Alert {
                    severity: "high".to_string(),
                    message: format!("High connection failure rate: {:.1}%", failure_rate),
                    timestamp: chrono::Utc::now(),
                });
            }
        }
        
        alerts
    }
}

#[derive(Clone, Serialize)]
pub struct StateChangeEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub peer_id: String,
    pub from_state: Option<String>,
    pub to_state: String,
}

pub struct StateChangeBroadcaster {
    sender: broadcast::Sender<StateChangeEvent>,
}

impl StateChangeBroadcaster {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(100);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StateChangeEvent> {
        self.sender.subscribe()
    }

    pub fn broadcast(&self, event: StateChangeEvent) {
        if let Err(e) = self.sender.send(event) {
            warn!("Failed to broadcast state change: {}", e);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectionMetrics {
    pub total_connections: usize,
    pub active_connections: usize,
    pub failed_connections: usize,
    pub success_rate: f64,
    pub average_connection_time: Option<f64>,
    pub state_distribution: HashMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct Alert {
    pub severity: String,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
} 