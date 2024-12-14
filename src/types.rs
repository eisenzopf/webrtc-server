// src/types.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;
use futures_util::stream::SplitSink;
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_remote::TrackRemote;
use std::time::SystemTime;
use crate::utils::{Error, Result};
use futures_util::SinkExt;
use warp::ws::WebSocket;

// Re-export room types
pub use crate::room::state::{Room, MediaSettings, MediaType};

// Define WebSocketSender type
pub type TungsteniteWebSocketSender = SplitSink<WebSocketStream<TcpStream>, Message>;
pub type WarpWebSocketSender = SplitSink<WebSocket, warp::ws::Message>;

// Create an enum to handle both types
#[derive(Debug, Clone)]
pub enum WebSocketSenderType {
    Tungstenite(Arc<Mutex<TungsteniteWebSocketSender>>),
    Warp(Arc<Mutex<WarpWebSocketSender>>),
}

#[derive(Debug, Clone)]
pub struct WebSocketConnection {
    sender: WebSocketSenderType,
}

impl WebSocketConnection {
    pub fn new_tungstenite(sender: Arc<Mutex<TungsteniteWebSocketSender>>) -> Self {
        Self {
            sender: WebSocketSenderType::Tungstenite(sender),
        }
    }

    pub fn new_warp(sender: Arc<Mutex<WarpWebSocketSender>>) -> Self {
        Self {
            sender: WebSocketSenderType::Warp(sender),
        }
    }

    pub async fn send(&self, text: String) -> Result<()> {
        match &self.sender {
            WebSocketSenderType::Tungstenite(sender) => {
                let mut sender = sender.lock().await;
                sender.send(Message::Text(text)).await.map_err(|e| Error::WebSocketError(e.to_string()))?;
            }
            WebSocketSenderType::Warp(sender) => {
                let mut sender = sender.lock().await;
                sender.send(warp::ws::Message::text(text)).await.map_err(|e| Error::WebSocketError(e.to_string()))?;
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "message_type")]
pub enum SignalingMessage {
    CallRequest {
        room_id: String,
        from_peer: String,
        to_peers: Vec<String>,
        sdp: String,
    },
    CallResponse {
        room_id: String,
        from_peer: String,
        to_peer: String,
        accepted: bool,
        reason: Option<String>,
    },
    Join {
        room_id: String,
        peer_id: String,
    },
    RequestPeerList {
        room_id: String,
    },
    PeerList {
        room_id: String,
        peers: Vec<String>,
    },
    Disconnect {
        room_id: String,
        peer_id: String,
    },
    Offer {
        room_id: String,
        sdp: String,
        from_peer: String,
        to_peer: String,
    },
    Answer {
        room_id: String,
        sdp: String,
        from_peer: String,
        to_peer: String,
    },
    IceCandidate {
        room_id: String,
        candidate: String,
        from_peer: String,
        to_peer: String,
    },
    MediaError {
        error_type: String,
        description: String,
        peer_id: String,
    },
    EndCall {
        room_id: String,
        peer_id: String,
    },
    PeerDisconnected {
        room_id: String,
        peer_id: String,
    },
}

pub type PeerConnection = (String, Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>);
pub type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

impl Default for SignalingMessage {
    fn default() -> Self {
        SignalingMessage::PeerList { 
            peers: Vec::new(), 
            room_id: String::new() 
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaRelay {
    pub peer_connection: Arc<RTCPeerConnection>,
    pub audio_track: Option<Arc<TrackLocalStaticRTP>>,
    pub video_track: Option<Arc<TrackLocalStaticRTP>>,
    pub peer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallMetadata {
    pub room_id: String,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
    pub participants: Vec<String>,
    pub recording_path: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct TurnCredentials {
    pub stun_server: String,
    pub stun_port: u16,
    pub turn_server: String,
    pub turn_port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub stun_server: String,
    pub stun_port: u16,
    pub turn_server: String,
    pub turn_port: u16,
    pub turn_username: String,
    pub turn_password: String,
    pub ws_port: u16,
}