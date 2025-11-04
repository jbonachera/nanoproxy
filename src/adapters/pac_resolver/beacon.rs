use crate::domain::{PacRule, Result};
use crate::ports::ProxyResolverPort;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

/// Beacon poller that checks if proxy hosts are reachable
pub struct BeaconPoller {
    rules: Vec<PacRule>,
    resolver: Arc<dyn ProxyResolverPort>,
}

impl BeaconPoller {
    pub fn new(rules: Vec<PacRule>, resolver: Arc<dyn ProxyResolverPort>) -> Self {
        Self { rules, resolver }
    }

    /// Start polling beacons in the background
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(3));

            loop {
                ticker.tick().await;
                if let Err(e) = self.refresh_pac_url().await {
                    log::warn!("Failed to refresh PAC URL: {}", e);
                }
            }
        })
    }

    fn select_pac_url(&self) -> Option<String> {
        for rule in &self.rules {
            // Try to resolve the beacon host
            if rule.beacon_host.to_socket_addrs().is_ok() {
                return Some(rule.pac_url.clone());
            }
        }
        None
    }

    async fn refresh_pac_url(&self) -> Result<()> {
        let pac_url = self.select_pac_url();
        self.resolver.update_pac_url(pac_url).await
    }
}
