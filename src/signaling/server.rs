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
use crate::monitoring::ConnectionMonitor;
use crate::signaling::connection_state::{ConnectionState, ConnectionStateManager};
use crate::monitoring::dashboard::{ConnectionMetrics, check_for_alerts, StateChangeBroadcaster};
use warp::Rejection;
use crate::monitoring::run_connection_monitor;
use std::collections::HashMap;

pub struct SignalingServer {
    pub address: String,
    pub handler: Arc<MessageHandler>,
    pub config: ServerConfig,
    turn_secret: String,
    turn_server: String,
    turn_port: u16,
    connection_monitor: Arc<ConnectionMonitor>,
    state_manager: Arc<ConnectionStateManager>,
    state_broadcaster: Arc<StateChangeBroadcaster>,
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
    pub async fn new(config: ServerConfig, turn_server: String, turn_port: u16, turn_secret: String) -> Result<Self> {
        let relay_manager = Arc::new(MediaRelayManager::new(
            config.stun_server.clone(),
            config.stun_port,
            config.turn_server.clone(),
            config.turn_port,
            config.turn_username.clone(),
            config.turn_password.clone(),
        ));

        let connection_monitor = Arc::new(ConnectionMonitor::new());
        let state_manager = Arc::new(ConnectionStateManager::new());
        let state_broadcaster = Arc::new(StateChangeBroadcaster::new());
        
        // Spawn the connection monitor task
        let monitor_clone = connection_monitor.clone();
        tokio::spawn(async move {
            run_connection_monitor(monitor_clone).await;
        });

        Ok(SignalingServer {
            address: format!("0.0.0.0:{}", config.ws_port),
            handler: Arc::new(MessageHandler::new(relay_manager)),
            config,
            turn_secret,
            turn_server,
            turn_port,
            connection_monitor,
            state_manager,
            state_broadcaster,
        })
    }

    pub async fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(&self.address).await?;
        info!("Server successfully bound to {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            info!("New connection from: {}", addr);
            let handler = self.handler.clone();
            let state_manager = self.state_manager.clone();
            
            tokio::spawn(async move {
                let ws_stream = accept_async(stream).await
                    .map_err(|e| Error::WebSocketError(e.to_string()))?;
                
                if let Err(e) = Self::handle_connection(ws_stream, addr, handler, state_manager).await {
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
        state_manager: Arc<ConnectionStateManager>,
    ) -> Result<()> {
        info!("New WebSocket connection established from: {}", addr);
        
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(Mutex::new(ws_sender));
        let ws_conn = WebSocketConnection::new_tungstenite(ws_sender.clone());
        
        // Generate temporary ID and log it
        let temp_id = format!("temp_{}", addr);
        debug!("Assigning temporary ID: {} to connection from {}", temp_id, addr);
        
        handler.set_websocket_sender(temp_id.clone(), ws_conn).await?;

        while let Some(msg) = ws_receiver.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                debug!("Received message from {}: {}", addr, text);
                
                let message: SignalingMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        error!("Failed to parse message from {}: {}", addr, e);
                        continue;
                    }
                };
                
                match message.clone() {
                    SignalingMessage::Join { peer_id, .. } => {
                        info!("Peer {} joining from address {}", peer_id, addr);
                        
                        // Initialize connection state
                        if state_manager.transition(&peer_id, ConnectionState::New).await {
                            debug!("Initialized state for peer {}", peer_id);
                            state_manager.transition(&peer_id, ConnectionState::Joining).await;
                        } else {
                            warn!("Failed to initialize state for peer {}", peer_id);
                        }

                        let new_ws_conn = WebSocketConnection::new_tungstenite(ws_sender.clone());
                        
                        debug!("Updating connection mapping for peer {}", peer_id);
                        handler.set_websocket_sender(peer_id.clone(), new_ws_conn).await?;
                        
                        debug!("Removing temporary connection {}", temp_id);
                        handler.remove_websocket_sender(&temp_id).await?;
                        
                        info!("Processing join message for peer {}", peer_id);
                        handler.handle_message(message, &peer_id).await?;
                    },
                    SignalingMessage::CallRequest { from_peer, .. } => {
                        state_manager.transition(&from_peer, ConnectionState::WaitingForOffer).await;
                        handler.handle_message(message, &temp_id).await?;
                    },
                    SignalingMessage::CallResponse { from_peer, accepted, .. } => {
                        if accepted {
                            state_manager.transition(&from_peer, ConnectionState::AnswerCreated).await;
                        } else {
                            state_manager.transition(&from_peer, ConnectionState::Failed).await;
                        }
                        handler.handle_message(message, &temp_id).await?;
                    },
                    SignalingMessage::IceCandidate { from_peer, .. } => {
                        if let Some(state) = state_manager.get_state(&from_peer).await {
                            match state {
                                ConnectionState::OfferReceived | 
                                ConnectionState::AnswerCreated | 
                                ConnectionState::Connected => {
                                    debug!("Processing ICE candidate in state: {:?}", state);
                                    handler.handle_message(message, &temp_id).await?;
                                },
                                _ => {
                                    warn!("Received ICE candidate in invalid state: {:?}", state);
                                }
                            }
                        }
                    },
                    _ => {
                        debug!("Processing other message type from {}", addr);
                        handler.handle_message(message, &temp_id).await?;
                    }
                }
            }
        }
        
        // Handle disconnection
        if let Some(peer_id) = get_peer_id_from_temp(&temp_id) {
            state_manager.transition(&peer_id, ConnectionState::Closed).await;
        }
        
        info!("Connection closed for {}", addr);
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

    pub fn monitoring_routes(&self) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
        let state_manager = self.state_manager.clone();
        
        let metrics_route = warp::path!("api" / "monitoring" / "metrics")
            .and(warp::get())
            .and(with_state_manager(state_manager.clone()))
            .and_then(handle_get_metrics);
            
        let alerts_route = warp::path!("api" / "monitoring" / "alerts")
            .and(warp::get())
            .and(with_state_manager(state_manager.clone()))
            .and_then(handle_get_alerts);
            
        metrics_route.or(alerts_route)
    }

    pub fn monitoring_ws_route(&self) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
        let state_broadcaster = self.state_broadcaster.clone();
        
        warp::path!("api" / "monitoring" / "ws")
            .and(warp::ws())
            .map(move |ws: warp::ws::Ws| {
                let state_broadcaster = state_broadcaster.clone();
                ws.on_upgrade(move |socket| handle_monitoring_socket(socket, state_broadcaster))
            })
    }

    pub async fn get_connection_state(&self, peer_id: &str) -> Option<ConnectionState> {
        self.state_manager.get_state(peer_id).await
    }

    pub fn debug_routes(&self) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
        let state_manager = self.state_manager.clone();
        
        let state_route = warp::path!("debug" / "connection-states")
            .and(warp::get())
            .and(with_state_manager(state_manager))
            .and_then(handle_get_states);

        state_route
    }

    async fn handle_connection_error(
        &self,
        peer_id: &str,
        error: &Error,
        state_manager: &Arc<ConnectionStateManager>
    ) -> Result<()> {
        error!("Connection error for peer {}: {}", peer_id, error);
        
        // Get current state
        let current_state = state_manager.get_state(peer_id).await;
        
        match current_state {
            Some(ConnectionState::Connected) => {
                // Try to recover if we were connected
                warn!("Attempting to recover connection for peer {}", peer_id);
                state_manager.transition(peer_id, ConnectionState::WaitingForOffer).await;
                
                // Notify peer to retry connection
                let error_message = SignalingMessage::ConnectionError {
                    peer_id: peer_id.to_string(),
                    error: error.to_string(),
                    should_retry: true,
                };
                self.handler.send_message(&error_message).await?;
            },
            Some(state) => {
                // For other states, fail the connection
                warn!("Connection failed for peer {} in state {:?}", peer_id, state);
                state_manager.transition(peer_id, ConnectionState::Failed).await;
                
                // Notify peer of failure
                let error_message = SignalingMessage::ConnectionError {
                    peer_id: peer_id.to_string(),
                    error: error.to_string(),
                    should_retry: false,
                };
                self.handler.send_message(&error_message).await?;
            },
            None => {
                error!("Error occurred for unknown peer {}", peer_id);
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

fn with_turn_config(
    server: String,
    port: u16,
    secret: String,
) -> impl Filter<Extract = ((String, u16, String),), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || (server.clone(), port, secret.clone()))
}

async fn handle_turn_credentials(
    (server, port, secret): (String, u16, String),
) -> std::result::Result<impl Reply, Rejection> {
    let credentials = TurnCredentials::new(server, port, &secret);
    Ok(warp::reply::json(&credentials))
}

// Add detailed logging for WebSocket messages
async fn handle_ws_message(
    message: Message,
    monitor: Arc<ConnectionMonitor>
) {
    match message {
        Message::Text(text) => {
            debug!("Received WebSocket message: {}", text);
            if let Ok(signal) = serde_json::from_str::<SignalingMessage>(&text) {
                match signal {
                    SignalingMessage::CallRequest { from_peer, .. } => {
                        monitor.register_connection(&from_peer).await;
                    },
                    SignalingMessage::IceCandidate { from_peer, .. } => {
                        monitor.record_ice_candidate(&from_peer, true).await;
                    },
                    SignalingMessage::CallResponse { from_peer, accepted, .. } => {
                        monitor.update_connection_state(
                            &from_peer, 
                            if accepted { "connected" } else { "rejected" }
                        ).await;
                    },
                    _ => {}
                }
            }
        },
        _ => warn!("Received non-text WebSocket message"),
    }
}

async fn handle_get_connections(
    monitor: Arc<ConnectionMonitor>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    let stats = monitor.get_connection_stats().await;
    Ok(warp::reply::json(&stats))
}

// Helper function to extract peer ID from temporary ID
fn get_peer_id_from_temp(temp_id: &str) -> Option<String> {
    if temp_id.starts_with("temp_") {
        None
    } else {
        Some(temp_id.to_string())
    }
}

fn with_state_manager(
    state_manager: Arc<ConnectionStateManager>,
) -> impl Filter<Extract = (Arc<ConnectionStateManager>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state_manager.clone())
}

async fn handle_get_states(
    state_manager: Arc<ConnectionStateManager>
) -> std::result::Result<impl Reply, Rejection> {
    let states = state_manager.get_all_states().await;
    Ok(warp::reply::json(&states))
}

async fn handle_get_metrics(
    state_manager: Arc<ConnectionStateManager>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    let states = state_manager.get_all_states().await;
    let metrics = ConnectionMetrics {
        total_connections: states.len(),
        active_connections: states.values()
            .filter(|&state| matches!(state, ConnectionState::Connected))
            .count(),
        failed_connections: states.values()
            .filter(|&state| matches!(state, ConnectionState::Failed))
            .count(),
        success_rate: if states.is_empty() { 0.0 } else {
            let successful = states.values()
                .filter(|&state| matches!(state, ConnectionState::Connected))
                .count() as f64;
            (successful / states.len() as f64) * 100.0
        },
        average_connection_time: None, // You'll need to track connection times separately
        state_distribution: states.into_iter()
            .fold(HashMap::new(), |mut acc, (_, state)| {
                *acc.entry(state).or_insert(0) += 1;
                acc
            }),
    };
    
    Ok(warp::reply::json(&metrics))
}

async fn handle_get_alerts(
    state_manager: Arc<ConnectionStateManager>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    let alerts = check_for_alerts(&state_manager).await;
    Ok(warp::reply::json(&alerts))
}

async fn handle_monitoring_socket(
    socket: warp::ws::WebSocket,
    broadcaster: Arc<StateChangeBroadcaster>,
) {
    let (mut sender, _) = socket.split();
    let mut receiver = broadcaster.subscribe();

    while let Ok(event) = receiver.recv().await {
        if let Ok(json) = serde_json::to_string(&event) {
            if let Err(e) = sender.send(warp::ws::Message::text(json)).await {
                error!("Error sending monitoring message: {}", e);
                break;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStats {
    // ... existing fields ...
}
 