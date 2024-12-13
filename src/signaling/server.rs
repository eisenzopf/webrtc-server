use crate::utils::{Error, Result};
use crate::signaling::handler::MessageHandler;
use crate::types::{SignalingMessage, WebSocketSender};
use tokio::net::TcpListener;
use std::sync::Arc;
use tokio_tungstenite::accept_async;
use futures_util::{StreamExt, SinkExt};
use log::{info, warn, error, debug};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use serde_json::json;
use warp::{Filter, Reply};
use crate::media::MediaRelayManager;
use warp::reject;

pub struct SignalingServer {
    address: String,
    handler: Arc<MessageHandler>,
}

impl SignalingServer {
    pub fn new(handler: Arc<MessageHandler>) -> Self {
        info!("Creating new SignalingServer instance");
        Self {
            address: "0.0.0.0:8080".to_string(),
            handler,
        }
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting server on {}", self.address);
        let listener = TcpListener::bind(&self.address).await
            .map_err(|e| {
                error!("Failed to bind to address {}: {}", self.address, e);
                Error::IO(e)
            })?;

        info!("Server successfully bound to {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            info!("New connection from: {}", addr);
            let handler = self.handler.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler).await {
                    error!("Connection error from {}: {}", addr, e);
                }
            });
        }

        warn!("Server accept loop terminated");
        Ok(())
    }

    async fn handle_connection(
        stream: tokio::net::TcpStream,
        handler: Arc<MessageHandler>,
    ) -> Result<()> {
        let peer_addr = stream.peer_addr()?;
        let ws_stream = accept_async(stream).await?;
        
        info!("Successfully established WebSocket connection with {}", peer_addr);
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));
        
        let mut current_peer_id: Option<String> = None;
        let mut current_room_id: Option<String> = None;

        // Handle messages and disconnection
        let media_relay = handler.media_relay.clone();
        
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    if let Ok(msg_str) = msg.to_text() {
                        debug!("Received message from {}: {}", peer_addr, msg_str);
                        if let Ok(mut signal_msg) = serde_json::from_str::<SignalingMessage>(msg_str) {
                            // For Join messages, include the sender
                            if let SignalingMessage::Join { ref peer_id, ref room_id, ref mut sender } = signal_msg {
                                current_peer_id = Some(peer_id.clone());
                                current_room_id = Some(room_id.clone());
                                *sender = Some(ws_sender.clone());
                                
                                debug!("Join message received from peer {} for room {}", peer_id, room_id);
                            }
                            
                            if let Err(e) = handler.handle_message(signal_msg).await {
                                error!("Error handling message: {}", e);
                                // Don't break on message handling errors unless it's a critical failure
                                if e.to_string().contains("closed connection") {
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // Log the error but don't propagate WebSocket closure errors
                    if !e.to_string().contains("closed connection") {
                        error!("WebSocket error for peer {}: {}", peer_addr, e);
                    }
                    break;
                }
            }
        }

        // Handle normal closure
        if let (Some(peer_id), Some(room_id)) = (current_peer_id, current_room_id) {
            info!("WebSocket closed for peer {} in room {}", peer_id, room_id);
            // Ignore errors during disconnect handling since the connection is already closed
            let _ = handler.handle_peer_disconnect(&peer_id, &room_id).await;
        }

        Ok(())
    }
}

pub async fn run_debug_server(media_relay: Arc<MediaRelayManager>) {
    let media_stats = warp::path!("debug" / "media-stats")
        .and(warp::get())
        .and(with_media_relay(media_relay.clone()))
        .and_then(|relay: Arc<MediaRelayManager>| async move {
            match handle_media_stats_internal(relay).await {
                Ok(response) => Ok(warp::reply::json(&response)),
                Err(e) => Err(warp::reject::custom(ServerError(e.to_string())))
            }
        });

    let routes = media_stats;

    warp::serve(routes)
        .run(([0, 0, 0, 0], 8081))
        .await;
}

// Add this struct for custom error handling
#[derive(Debug)]
struct ServerError(String);

impl reject::Reject for ServerError {}

async fn handle_media_stats_internal(media_relay: Arc<MediaRelayManager>) -> Result<Vec<serde_json::Value>> {
    let mut stats = Vec::new();
    let relays = media_relay.get_relays().await?;
    
    for (peer_id, relay) in relays {
        if let Ok(relay_stats) = relay.get_stats().await {
            stats.push(json!({
                "peer_id": peer_id,
                "stats": {
                    "packets_received": relay_stats.packets_received,
                    "packets_sent": relay_stats.packets_sent,
                    "bytes_received": relay_stats.bytes_received,
                    "bytes_sent": relay_stats.bytes_sent,
                    "last_updated": relay_stats.last_updated.elapsed().as_secs()
                }
            }));
        }
    }
    
    Ok(stats)
}

fn with_media_relay(
    media_relay: Arc<MediaRelayManager>,
) -> impl Filter<Extract = (Arc<MediaRelayManager>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || media_relay.clone())
} 