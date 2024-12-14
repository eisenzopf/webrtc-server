use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use hmac::{Hmac, Mac};
use sha1::Sha1;

#[derive(Debug, Serialize)]
pub struct TurnCredentials {
    pub username: String,
    pub password: String,
    pub ttl: u64,
    pub server: String,
    pub port: u16,
}

impl TurnCredentials {
    pub fn new(server: String, port: u16, secret: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // TTL of 12 hours
        let ttl = 12 * 3600;
        let expires = timestamp + ttl;
        
        // Create temporary username
        let username = format!("{}:{}", expires, "webrtc-user");
        
        // Generate HMAC-SHA1 password
        let mut mac = Hmac::<Sha1>::new_from_slice(secret.as_bytes())
            .expect("HMAC initialization failed");
        mac.update(username.as_bytes());
        let password = base64::encode(mac.finalize().into_bytes());

        TurnCredentials {
            username,
            password,
            ttl,
            server,
            port,
        }
    }
}
