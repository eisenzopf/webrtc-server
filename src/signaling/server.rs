use crate::utils::{Error, Result};
use crate::signaling::handler::MessageHandler;
use crate::types::{SignalingMessage, WebSocketConnection, TurnCredentials};
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
use serde::Serialize;

pub struct SignalingServer {
    pub address: String,
    pub handler: Arc<MessageHandler>,
    pub config: ServerConfig,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub stun_server: String,
    pub stun_port: u16,
    pub turn_server: String,
    pub turn_port: u16,
    pub turn_username: String,
    pub turn_password: String,
    pub ws_port: u16,
}

impl SignalingServer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        let relay_manager = Arc::new(MediaRelayManager::new(
            config.stun_server.clone(),
            config.stun_port,
            config.turn_server.clone(),
            config.turn_port,
            config.turn_username.clone(),
            config.turn_password.clone(),
        ));

        Ok(SignalingServer {
            address: format!("0.0.0.0:{}", config.ws_port),
            handler: Arc::new(MessageHandler::new(relay_manager)),
            config,
        })
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
        let ws_sender = Arc::new(Mutex::new(ws_sender));
        let ws_conn = WebSocketConnection::new_tungstenite(ws_sender.clone());
        
        // We'll store this connection temporarily until we get the Join message
        let temp_id = format!("temp_{}", addr);
        handler.set_websocket_sender(temp_id.clone(), ws_conn).await?;

        while let Some(msg) = ws_receiver.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                let message: SignalingMessage = serde_json::from_str(&text)?;
                
                match message.clone() {  // Clone the message to avoid ownership issues
                    SignalingMessage::Join { peer_id, .. } => {
                        let new_ws_conn = WebSocketConnection::new_tungstenite(ws_sender.clone());
                        handler.set_websocket_sender(peer_id.clone(), new_ws_conn).await?;
                        // Remove temporary connection
                        handler.remove_websocket_sender(&temp_id).await?;
                        handler.handle_message(message, &peer_id).await?;
                    },
                    _ => {
                        handler.handle_message(message, &temp_id).await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn turn_credentials_route(&self) -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
        let credentials = TurnCredentials {
            stun_server: self.config.stun_server.clone(),
            stun_port: self.config.stun_port,
            turn_server: self.config.turn_server.clone(),
            turn_port: self.config.turn_port,
            username: self.config.turn_username.clone(),
            password: self.config.turn_password.clone(),
        };
        
        warp::path!("api" / "turn-credentials")
            .and(warp::get())
            .map(move || {
                warp::reply::json(&credentials)
            })
    }

    pub fn ws_route(&self) -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
        let handler = self.handler.clone();
        
        warp::ws()
            .and(warp::addr::remote())
            .map(move |ws: warp::ws::Ws, addr: Option<SocketAddr>| {
                let handler = handler.clone();
                let addr = addr.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));
                
                ws.on_upgrade(move |websocket| async move {
                    let (ws_sender, mut ws_receiver) = websocket.split();
                    let ws_sender = Arc::new(Mutex::new(ws_sender));
                    let ws_conn = WebSocketConnection::new_warp(ws_sender.clone());
                    
                    // Generate a temporary ID for the connection
                    let temp_id = format!("temp_{}", addr);
                    if let Err(e) = handler.set_websocket_sender(temp_id.clone(), ws_conn).await {
                        error!("Failed to set websocket sender: {}", e);
                        return;
                    }
                    
                    info!("New WebSocket connection from: {}", addr);

                    while let Some(result) = ws_receiver.next().await {
                        match result {
                            Ok(msg) => {
                                if let Ok(text) = msg.to_str() {
                                    match serde_json::from_str::<SignalingMessage>(text) {
                                        Ok(message) => {
                                            let message_clone = message.clone();
                                            match message {
                                                SignalingMessage::Join { peer_id, .. } => {
                                                    let new_ws_conn = WebSocketConnection::new_warp(ws_sender.clone());
                                                    if let Err(e) = handler.set_websocket_sender(peer_id.clone(), new_ws_conn).await {
                                                        error!("Failed to set websocket sender for peer {}: {}", peer_id, e);
                                                        continue;
                                                    }
                                                    if let Err(e) = handler.remove_websocket_sender(&temp_id).await {
                                                        error!("Failed to remove temporary connection: {}", e);
                                                    }
                                                    if let Err(e) = handler.handle_message(message_clone, &peer_id).await {
                                                        error!("Failed to handle join message: {}", e);
                                                    }
                                                },
                                                _ => {
                                                    if let Err(e) = handler.handle_message(message_clone, &temp_id).await {
                                                        error!("Failed to handle message: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => error!("Failed to parse message: {}", e),
                                    }
                                }
                            }
                            Err(e) => {
                                error!("WebSocket error: {}", e);
                                break;
                            }
                        }
                    }
                    
                    // Clean up when the connection is closed
                    if let Err(e) = handler.remove_websocket_sender(&temp_id).await {
                        error!("Failed to remove websocket sender: {}", e);
                    }
                })
            })
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

async fn handle_websocket_connection(
    ws_stream: WebSocketStream<TcpStream>,
    handler: Arc<MessageHandler>,
    addr: SocketAddr,
) -> Result<()> {
    let (ws_sender, mut ws_receiver) = ws_stream.split();
    let ws_sender = Arc::new(Mutex::new(ws_sender));
    let ws_conn = WebSocketConnection::new_tungstenite(ws_sender.clone());
    
    // Generate a temporary ID for the connection
    let temp_id = format!("temp_{}", addr);
    handler.set_websocket_sender(temp_id.clone(), ws_conn).await?;
    
    info!("New connection from: {}", addr);

    while let Some(msg) = ws_receiver.next().await {
        let msg = msg?;
        if let Message::Text(text) = msg {
            let message: SignalingMessage = serde_json::from_str(&text)?;
            
            match message.clone() {  // Clone the message to avoid ownership issues
                SignalingMessage::Join { peer_id, .. } => {
                    // Update connection with real peer ID
                    let new_ws_conn = WebSocketConnection::new_tungstenite(ws_sender.clone());
                    handler.set_websocket_sender(peer_id.clone(), new_ws_conn).await?;
                    handler.remove_websocket_sender(&temp_id).await?;
                    handler.handle_message(message, &peer_id).await?;
                },
                _ => {
                    handler.handle_message(message, &temp_id).await?;
                }
            }
        }
    }
    Ok(())
} 