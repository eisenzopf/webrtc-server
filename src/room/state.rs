use crate::signaling::PeerConnection;
use crate::media::MediaRelay;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

#[derive(Debug, Clone)]
pub struct MediaSettings {
    pub max_participants: usize,
    pub allowed_media_types: Vec<MediaType>,
    pub bandwidth_limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    Audio,
    Video,
    Screen,
}

#[derive(Debug, Clone)]
pub struct Room {
    pub id: String,
    pub peers: Vec<PeerConnection>,
    pub media_settings: MediaSettings,
    pub media_relays: HashMap<String, MediaRelay>,
    pub recording_enabled: bool,
    pub connected_pairs: HashSet<(String, String)>,
    pub peer_connections: HashMap<String, Arc<RTCPeerConnection>>,
    pub media_tracks: HashMap<String, Vec<Arc<TrackLocalStaticRTP>>>,
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
            connected_pairs: HashSet::new(),
            peer_connections: HashMap::new(),
            media_tracks: HashMap::new(),
        }
    }
} 