use crate::types::*;

#[derive(Clone, Debug)]
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
            bandwidth_limit: Some(128),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Room {
    pub id: String,
    pub peers: Vec<PeerConnection>,
    pub media_settings: MediaSettings,
}