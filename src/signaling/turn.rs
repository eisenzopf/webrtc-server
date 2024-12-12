use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::time::Duration;
use turn::auth::*;
use turn::relay::relay_static::*;
use turn::server::config::*;
use turn::server::*;
use turn::Error;
use util::vnet::net::*;
use log::{info, error};
use crate::utils::Result;

struct TurnAuthHandler {
    cred_map: HashMap<String, Vec<u8>>,
}

impl TurnAuthHandler {
    fn new(cred_map: HashMap<String, Vec<u8>>) -> Self {
        Self { cred_map }
    }
}

impl AuthHandler for TurnAuthHandler {
    fn auth_handle(
        &self,
        username: &str,
        _realm: &str,
        _src_addr: SocketAddr,
    ) -> std::result::Result<Vec<u8>, Error> {
        self.cred_map
            .get(username)
            .cloned()
            .ok_or(Error::ErrFakeErr)
    }
}

pub struct TurnServer {
    server: Server,
}

impl TurnServer {
    pub async fn new(
        public_ip: &str,
        port: u16,
        realm: &str,
        credentials: Vec<(String, String)>,
    ) -> Result<Self> {
        let mut cred_map = HashMap::new();
        for (username, password) in credentials {
            let key = generate_auth_key(&username, realm, &password);
            cred_map.insert(username, key);
        }

        let conn = Arc::new(UdpSocket::bind(format!("0.0.0.0:{port}")).await?);
        info!("TURN server listening on {}", conn.local_addr()?);

        let server = Server::new(ServerConfig {
            conn_configs: vec![ConnConfig {
                conn,
                relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                    relay_address: IpAddr::from_str(public_ip)?,
                    address: "0.0.0.0".to_owned(),
                    net: Arc::new(Net::new(None)),
                }),
            }],
            realm: realm.to_owned(),
            auth_handler: Arc::new(TurnAuthHandler::new(cred_map)),
            channel_bind_timeout: Duration::from_secs(0),
            alloc_close_notify: None,
        })
        .await?;

        Ok(Self { server })
    }

    pub async fn close(&self) -> Result<()> {
        self.server.close().await?;
        Ok(())
    }
} 