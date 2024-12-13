use super::state::Room;
use crate::utils::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::signaling::PeerConnection;
use crate::media::MediaRelay;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;

pub struct RoomManager {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    stun_server: String,
    stun_port: u16,
    turn_server: String,
    turn_port: u16,
    turn_username: String,
    turn_password: String,
}

impl RoomManager {
    pub fn new(
        stun_server: String,
        stun_port: u16,
        turn_server: String,
        turn_port: u16,
        turn_username: String,
        turn_password: String,
    ) -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            stun_server,
            stun_port,
            turn_server,
            turn_port,
            turn_username,
            turn_password,
        }
    }

    pub async fn create_room(&self, room_id: String) -> Result<Room> {
        let mut rooms = self.rooms.write().await;
        if rooms.contains_key(&room_id) {
            return Err(Error::Room(format!("Room {} already exists", room_id)));
        }

        let room = Room {
            id: room_id.clone(),
            ..Room::default()
        };
        rooms.insert(room_id, room.clone());
        Ok(room)
    }

    pub async fn get_room(&self, room_id: &str) -> Result<Room> {
        let rooms = self.rooms.read().await;
        rooms
            .get(room_id)
            .cloned()
            .ok_or_else(|| Error::Room(format!("Room {} not found", room_id)))
    }

    pub async fn remove_room(&self, room_id: &str) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        rooms
            .remove(room_id)
            .ok_or_else(|| Error::Room(format!("Room {} not found", room_id)))?;
        Ok(())
    }

    pub async fn add_peer_to_room(
        &self,
        room_id: &str,
        peer_id: String,
        peer_connection: PeerConnection,
    ) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| Error::Room(format!("Room {} not found", room_id)))?;

        if room.peers.len() >= room.media_settings.max_participants {
            return Err(Error::Room("Room is full".to_string()));
        }

        // Create API and configuration for the peer connection
        let mut media_engine = webrtc::api::media_engine::MediaEngine::default();
        media_engine.register_default_codecs()?;

        let api = webrtc::api::APIBuilder::new()
            .with_media_engine(media_engine)
            .build();

        let config = webrtc::peer_connection::configuration::RTCConfiguration {
            ice_servers: vec![
                webrtc::ice_transport::ice_server::RTCIceServer {
                    urls: vec![
                        format!("stun:{}:{}", self.stun_server, self.stun_port),
                        format!("turn:{}:{}", self.turn_server, self.turn_port)
                    ],
                    username: self.turn_username.clone(),
                    credential: self.turn_password.clone(),
                    credential_type: RTCIceCredentialType::Password,
                    ..Default::default()
                },
            ],
            ice_transport_policy: RTCIceTransportPolicy::All,
            ..Default::default()
        };

        // Create the peer connection
        let peer_connection = Arc::new(api.new_peer_connection(config).await?);

        let media_relay = MediaRelay::new(peer_connection, peer_id.clone());

        room.peers.push((peer_id, media_relay));
        Ok(())
    }

    pub async fn remove_peer_from_room(&self, room_id: &str, peer_id: &str) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| Error::Room(format!("Room {} not found", room_id)))?;

        room.peers.retain(|(id, _)| id != peer_id);
        Ok(())
    }
} 