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
use webrtc_server::config::ServerConfig;
use warp::cors::CorsForbidden;
use std::env;
use std::path::PathBuf;
use webrtc_server::voip::VoipGateway;
use webrtc_server::config::SipConfig;
use dotenv::dotenv;

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
    // Load environment variables from .env file if present
    dotenv().ok();

    // Create recordings directory
    let recording_path = PathBuf::from("recordings");
    std::fs::create_dir_all(&recording_path)?;

    // Load configuration from environment
    let config = webrtc_server::config::ServerConfig::from_env();

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
        config.clone(),
        config.turn_server.clone(),
        config.turn_port,
        config.turn_password.clone()
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

    // Load configuration from environment
    let config = webrtc_server::config::ServerConfig::from_env();

    // Create recordings directory
    let recording_path = PathBuf::from("recordings");
    std::fs::create_dir_all(&recording_path)?;

    // Create MediaRelayManager for debug server
    let media_relay = Arc::new(MediaRelayManager::new(
        config.turn_server.clone(),
        config.turn_port,
        config.turn_server.clone(),
        config.turn_port,
        config.turn_username.clone(),
        config.turn_password.clone()
    ));

    // Create SignalingServer
    let server = SignalingServer::new(
        config.clone(),
        config.turn_server.clone(),
        config.turn_port,
        config.turn_password.clone()
    ).await?;

    // Initialize VoIP gateway if enabled
    if let Some(sip_config) = config.sip_config {
        info!("Initializing VoIP gateway with local SIP server on {}:{}", 
            sip_config.bind_address, sip_config.port);
        
        let voip_gateway = VoipGateway::new(
            &format!("{}:{}", sip_config.bind_address, sip_config.port),
            &sip_config.domain,
            server.handler.clone(),
        ).await?;

        tokio::spawn(async move {
            if let Err(e) = voip_gateway.start().await {
                error!("VoIP gateway error: {}", e);
            }
        });
    }

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
        &config.turn_server,
        config.turn_port,
        "webrtc.rs",
        vec![(config.turn_username.clone(), config.turn_password.clone())],
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