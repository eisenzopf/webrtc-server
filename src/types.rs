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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum MediaType {
    Audio,
    Video,
    Screen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "message_type")]
pub enum SignalingMessage {
    Join {
        room_id: String,
        peer_id: String,
    },
    Disconnect {
        room_id: String,
        peer_id: String,
    },
    PeerList {
        peers: Vec<String>,
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
    RequestPeerList,
    InitiateCall {
        peer_id: String,
        room_id: String,
    },
    MediaError {
        error_type: String,
        description: String,
        peer_id: String,
    }
}

pub type PeerConnection = (String, Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>);
pub type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

impl Default for SignalingMessage {
    fn default() -> Self {
        SignalingMessage::PeerList { 
            peers: Vec::new() 
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

#[derive(Debug, Clone)]
pub struct Room {
    pub id: String,
    pub peers: Vec<PeerConnection>,
    pub media_settings: MediaSettings,
    pub media_relays: HashMap<String, MediaRelay>,
    pub recording_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallMetadata {
    pub room_id: String,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
    pub participants: Vec<String>,
    pub recording_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaSettings {
    pub max_participants: usize,
    pub allowed_media_types: Vec<MediaType>,
    pub bandwidth_limit: Option<u32>,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            max_participants: 10,
            allowed_media_types: vec![MediaType::Audio],
            bandwidth_limit: None,
        }
    }
}

impl Default for Room {
    fn default() -> Self {
        Self {
            id: String::new(),
            peers: Vec::new(),
            media_settings: MediaSettings::default(),
            media_relays: HashMap::new(),
            recording_enabled: false,
        }
    }
}