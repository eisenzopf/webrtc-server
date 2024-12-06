use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ConnectionMetrics {
    pub peer_id: String,
    pub last_seen: Instant,
    pub packet_loss_rate: f32,
    pub latency: Duration,
}

impl ConnectionMetrics {
    pub fn new(peer_id: String) -> Self {
        Self {
            peer_id,
            last_seen: Instant::now(),
            packet_loss_rate: 0.0,
            latency: Duration::from_millis(0),
        }
    }

    pub fn update_metrics(&mut self, packet_loss: f32, latency: Duration) {
        self.last_seen = Instant::now();
        self.packet_loss_rate = packet_loss;
        self.latency = latency;
    }
}