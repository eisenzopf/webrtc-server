// src/types.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;
use futures_util::stream::SplitSink;
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum MediaType {
    Audio,
    Video,
    Screen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalingMessage {
    pub message_type: String,
    pub room_id: Option<String>,
    pub peer_id: Option<String>,
    pub sdp: Option<String>,
    pub candidate: Option<String>,
    pub from_peer: Option<String>,
    pub to_peer: Option<String>,
    pub media_types: Option<Vec<MediaType>>,
    pub error_type: Option<String>,
    pub description: Option<String>,
    pub peers: Option<Vec<String>>,
}

pub type PeerConnection = (String, Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>);
pub type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

impl Default for SignalingMessage {
    fn default() -> Self {
        Self {
            message_type: String::new(),
            room_id: None,
            peer_id: None,
            sdp: None,
            candidate: None,
            from_peer: None,
            to_peer: None,
            media_types: None,
            error_type: None,
            description: None,
            peers: None,
        }
    }
}