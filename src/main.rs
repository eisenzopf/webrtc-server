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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with default level
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    info!("Starting WebRTC server...");

    let media_relay = Arc::new(MediaRelayManager::new());
    let handler = Arc::new(MessageHandler::new(media_relay.clone()));

    // Start media relay monitoring in a separate task
    let monitor_relay = media_relay.clone();
    let monitor_handle = tokio::spawn(async move {
        info!("Starting media relay monitoring...");
        monitor_relay.monitor_relays().await;
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
        }
    }

    // Clean up monitoring tasks
    monitor_handle.abort();
    debug_handle.abort();

    info!("Server shutdown complete");
    Ok(())
}