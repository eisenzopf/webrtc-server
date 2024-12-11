use crate::signaling::PeerConnection;
use crate::media::MediaRelay;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use crate::utils::Error;

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
    pub peers: Vec<(String, MediaRelay)>,
    pub media_settings: MediaSettings,
    pub recording_enabled: bool,
}

impl Room {
    pub fn add_peer(&mut self, peer_id: String, relay: MediaRelay) -> Result<(), Error> {
        if self.peers.len() >= self.media_settings.max_participants {
            return Err(Error::Room("Room is full".to_string()));
        }
        self.peers.push((peer_id, relay));
        Ok(())
    }

    pub fn get_peer_relay(&self, peer_id: &str) -> Option<&MediaRelay> {
        self.peers.iter()
            .find(|(id, _)| id == peer_id)
            .map(|(_, relay)| relay)
    }

    pub fn remove_peer(&mut self, peer_id: &str) {
        self.peers.retain(|(id, _)| id != peer_id);
    }

    pub async fn broadcast_track(&self, from_peer: &str, track: Arc<TrackLocalStaticRTP>) -> Result<(), Error> {
        for (peer_id, relay) in &self.peers {
            if peer_id != from_peer {
                relay.peer_connection.add_track(track.clone()).await?;
            }
        }
        Ok(())
    }

    pub fn has_peer(&self, peer_id: &str) -> bool {
        self.peers.iter().any(|(id, _)| id == peer_id)
    }
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
            recording_enabled: false,
        }
    }
} 