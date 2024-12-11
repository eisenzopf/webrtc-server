use crate::utils::{Error, Result};
use crate::room::Room;
use crate::metrics::ConnectionMetrics;
use crate::signaling::messages::SignalingMessage;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use std::collections::HashMap;
use tokio_tungstenite::tungstenite::Message;
use futures_util::SinkExt;
use webrtc::{
    api::{
        media_engine::MediaEngine,
        APIBuilder,
    },
    peer_connection::{
        configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        RTCPeerConnection,
        sdp::session_description::RTCSessionDescription,
    },
    ice_transport::{
        ice_server::RTCIceServer,
        ice_connection_state::RTCIceConnectionState,
    },
    rtp_transceiver::{
        rtp_codec::RTCRtpCodecCapability,
        rtp_receiver::RTCRtpReceiver,
        RTCRtpTransceiver,
    },
    track::{
        track_remote::TrackRemote,
        track_local::track_local_static_rtp::TrackLocalStaticRTP,
    },
};
use crate::media::{MediaRelayManager, MediaRelay};
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::track::track_local::TrackLocal;
use log::info;

pub type WebSocketSender = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    Message
>;
pub type PeerConnection = (String, Arc<Mutex<WebSocketSender>>);
pub type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

pub struct MessageHandler {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    peers: Arc<RwLock<HashMap<String, Vec<(String, Arc<Mutex<WebSocketSender>>)>>>>,
    media_relay: Arc<MediaRelayManager>,
}

impl MessageHandler {
    pub fn new(media_relay: Arc<MediaRelayManager>) -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            peers: Arc::new(RwLock::new(HashMap::new())),
            media_relay,
        }
    }

    async fn handle_join(
        &self,
        room_id: String,
        peer_id: String,
        ws_sender: Arc<Mutex<WebSocketSender>>
    ) -> Result<()> {
        // Create media relay for the new peer
        let relay = self.media_relay.create_relay(peer_id.clone()).await?;
        
        // Add peer to room with the relay
        let mut rooms = self.rooms.write().await;
        let room = rooms.entry(room_id.clone())
            .or_insert_with(Room::default);
        
        room.add_peer(peer_id.clone(), relay)?;

        // Store WebSocket sender
        let mut peers = self.peers.write().await;
        let room_peers = peers.entry(room_id.clone())
            .or_insert_with(Vec::new);
        room_peers.push((peer_id.clone(), ws_sender));

        Ok(())
    }

    async fn handle_offer(&self, room_id: &str, from_peer: &str, to_peer: &str, sdp: &str) -> Result<()> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            // Get the media relay for the receiving peer
            if let Some(relay) = room.get_peer_relay(to_peer) {
                // Set the remote description on the relay's peer connection
                let desc = RTCSessionDescription::offer(sdp.to_string())?;
                relay.peer_connection.set_remote_description(desc).await?;

                // Create answer
                let answer = relay.peer_connection.create_answer(None).await?;
                relay.peer_connection.set_local_description(answer.clone()).await?;

                // Send answer back through signaling
                let answer_msg = SignalingMessage::Answer {
                    room_id: room_id.to_string(),
                    sdp: answer.sdp,
                    from_peer: to_peer.to_string(),
                    to_peer: from_peer.to_string(),
                };
                self.send_to_peer(from_peer, &answer_msg).await?;
            }
        }
        Ok(())
    }

    async fn handle_ice_candidate(&self, room_id: &str, from_peer: &str, to_peer: &str, candidate: &RTCIceCandidateInit) -> Result<()> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            if let Some(relay) = room.get_peer_relay(to_peer) {
                // Add ICE candidate to the relay's peer connection
                relay.peer_connection.add_ice_candidate(candidate.clone()).await?;
            }
        }
        Ok(())
    }

    pub async fn handle_message(
        &self,
        msg: SignalingMessage,
        sender: Arc<Mutex<WebSocketSender>>
    ) -> Result<()> {
        match msg {
            SignalingMessage::Join { room_id, peer_id } => {
                self.handle_join(room_id, peer_id, sender).await
            }
            SignalingMessage::Offer { room_id, sdp, from_peer, to_peer } => {
                self.handle_offer(&room_id, &from_peer, &to_peer, &sdp).await
            }
            SignalingMessage::Answer { room_id, sdp, from_peer, to_peer } => {
                let rooms = self.rooms.read().await;
                if let Some(room) = rooms.get(&room_id) {
                    if let Some(relay) = room.get_peer_relay(&to_peer) {
                        let desc = RTCSessionDescription::answer(sdp)?;
                        relay.peer_connection.set_remote_description(desc).await?;
                    }
                }
                Ok(())
            }
            SignalingMessage::IceCandidate { room_id, candidate, from_peer, to_peer } => {
                let candidate_init = serde_json::from_str::<RTCIceCandidateInit>(&candidate)?;
                self.handle_ice_candidate(&room_id, &from_peer, &to_peer, &candidate_init).await
            }
            SignalingMessage::Disconnect { room_id, peer_id } => {
                self.media_relay.remove_relay(&peer_id).await?;
                
                let mut rooms = self.rooms.write().await;
                if let Some(room) = rooms.get_mut(&room_id) {
                    room.remove_peer(&peer_id);
                }

                let mut peers = self.peers.write().await;
                if let Some(room_peers) = peers.get_mut(&room_id) {
                    room_peers.retain(|(id, _)| id != &peer_id);
                }
                Ok(())
            }
            SignalingMessage::PeerList { .. } => {
                Ok(())
            }
            SignalingMessage::RequestPeerList => {
                Ok(())
            }
            SignalingMessage::InitiateCall { .. } => {
                Ok(())
            }
            SignalingMessage::MediaError { .. } => {
                Ok(())
            }
            SignalingMessage::EndCall { .. } => {
                Ok(())
            }
            _ => Ok(())
        }
    }

    async fn send_to_peer(&self, peer_id: &str, msg: &SignalingMessage) -> Result<()> {
        let peers = self.peers.read().await;
        for (_, room_peers) in peers.iter() {
            for (id, sender) in room_peers {
                if id == peer_id {
                    let mut sender = sender.lock().await;
                    let msg_str = serde_json::to_string(&msg)?;
                    sender.send(Message::Text(msg_str)).await?;
                    return Ok(());
                }
            }
        }
        Err(Error::Peer(format!("Peer {} not found", peer_id)))
    }

    async fn broadcast_to_room(&self, room_id: &str, msg: &SignalingMessage, exclude_peer: Option<&str>) -> Result<()> {
        let peers = self.peers.read().await;
        if let Some(room_peers) = peers.get(room_id) {
            for (peer_id, sender) in room_peers {
                if let Some(excluded) = exclude_peer {
                    if peer_id == excluded {
                        continue;
                    }
                }
                let mut sender = sender.lock().await;
                let msg_str = serde_json::to_string(&msg)?;
                sender.send(Message::Text(msg_str)).await?;
            }
        }
        Ok(())
    }

    pub async fn get_peer_relay(&self, peer_id: &str) -> Result<Option<MediaRelay>> {
        let rooms = self.rooms.read().await;
        for room in rooms.values() {
            if let Some(relay) = room.get_peer_relay(peer_id) {
                return Ok(Some(relay.clone()));
            }
        }
        Ok(None)
    }

    pub async fn broadcast_track(&self, from_peer: &str, track: Arc<TrackLocalStaticRTP>) -> Result<()> {
        let rooms = self.rooms.read().await;
        for room in rooms.values() {
            if room.has_peer(from_peer) {
                room.broadcast_track(from_peer, track.clone()).await?;
                break;
            }
        }
        Ok(())
    }

    async fn monitor_connection_state(&self, peer_id: &str, relay: &MediaRelay) {
        let pc = relay.peer_connection.clone();
        let peer_id_state = peer_id.to_string();
        let peer_id_ice = peer_id.to_string(); // Create separate clone for ice handler

        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            let peer_id = peer_id_state.clone();
            Box::pin(async move {
                info!(
                    "Peer {} connection state changed to {:?}",
                    peer_id, s
                );
            })
        }));

        pc.on_ice_connection_state_change(Box::new(move |s: RTCIceConnectionState| {
            let peer_id = peer_id_ice.clone();
            Box::pin(async move {
                info!(
                    "Peer {} ICE connection state changed to {:?}",
                    peer_id, s
                );
            })
        }));
    }
} 