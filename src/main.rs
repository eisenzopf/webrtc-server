#![allow(warnings)]
use anyhow::Result;
use webrtc_server::signaling::server::SignalingServer;
use webrtc_server::signaling::stun::StunService;
use std::error::Error;
use env_logger::Env;
use tokio::select;
use tokio::signal;
use std::sync::Arc;
use webrtc_server::media::MediaRelayManager;
use webrtc_server::signaling::handler::MessageHandler;
use webrtc_server::signaling::server::run_debug_server;
use log::{info, warn, error};
use webrtc_server::signaling::turn::TurnServer;
use webrtc_server::room::manager::RoomManager;
use tokio::sync::Mutex;
use webrtc_server::types::WebSocketConnection;
use warp::Filter;
use warp::fs;
use serde::Serialize;
use webrtc_server::signaling::server::ServerConfig;
use warp::cors::CorsForbidden;
use std::env;

#[derive(Serialize)]
struct TurnCredentials {
    stun_server: String,
    stun_port: u16,
    turn_server: String,
    turn_port: u16,
    username: String,
    password: String,
}

async fn start_server() -> Result<()> {
    let config = ServerConfig {
        stun_server: "192.168.1.68".to_string(),
        stun_port: 3478,
        turn_server: "192.168.1.68".to_string(),
        turn_port: 3478,
        turn_username: "testuser".to_string(),
        turn_password: "testpass".to_string(),
        ws_port: 8080,
    };

    // Create MediaRelayManager with proper arguments
    let media_relay = Arc::new(MediaRelayManager::new(
        config.stun_server.clone(),
        config.stun_port,
        config.turn_server.clone(),
        config.turn_port,
        config.turn_username.clone(),
        config.turn_password.clone(),
    ));

    // Create SignalingServer with proper arguments
    let signaling_server = SignalingServer::new(
        config,
        "192.168.1.68".to_string(),
        3478,
        "your-secret-key".to_string()
    ).await?;
    
    let ws_route = signaling_server.ws_route();
    let credentials_route = signaling_server.turn_credentials_route();
    
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["content-type", "upgrade", "connection"])
        .allow_credentials(true)
        .max_age(3600);

    let static_files = warp::path("static")
        .and(warp::fs::dir("static"));

    let routes = ws_route
        .or(credentials_route)
        .or(static_files)
        .with(cors);
    
    warp::serve(routes)
        .run(([0, 0, 0, 0], signaling_server.config.ws_port))
        .await;
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with more detailed output
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
        .format_timestamp_millis()
        .format_module_path(true)
        .init();

    info!("Starting WebRTC server...");

    // Load configuration
    let turn_secret = env::var("TURN_SECRET").unwrap_or_else(|_| "your-secret-key".to_string());
    let turn_server = env::var("TURN_SERVER").unwrap_or_else(|_| "192.168.1.68".to_string());
    let turn_port = env::var("TURN_PORT")
        .unwrap_or_else(|_| "3478".to_string())
        .parse::<u16>()
        .expect("Invalid TURN port");

    // Create MediaRelayManager for debug server
    let media_relay = Arc::new(MediaRelayManager::new(
        turn_server.clone(),
        turn_port,
        turn_server.clone(),
        turn_port,
        "testuser".to_string(),
        "testpass".to_string()
    ));

    let server_config = ServerConfig {
        stun_server: turn_server.clone(),
        stun_port: turn_port,
        turn_server: turn_server.clone(),
        turn_port,
        turn_username: "testuser".to_string(),
        turn_password: "testpass".to_string(),
        ws_port: 8080,
    };

    // Create SignalingServer first
    let server = SignalingServer::new(
        server_config,
        turn_server.clone(),
        turn_port,
        turn_secret.clone()
    ).await?;

    // Create routes
    let ws_route = server.ws_route();
    let credentials_route = server.turn_credentials_route();
    
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["content-type", "upgrade", "connection"])
        .allow_credentials(true)
        .max_age(3600);

    let static_files = warp::path("static")
        .and(warp::fs::dir("static"));

    let routes = ws_route
        .or(credentials_route)
        .or(static_files)
        .with(cors);

    // Start TURN server
    let turn_server = TurnServer::new(
        &turn_server,
        turn_port,
        "webrtc.rs",
        vec![("testuser".to_string(), "testpass".to_string())],
    ).await?;

    // Start debug server
    info!("Starting debug server on port 8081...");
    let debug_server = tokio::spawn(run_debug_server(media_relay.clone()));

    // Start the main server
    warp::serve(routes)
        .run(([0, 0, 0, 0], server.config.ws_port))
        .await;

    Ok(())
}