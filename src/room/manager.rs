use super::state::Room;
use crate::utils::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::signaling::PeerConnection;

pub struct RoomManager {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
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
        peer_connection: crate::signaling::PeerConnection,
    ) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        let room = rooms
            .get_mut(room_id)
            .ok_or_else(|| Error::Room(format!("Room {} not found", room_id)))?;

        if room.peers.len() >= room.media_settings.max_participants {
            return Err(Error::Room("Room is full".to_string()));
        }

        room.peers.push(peer_connection);
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