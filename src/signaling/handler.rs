use crate::utils::{Error, Result};
use crate::room::Room;
use crate::metrics::ConnectionMetrics;
use crate::types::{SignalingMessage, WebSocketConnection};
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
        ice_candidate::{RTCIceCandidate, RTCIceCandidateInit},
    },
};
use crate::media::{MediaRelayManager, MediaRelay};
use log::{info, error, debug, warn};
use serde::{Serialize, Deserialize};

pub struct MessageHandler {
    relay_manager: Arc<MediaRelayManager>,
    websocket_senders: Arc<RwLock<HashMap<String, WebSocketConnection>>>,
}

impl MessageHandler {
    pub fn new(relay_manager: Arc<MediaRelayManager>) -> Self {
        Self {
            relay_manager,
            websocket_senders: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn handle_offer(
        &self,
        room_id: String,
        from_peer: String,
        to_peer: String,
        sdp: String,
    ) -> Result<()> {
        let relays = self.relay_manager.get_relays().await?;
        
        if let Some(relay) = relays.get(&to_peer) {
            debug!("Setting remote description for peer {}", to_peer);
            
            let offer = RTCSessionDescription::offer(sdp)?;
            relay.peer_connection.set_remote_description(offer).await?;
            
            debug!("Creating answer for peer {}", to_peer);
            let answer = relay.peer_connection.create_answer(None).await?;
            
            debug!("Setting local description for peer {}", to_peer);
            relay.peer_connection.set_local_description(answer.clone()).await?;
            
            let answer_msg = SignalingMessage::Answer {
                room_id,
                sdp: answer.sdp,
                from_peer: to_peer,
                to_peer: from_peer.clone(),
            };
            
            self.send_message(&answer_msg).await?;
            debug!("Sent answer to peer {}", from_peer);
        }

        Ok(())
    }

    pub async fn handle_ice_candidate(
        &self,
        room_id: String,
        from_peer: String,
        to_peer: String,
        candidate: String,
    ) -> Result<()> {
        let relays = self.relay_manager.get_relays().await?;
        
        // Parse the candidate string to remove extra encoding
        let parsed_candidate = if candidate.starts_with('"') {
            serde_json::from_str::<String>(&candidate)?
        } else {
            candidate.clone()
        };
        
        // Forward the ICE candidate to the remote peer first
        if let Some(sender) = self.get_websocket_sender(&to_peer).await? {
            let message = SignalingMessage::IceCandidate {
                room_id: room_id.clone(),
                from_peer: from_peer.clone(),
                to_peer: to_peer.clone(),
                candidate: parsed_candidate.clone(),
            };
            sender.send(serde_json::to_string(&message)?).await?;
        }

        // Then handle it for the local peer connection
        if let Some(relay) = relays.get(&to_peer) {
            match serde_json::from_str::<RTCIceCandidateInit>(&parsed_candidate) {
                Ok(ice_candidate) => {
                    // Only try to add the candidate if we have a remote description
                    if relay.peer_connection.remote_description().await.is_some() {
                        match relay.peer_connection.add_ice_candidate(ice_candidate).await {
                            Ok(_) => {
                                debug!("Successfully added ICE candidate for peer {}", to_peer);
                            }
                            Err(e) => {
                                warn!("Could not add ICE candidate for {}: {}", to_peer, e);
                            }
                        }
                    } else {
                        debug!("Buffering ICE candidate for peer {} until remote description is set", to_peer);
                        // Here you might want to implement a buffer mechanism in the MediaRelay struct
                    }
                }
                Err(e) => {
                    warn!("Failed to parse ICE candidate: {}", e);
                }
            }
        }

        Ok(())
    }

    pub async fn send_message(&self, msg: &SignalingMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        let senders = self.websocket_senders.read().await;
        
        for ws_conn in senders.values() {
            if let Err(e) = ws_conn.send(json.clone()).await {
                warn!("Failed to send message to peer: {}", e);
            }
        }
        Ok(())
    }

    pub async fn remove_websocket_sender(&self, peer_id: &str) -> Result<()> {
        let mut senders = self.websocket_senders.write().await;
        senders.remove(peer_id);
        Ok(())
    }

    pub async fn handle_message(&self, msg: SignalingMessage, peer_id: &str) -> Result<()> {
        match msg {
            SignalingMessage::CallRequest { room_id, from_peer, to_peers, sdp } => {
                debug!("Handling call request from {} to {:?}", from_peer, to_peers);
                // Forward the call request to each target peer
                for to_peer in to_peers {
                    if let Some(ws_sender) = self.websocket_senders.read().await.get(&to_peer) {
                        let message = SignalingMessage::CallRequest {
                            room_id: room_id.clone(),
                            from_peer: from_peer.clone(),
                            to_peers: vec![to_peer.clone()],
                            sdp: sdp.clone(),
                        };
                        ws_sender.send(serde_json::to_string(&message)?).await?;
                        debug!("Forwarded call request to {}", to_peer);
                    }
                }
                Ok(())
            },
            SignalingMessage::CallResponse { room_id, from_peer, to_peer, accepted, reason, sdp } => {
                debug!("Handling call response from {} to {}: accepted={}", from_peer, to_peer, accepted);
                if let Some(ws_sender) = self.websocket_senders.read().await.get(&to_peer) {
                    let message = SignalingMessage::CallResponse {
                        room_id,
                        from_peer,
                        to_peer: to_peer.clone(),
                        accepted,
                        reason,
                        sdp,
                    };
                    ws_sender.send(serde_json::to_string(&message)?).await?;
                    debug!("Forwarded call response to {}", to_peer);
                }
                Ok(())
            },
            SignalingMessage::Join { room_id, peer_id } => {
                self.handle_join(room_id, peer_id).await
            },
            SignalingMessage::RequestPeerList { room_id } => {
                self.handle_peer_list_request(room_id).await
            },
            SignalingMessage::Offer { room_id, sdp, from_peer, to_peer } => {
                self.handle_offer(room_id, from_peer, to_peer, sdp).await
            },
            SignalingMessage::IceCandidate { room_id, candidate, from_peer, to_peer } => {
                self.handle_ice_candidate(room_id, from_peer, to_peer, candidate).await
            },
            _ => Ok(()),
        }
    }

    pub async fn handle_disconnect(&self, peer_id: &str, room_id: &str) -> Result<()> {
        // Implementation for disconnect handling
        Ok(())
    }

    pub async fn set_websocket_sender(&self, peer_id: String, ws_conn: WebSocketConnection) -> Result<()> {
        let mut senders = self.websocket_senders.write().await;
        senders.insert(peer_id, ws_conn);
        Ok(())
    }

    pub async fn handle_join(&self, room_id: String, peer_id: String) -> Result<()> {
        // First add the peer to the relay manager
        self.relay_manager.add_peer(&room_id, peer_id.clone()).await?;
        
        // Then get updated peer list
        let relays = self.relay_manager.get_relays().await?;
        let peer_ids: Vec<String> = relays.keys().cloned().collect();
        
        // Create peer list message
        let peer_list_msg = SignalingMessage::PeerList {
            room_id: room_id.clone(),
            peers: peer_ids,
        };
        
        // Broadcast to all connected peers
        self.broadcast_message(&peer_list_msg).await?;
        
        Ok(())
    }

    pub async fn handle_peer_list_request(&self, room_id: String) -> Result<()> {
        let relays = self.relay_manager.get_relays().await?;
        let peer_ids: Vec<String> = relays.keys().cloned().collect();
        
        let peer_list_msg = SignalingMessage::PeerList {
            room_id,
            peers: peer_ids,
        };
        
        self.send_message(&peer_list_msg).await?;
        Ok(())
    }

    pub async fn broadcast_message(&self, msg: &SignalingMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        let senders = self.websocket_senders.read().await;
        
        for ws_conn in senders.values() {
            if let Err(e) = ws_conn.send(json.clone()).await {
                warn!("Failed to send message to peer: {}", e);
            }
        }
        Ok(())
    }

    pub async fn get_websocket_sender(&self, peer_id: &str) -> Result<Option<WebSocketConnection>> {
        let senders = self.websocket_senders.read().await;
        Ok(senders.get(peer_id).cloned())
    }
} 