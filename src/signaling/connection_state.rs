use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Serialize, Deserialize};
use log::{debug, info, warn};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnectionState {
    New,
    Joining,
    WaitingForOffer,
    OfferReceived,
    AnswerCreated,
    Connected,
    Failed,
    Closed,
}

#[derive(Debug, Serialize)]
pub struct StateTransition {
    pub timestamp: DateTime<Utc>,
    pub from_state: Option<ConnectionState>,
    pub to_state: ConnectionState,
}

#[derive(Debug)]
pub struct ConnectionStateManager {
    states: Arc<Mutex<HashMap<String, ConnectionState>>>,
    transition_log: Arc<Mutex<HashMap<String, Vec<StateTransition>>>>,
    broadcaster: broadcast::Sender<StateChangeEvent>,
}

#[derive(Debug, Clone)]
pub struct StateChangeEvent {
    pub timestamp: DateTime<Utc>,
    pub peer_id: String,
    pub from_state: Option<ConnectionState>,
    pub to_state: ConnectionState,
}

impl ConnectionStateManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
            transition_log: Arc::new(Mutex::new(HashMap::new())),
            broadcaster: tx,
        }
    }

    pub async fn transition(&self, peer_id: &str, new_state: ConnectionState) -> bool {
        let mut states = self.states.lock().await;
        let current_state = states.get(peer_id).cloned();

        let valid_transition = match (&current_state, &new_state) {
            (None, ConnectionState::New) => true,
            (Some(ConnectionState::New), ConnectionState::Joining) => true,
            (Some(ConnectionState::Joining), ConnectionState::WaitingForOffer) => true,
            (Some(ConnectionState::WaitingForOffer), ConnectionState::OfferReceived) => true,
            (Some(ConnectionState::OfferReceived), ConnectionState::AnswerCreated) => true,
            (Some(ConnectionState::AnswerCreated), ConnectionState::Connected) => true,
            (Some(_), ConnectionState::Failed) => true,
            (Some(_), ConnectionState::Closed) => true,
            _ => false,
        };

        if valid_transition {
            let _ = self.broadcaster.send(StateChangeEvent {
                timestamp: Utc::now(),
                peer_id: peer_id.to_string(),
                from_state: current_state.clone(),
                to_state: new_state.clone(),
            });

            states.insert(peer_id.to_string(), new_state);
            true
        } else {
            false
        }
    }

    pub async fn get_state(&self, peer_id: &str) -> Option<ConnectionState> {
        let states = self.states.lock().await;
        states.get(peer_id).cloned()
    }

    pub async fn get_all_states(&self) -> HashMap<String, ConnectionState> {
        let states = self.states.lock().await;
        states.clone()
    }
} 