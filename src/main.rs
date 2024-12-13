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
            // Add more credentials as needed
        ],
    ).await?;

    // Update WebRTC configuration to use the TURN server
    let media_relay = Arc::new(MediaRelayManager::new_with_turn(
        "192.168.1.68",  // TURN server IP
        3478,            // TURN server port
        "testuser",      // TURN username
        "testpass",      // TURN password
    ));

    let handler = Arc::new(MessageHandler::new(media_relay.clone()));

    // Start the cleanup task
    let cleanup_manager = media_relay.clone();
    tokio::spawn(async move {
        cleanup_manager.cleanup_stale_relays().await;
    });

    // Start the monitoring task
    let monitor_manager = media_relay.clone();
    tokio::spawn(async move {
        monitor_manager.monitor_relays().await;
    });

    // Start debug server in a separate task
    let debug_relay = media_relay.clone();
    let debug_handle = tokio::spawn(async move {
        info!("Starting debug server...");
        run_debug_server(debug_relay).await;
    });

    // Create shutdown signal handler
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
    let shutdown_tx_clone = shutdown_tx.clone();

    // Handle Ctrl+C
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("Received shutdown signal, initiating graceful shutdown...");
                shutdown_tx_clone.send(()).unwrap_or_else(|e| {
                    error!("Failed to send shutdown signal: {}", e);
                    0
                });
            }
            Err(err) => {
                error!("Failed to listen for shutdown signal: {}", err);
            }
        }
    });

    // Run the main WebSocket server with shutdown handling
    let server = SignalingServer::new(handler);
    select! {
        result = server.run() => {
            if let Err(e) = result {
                error!("Server error: {}", e);
            }
        }
        _ = shutdown_rx.recv() => {
            info!("Shutting down server...");
            if let Err(e) = turn_server.close().await {
                error!("Error shutting down TURN server: {}", e);
            }
        }
    }

    // Clean up monitoring tasks
    debug_handle.abort();

    info!("Server shutdown complete");
    Ok(())
}