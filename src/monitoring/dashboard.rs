use serde::Serialize;
use crate::signaling::connection_state::{ConnectionState, ConnectionStateManager};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast;
use log::warn;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Instant;
use log::debug;
use super::ConnectionStats;

#[derive(Debug, Serialize)]
pub struct ConnectionMetrics {
    pub total_connections: usize,
    pub active_connections: usize,
    pub failed_connections: usize,
    pub success_rate: f64,
    pub average_connection_time: Option<Duration>,
    pub state_distribution: HashMap<ConnectionState, usize>,
}

impl ConnectionMetrics {
    pub async fn collect(state_manager: &ConnectionStateManager) -> Self {
        let states = state_manager.get_all_states().await;
        
        let total = states.len();
        let active = states.values()
            .filter(|&state| matches!(state, ConnectionState::Connected))
            .count();
        let failed = states.values()
            .filter(|&state| matches!(state, ConnectionState::Failed))
            .count();
            
        let success_rate = if total == 0 {
            0.0
        } else {
            (active as f64 / total as f64) * 100.0
        };
        
        let state_distribution = states.into_iter()
            .fold(HashMap::new(), |mut acc, (_, state)| {
                *acc.entry(state).or_insert(0) += 1;
                acc
            });
            
        ConnectionMetrics {
            total_connections: total,
            active_connections: active,
            failed_connections: failed,
            success_rate,
            average_connection_time: None, // Implement if you're tracking connection times
            state_distribution,
        }
    }
}

pub struct StateChangeBroadcaster {
    sender: broadcast::Sender<StateChangeEvent>,
}

#[derive(Clone, Serialize)]
pub struct StateChangeEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub peer_id: String,
    pub from_state: Option<ConnectionState>,
    pub to_state: ConnectionState,
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

pub async fn check_for_alerts(state_manager: &ConnectionStateManager) -> Vec<Alert> {
    // Implementation for alert checking
    Vec::new()
}

#[derive(Debug, Serialize)]
pub struct Alert {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub peer_id: String,
    pub alert_type: String,
    pub message: String,
    pub severity: AlertSeverity,
}

#[derive(Debug, Serialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug)]
pub struct ConnectionMonitor {
    connections: Arc<Mutex<HashMap<String, ConnectionStats>>>,
}

impl ConnectionMonitor {
    pub fn new() -> Self {
        ConnectionMonitor {
            connections: Arc::new(Mutex::new(HashMap::new()))
        }
    }

    pub async fn register_connection(&self, peer_id: &str) {
        let mut connections = self.connections.lock().await;
        connections.insert(peer_id.to_string(), ConnectionStats {
            peer_id: peer_id.to_string(),
            connection_state: "new".to_string(),
            ice_connection_state: "new".to_string(),
            connected_at: None,
            last_activity: Instant::now(),
            ice_candidates_received: 0,
            ice_candidates_sent: 0,
            data_channels_open: 0,
            error_count: 0,
        });
        debug!("Registered new connection for peer: {}", peer_id);
    }

    pub async fn check_connections(&self) {
        let mut connections = self.connections.lock().await;
        let stale_timeout = Duration::from_secs(60); // Configure timeout period
        let now = Instant::now();

        // Collect stale connections
        let stale_peers: Vec<String> = connections
            .iter()
            .filter(|(_, stats)| now.duration_since(stats.last_activity) > stale_timeout)
            .map(|(peer_id, _)| peer_id.clone())
            .collect();

        // Remove stale connections
        for peer_id in stale_peers {
            debug!("Removing stale connection for peer: {}", peer_id);
            connections.remove(&peer_id);
        }
    }

    pub async fn update_connection_state(&self, peer_id: &str, state: &str) {
        if let Some(stats) = self.connections.lock().await.get_mut(peer_id) {
            stats.connection_state = state.to_string();
            stats.last_activity = Instant::now();
            debug!("Updated connection state for peer {}: {}", peer_id, state);
        }
    }

    pub async fn record_ice_candidate(&self, peer_id: &str, received: bool) {
        if let Some(stats) = self.connections.lock().await.get_mut(peer_id) {
            if received {
                stats.ice_candidates_received += 1;
            } else {
                stats.ice_candidates_sent += 1;
            }
            stats.last_activity = Instant::now();
        }
    }

    pub async fn get_connection_stats(&self) -> Vec<ConnectionStats> {
        self.connections.lock().await
            .values()
            .cloned()
            .collect()
    }
} 