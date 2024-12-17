use std::env;
use std::path::PathBuf;

#[derive(Clone)]
pub struct ServerConfig {
    pub stun_server: String,
    pub stun_port: u16,
    pub turn_server: String,
    pub turn_port: u16,
    pub turn_username: String,
    pub turn_password: String,
    pub ws_port: u16,
    pub recording_path: Option<PathBuf>,
    pub sip_config: Option<SipConfig>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            stun_server: env::var("STUN_SERVER")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            stun_port: env::var("STUN_PORT")
                .unwrap_or_else(|_| "3478".to_string())
                .parse()
                .unwrap_or(3478),
            turn_server: env::var("TURN_SERVER")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            turn_port: env::var("TURN_PORT")
                .unwrap_or_else(|_| "3478".to_string())
                .parse()
                .unwrap_or(3478),
            turn_username: env::var("TURN_USERNAME")
                .unwrap_or_else(|_| "webrtc".to_string()),
            turn_password: env::var("TURN_PASSWORD")
                .unwrap_or_else(|_| "webrtc".to_string()),
            ws_port: env::var("WS_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .unwrap_or(8080),
            recording_path: Some(PathBuf::from("recordings")),
            sip_config: None,
        }
    }
}

#[derive(Clone)]
pub struct SipConfig {
    pub bind_address: String,
    pub port: u16,
    pub domain: String,
}

impl SipConfig {
    pub fn from_env() -> Self {
        Self {
            bind_address: env::var("SIP_BIND_ADDRESS")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("SIP_PORT")
                .unwrap_or_else(|_| "5060".to_string())
                .parse()
                .unwrap_or(5060),
            domain: env::var("SIP_DOMAIN")
                .unwrap_or_else(|_| "localhost".to_string()),
        }
    }
}
