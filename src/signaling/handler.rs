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
use log::{info, error};
use crate::media::relay::SignalingHandler;

pub type WebSocketSender = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    Message
>;
pub type PeerConnection = (String, Arc<Mutex<WebSocketSender>>);
pub type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

#[derive(Clone)]
pub struct MessageHandler {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    peers: Arc<RwLock<HashMap<String, Vec<(String, Arc<Mutex<WebSocketSender>>)>>>>,
    pub media_relay: Arc<MediaRelayManager>,
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
        // Create Arc of self (MessageHandler) instead of reference
        let handler = Arc::new(self.clone());
        
        let relay = self.media_relay.create_relay(
            peer_id.clone(),
            room_id.clone(),
            peer_id.clone(), // This might need to be adjusted depending on your peer-to-peer logic
            handler
        ).await?;
        
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

        let peer_ids: Vec<String> = room_peers.iter().map(|(id, _)| id.clone()).collect();
        let peer_list_msg = SignalingMessage::PeerList { 
            peers: peer_ids,
            room_id: room_id.clone(),
        };

        for (_, sender) in room_peers {
            let msg_str = serde_json::to_string(&peer_list_msg)?;
            let mut sender_guard = sender.lock().await;
            let _ = sender_guard.send(Message::Text(msg_str)).await;
        }

        Ok(())
    }

    async fn handle_offer(&self, room_id: &str, from_peer: &str, to_peer: &str, sdp: &str) -> Result<()> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            if let Some(relay) = room.get_peer_relay(to_peer) {
                // Set the remote description on the relay's peer connection
                let desc = RTCSessionDescription::offer(sdp.to_string())?;
                relay.peer_connection.set_remote_description(desc).await?;

                // Create answer with proper configuration
                let answer = relay.peer_connection.create_answer(None).await?;
                
                // Important: Set local description before sending answer
                relay.peer_connection.set_local_description(answer.clone()).await?;

                // Wait for ICE gathering to complete
                let mut gathering_complete = relay.peer_connection.gathering_complete_promise().await;
                let _ = gathering_complete.recv().await;

                // Get the complete local description with ICE candidates
                let local_desc = relay.peer_connection.local_description().await
                    .ok_or_else(|| Error::Peer("No local description available".to_string()))?;

                // Send answer back through signaling
                let answer_msg = SignalingMessage::Answer {
                    room_id: room_id.to_string(),
                    sdp: local_desc.sdp,
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

    async fn handle_call_request(
        &self,
        room_id: String,
        from_peer: String,
        to_peers: Vec<String>
    ) -> Result<()> {
        info!("Handling call request from {} to {:?}", from_peer, to_peers);
        
        // Get the caller's relay
        let caller_relay = self.get_peer_relay(&from_peer).await?
            .ok_or_else(|| Error::Peer(format!("Caller {} not found", from_peer)))?;

        // Create offer on the caller's relay
        let offer = caller_relay.peer_connection.create_offer(None).await?;
        caller_relay.peer_connection.set_local_description(offer.clone()).await?;

        // Send offer to each target peer
        for peer_id in to_peers {
            let offer_msg = SignalingMessage::Offer {
                room_id: room_id.clone(),
                sdp: offer.sdp.clone(),
                from_peer: from_peer.clone(),
                to_peer: peer_id.clone(),
            };
            self.send_to_peer(&peer_id, &offer_msg).await?;
        }

        Ok(())
    }

    pub async fn handle_message(&self, message: SignalingMessage) -> Result<()> {
        match message {
            SignalingMessage::EndCall { room_id, peer_id } => {
                info!("Handling EndCall from peer {} in room {}", peer_id, room_id);
                self.media_relay.handle_peer_disconnect(&peer_id, &room_id).await?;
                Ok(())
            },
            SignalingMessage::PeerDisconnected { room_id, peer_id } => {
                info!("Handling peer disconnection for {} in room {}", peer_id, room_id);
                self.media_relay.handle_peer_disconnect(&peer_id, &room_id).await?;
                Ok(())
            },
            SignalingMessage::Join { room_id, peer_id, sender } => {
                if let Some(sender) = sender {
                    self.handle_join(room_id, peer_id, sender).await
                } else {
                    Err(Error::Peer("No WebSocket sender provided for Join message".to_string()))
                }
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
            SignalingMessage::CallRequest { room_id, from_peer, to_peers } => {
                self.handle_call_request(room_id, from_peer, to_peers).await
            }
            _ => Ok(()),
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
        let peer_id_ice = peer_id.to_string();

        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            let peer_id = peer_id_state.clone();
            Box::pin(async move {
                info!(
                    "Peer {} connection state changed to {:?}",
                    peer_id, s
                );
                match s {
                    RTCPeerConnectionState::Connected => {
                        info!("Peer {} successfully connected", peer_id);
                    }
                    RTCPeerConnectionState::Failed => {
                        error!("Peer {} connection failed", peer_id);
                    }
                    _ => {}
                }
            })
        }));

        pc.on_ice_candidate(Box::new(move |c| {
            let peer_id = peer_id_ice.clone();
            Box::pin(async move {
                if let Some(c) = c {
                    info!(
                        "Peer {} generated ICE candidate: {:?}",
                        peer_id, c
                    );
                }
            })
        }));
    }
}

impl SignalingHandler for MessageHandler {
    async fn send_to_peer(&self, peer_id: &str, msg: &SignalingMessage) -> Result<()> {
        self.send_to_peer(peer_id, msg).await
    }
} 