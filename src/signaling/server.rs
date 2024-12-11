use crate::utils::{Error, Result};
use crate::signaling::handler::MessageHandler;
use crate::signaling::SignalingMessage;
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
            address: "0.0.0.0:8080".to_string(), // Changed from 127.0.0.1 to 0.0.0.0
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
        let peer_addr = stream.peer_addr().map_err(|e| {
            error!("Failed to get peer address: {}", e);
            Error::IO(e)
        })?;
        
        debug!("Upgrading connection to WebSocket for peer {}", peer_addr);
        let ws_stream = accept_async(stream).await
            .map_err(|e| {
                error!("WebSocket upgrade failed for {}: {}", peer_addr, e);
                Error::WebSocket(e)
            })?;
        
        info!("Successfully established WebSocket connection with {}", peer_addr);
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));
        
        let mut current_peer_id: Option<String> = None;
        let mut current_room_id: Option<String> = None;

        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    if let Ok(msg_str) = msg.to_text() {
                        debug!("Received message from {}: {}", peer_addr, msg_str);
                        if let Ok(signal_msg) = serde_json::from_str(msg_str) {
                            // Update peer and room IDs when handling Join message
                            if let SignalingMessage::Join { peer_id, room_id, .. } = &signal_msg {
                                current_peer_id = Some(peer_id.clone());
                                current_room_id = Some(room_id.clone());
                                
                                // Set up media relay handlers after Join
                                if let Some(relay) = handler.get_peer_relay(peer_id).await? {
                                    let relay_clone = relay.clone();
                                    let handler_clone = handler.clone();
                                    let peer_id_clone = peer_id.clone();
                                    
                                    relay.peer_connection.on_track(Box::new(move |track, _, _| {
                                        let relay = relay_clone.clone();
                                        let handler = handler_clone.clone();
                                        let peer_id = peer_id_clone.clone();
                                        
                                        Box::pin(async move {
                                            // Create a new local track for relaying based on track type
                                            let track_id = match track.kind() {
                                                RTPCodecType::Audio => format!("audio-relay-{}", track.stream_id()),
                                                RTPCodecType::Video => format!("video-relay-{}", track.stream_id()),
                                                _ => {
                                                    error!("Unsupported track type: {:?}", track.kind());
                                                    return;
                                                }
                                            };

                                            debug!("Creating new {} relay track: {}", track.kind(), track_id);
                                            
                                            let local_track = Arc::new(TrackLocalStaticRTP::new(
                                                track.codec().capability,
                                                track_id,
                                                track.stream_id(),
                                            ));

                                            // Forward this track to all other peers in the room
                                            if let Err(e) = handler.broadcast_track(&peer_id, local_track).await {
                                                error!("Failed to broadcast {} track: {}", track.kind(), e);
                                            }
                                        })
                                    }));
                                }
                            }
                            
                            handler.handle_message(
                                signal_msg,
                                ws_sender.clone(),
                            ).await?;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("WebSocket error: {}", e);
                    break;
                }
            }
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