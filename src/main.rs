#![allow(warnings)]
use anyhow::Result;
use webrtc_server::signaling::server::SignalingServer;
use std::error::Error;
use env_logger::Env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging with default level INFO
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    log::info!("Starting WebRTC signaling server...");

    // Create and start the server
    let server = SignalingServer::new("127.0.0.1:8080");
    
    // Start the server and handle any errors
    server.start().await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
    log::info!("Server stopped normally");
    Ok(())
}