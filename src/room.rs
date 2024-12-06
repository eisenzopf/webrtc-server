use crate::types::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Room {
    pub id: String,
    pub peers: Vec<PeerConnection>,
    pub media_settings: MediaSettings,
    pub media_relays: HashMap<String, MediaRelay>,
    pub recording_enabled: bool,
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