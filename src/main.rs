#![allow(warnings)]
use anyhow::Result;
use webrtc_server::signaling::server::SignalingServer;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging (you might want to add proper logging setup here)
    println!("Starting WebRTC signaling server...");

    // Create and start the server
    let server = SignalingServer::new("127.0.0.1:8080");
    
    // Start the server and handle any errors
    server.start().await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
    println!("Server stopped normally");
    Ok(())
}