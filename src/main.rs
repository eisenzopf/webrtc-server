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

    let signaling_server = SignalingServer::new(config).await?;
    
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
    // Initialize logging with default level
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    info!("Starting WebRTC server...");

    // Start TURN server
    let turn_server = TurnServer::new(
        "192.168.1.68",  // Your public IP
        3478,            // TURN port
        "webrtc.rs",     // Realm
        vec![
            ("testuser".to_string(), "testpass".to_string()),
        ],
    ).await?;

    // Start debug server
    info!("Starting debug server on port 8081...");
    let debug_handle = tokio::spawn(async {
        run_debug_server(Arc::new(MediaRelayManager::new(
            "192.168.1.68".to_string(),
            3478,
            "192.168.1.68".to_string(),
            3478,
            "testuser".to_string(),
            "testpass".to_string(),
        ))).await;
    });

    // Start the main server
    start_server().await?;

    Ok(())
}