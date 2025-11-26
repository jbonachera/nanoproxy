use crate::domain::{GatewayRule, ProxyError, Result};
use crate::ports::ProxyResolverPort;
use ipnet::Ipv4Net;
use log::warn;
use std::sync::Arc;
use std::time::Duration;

pub struct GatewayListener {
    rules: Vec<GatewayRule>,
    resolver: Arc<dyn ProxyResolverPort>,
}

impl GatewayListener {
    pub fn new(rules: Vec<GatewayRule>, resolver: Arc<dyn ProxyResolverPort>) -> Self {
        Self { rules, resolver }
    }

    pub fn start(self) -> Result<tokio::task::JoinHandle<()>> {
        let resolver_clone = self.resolver.clone();
        let rules_clone = self.rules.clone();

        Ok(tokio::spawn(async move {
            if let Err(e) = Self::refresh_rules_static(&rules_clone, &resolver_clone).await {
                log::info!("Failed initial gateway refresh: {:?}", e);
            }

            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;

                if let Err(e) = Self::refresh_rules_static(&rules_clone, &resolver_clone).await {
                    log::info!("Failed to detect gateway: {:?}", e);
                }
            }
        }))
    }

    async fn refresh_rules_static(rules: &[GatewayRule], resolver: &Arc<dyn ProxyResolverPort>) -> Result<()> {
        let interface = netdev::get_default_interface()
            .map_err(|e| ProxyError::ResolutionFailed(format!("Cannot get default interface: {}", e)))?;

        let gateway = interface
            .gateway
            .ok_or_else(|| ProxyError::ResolutionFailed("No gateway found".to_string()))?;

        let gateway_ip_str = gateway
            .ipv4
            .first()
            .ok_or_else(|| ProxyError::ResolutionFailed("No IPv4 gateway found".to_string()))?
            .to_string();

        let gateway_addr: std::net::Ipv4Addr = gateway_ip_str
            .parse()
            .map_err(|e| ProxyError::ResolutionFailed(format!("Invalid gateway IP: {}", e)))?;

        let mut matched = false;
        for rule in rules {
            let net: Ipv4Net = rule
                .default_gateway_subnet
                .parse()
                .map_err(|e| ProxyError::ResolutionFailed(format!("Invalid subnet: {}", e)))?;

            if net.contains(&gateway_addr) {
                resolver.update_pac_url(Some(rule.pac_url.clone())).await?;
                matched = true;

                if let Some(cmd) = &rule.when_match {
                    warn!("Running command: {}", cmd);
                    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).output();
                }
                break;
            } else if let Some(cmd) = &rule.when_no_match {
                warn!("Running command: {}", cmd);
                let _ = std::process::Command::new("sh").arg("-c").arg(cmd).output();
            }
        }

        if !matched {
            resolver.update_pac_url(None).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProxyRoute;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use url::Url;

    struct MockResolver {
        update_count: AtomicUsize,
        last_pac_url: tokio::sync::RwLock<Option<String>>,
    }

    impl MockResolver {
        fn new() -> Self {
            Self {
                update_count: AtomicUsize::new(0),
                last_pac_url: tokio::sync::RwLock::new(None),
            }
        }

        async fn get_last_pac_url(&self) -> Option<String> {
            self.last_pac_url.read().await.clone()
        }

        fn get_update_count(&self) -> usize {
            self.update_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ProxyResolverPort for MockResolver {
        async fn update_pac_url(&self, pac_url: Option<String>) -> Result<()> {
            self.update_count.fetch_add(1, Ordering::SeqCst);
            *self.last_pac_url.write().await = pac_url;
            Ok(())
        }

        async fn resolve_route(&self, _target_url: &Url) -> Result<ProxyRoute> {
            Ok(ProxyRoute::Direct)
        }

        async fn resolve_all_routes(&self, _target_url: &Url) -> Result<Vec<ProxyRoute>> {
            Ok(vec![ProxyRoute::Direct])
        }
    }

    #[tokio::test]
    async fn test_gateway_listener_updates_pac_url() {
        let rules = vec![GatewayRule {
            default_gateway_subnet: "0.0.0.0/0".to_string(),
            pac_url: "http://test.pac".to_string(),
            when_match: None,
            when_no_match: None,
        }];

        let resolver = Arc::new(MockResolver::new());
        let listener = GatewayListener::new(rules, resolver.clone());

        let handle = listener.start().expect("Failed to start listener");

        tokio::time::sleep(Duration::from_millis(100)).await;

        let pac_url = resolver.get_last_pac_url().await;
        assert!(pac_url.is_some());
        assert_eq!(pac_url.unwrap(), "http://test.pac");

        handle.abort();
    }

    #[tokio::test]
    async fn test_gateway_listener_no_match_clears_pac() {
        let rules = vec![GatewayRule {
            default_gateway_subnet: "192.0.2.0/24".to_string(),
            pac_url: "http://test.pac".to_string(),
            when_match: None,
            when_no_match: None,
        }];

        let resolver = Arc::new(MockResolver::new());
        let listener = GatewayListener::new(rules, resolver.clone());

        let handle = listener.start().expect("Failed to start listener");

        tokio::time::sleep(Duration::from_millis(100)).await;

        let pac_url = resolver.get_last_pac_url().await;
        assert!(pac_url.is_none());

        handle.abort();
    }
}
