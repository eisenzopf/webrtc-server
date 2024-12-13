use crate::utils::{Error, Result};
use crate::signaling::handler::MessageHandler;
use crate::types::{SignalingMessage, WebSocketSender, WebSocketConnection};
use tokio::net::{TcpListener, TcpStream};
use std::sync::Arc;
use tokio_tungstenite::{accept_async, WebSocketStream};
use futures_util::{StreamExt, SinkExt};
use log::{info, warn, error, debug};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use serde_json::json;
use warp::{Filter, Reply};
use crate::media::MediaRelayManager;
use warp::reject;
use tokio::sync::Mutex;
use std::net::SocketAddr;
use tokio_tungstenite::tungstenite::Message;

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
        let listener = TcpListener::bind(&self.address).await?;
        info!("Server successfully bound to {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            info!("New connection from: {}", addr);
            let handler = self.handler.clone();
            
            tokio::spawn(async move {
                let ws_stream = accept_async(stream).await
                    .map_err(|e| Error::WebSocketError(e.to_string()))?;
                
                if let Err(e) = Self::handle_connection(ws_stream, addr, handler).await {
                    error!("Connection error: {}", e);
                }
                Ok::<_, Error>(())
            });
        }

        Ok(())
    }

    pub async fn handle_connection(
        ws_stream: WebSocketStream<TcpStream>,
        addr: SocketAddr,
        handler: Arc<MessageHandler>,
    ) -> Result<()> {
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_conn = WebSocketConnection::new(ws_sender);
        
        // We'll store this connection temporarily until we get the Join message
        let temp_id = format!("temp_{}", addr);
        handler.set_websocket_sender(temp_id.clone(), ws_conn.clone()).await?;

        while let Some(msg) = ws_receiver.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                let message: SignalingMessage = serde_json::from_str(&text)?;
                
                // If this is a Join message, update the WebSocket sender with the real peer_id
                if let SignalingMessage::Join { peer_id, .. } = &message {
                    handler.set_websocket_sender(peer_id.clone(), ws_conn.clone()).await?;
                    // Remove temporary connection
                    handler.remove_websocket_sender(&temp_id).await?;
                }
                
                handler.handle_message(message).await?;
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