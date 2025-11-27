use crate::domain::{GatewayRule, ProxyError, Result};
use crate::ports::ProxyResolverPort;
use ipnet::Ipv4Net;
use log::{debug, warn};
use std::sync::Arc;
use std::time::Duration;
use wildmatch::WildMatch;

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
            let mut last_pac_url: Option<String> = None;

            if let Err(e) = Self::refresh_rules_static(&rules_clone, &resolver_clone, &mut last_pac_url).await {
                log::info!("Failed initial gateway refresh: {:?}", e);
            }

            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;

                if let Err(e) = Self::refresh_rules_static(&rules_clone, &resolver_clone, &mut last_pac_url).await {
                    log::info!("Failed to detect gateway: {:?}", e);
                }
            }
        }))
    }

    async fn refresh_rules_static(
        rules: &[GatewayRule],
        resolver: &Arc<dyn ProxyResolverPort>,
        last_pac_url: &mut Option<String>,
    ) -> Result<()> {
        let interface = netdev::get_default_interface()
            .map_err(|e| ProxyError::ResolutionFailed(format!("Cannot get default interface: {}", e)))?;

        let interface_name = &interface.name;

        debug!("Default gateway interface: {}", interface_name);
        debug!("Interface IPv4 addresses: {:?}", interface.ipv4);

        let mut matched = false;
        let mut new_pac_url: Option<String> = None;
        let mut matched_rule_index: Option<usize> = None;

        for (index, rule) in rules.iter().enumerate() {
            debug!("Checking rule #{}: {:?}", index, rule);
            let pattern = WildMatch::new(&rule.default_route_interface);

            let interface_matches = pattern.matches(interface_name);
            debug!(
                "  Interface match: pattern='{}' vs interface='{}' => {}",
                rule.default_route_interface, interface_name, interface_matches
            );

            let ip_matches = if let Some(subnet_str) = &rule.interface_ip_subnet {
                let subnet: Ipv4Net = subnet_str
                    .parse()
                    .map_err(|e| ProxyError::ResolutionFailed(format!("Invalid IP subnet: {}", e)))?;

                debug!("  Checking IP subnet: {}", subnet_str);
                let matches = interface.ipv4.iter().any(|ip_net| {
                    let contains = subnet.contains(&ip_net.addr());
                    debug!("    IP {} in subnet {}? {}", ip_net.addr(), subnet, contains);
                    contains
                });
                debug!("  IP subnet match result: {}", matches);
                matches
            } else {
                debug!("  No IP subnet specified, skipping IP check");
                true
            };

            if interface_matches && ip_matches {
                debug!("✓ Rule #{} matched!", index);
                matched = true;
                new_pac_url = Some(rule.pac_url.clone());
                matched_rule_index = Some(index);
                break;
            } else {
                debug!(
                    "✗ Rule #{} did NOT match (interface_matches={}, ip_matches={})",
                    index, interface_matches, ip_matches
                );
            }
        }

        if !matched {
            new_pac_url = None;
        }

        if new_pac_url != *last_pac_url {
            debug!("Network changed: {:?} -> {:?}", last_pac_url, new_pac_url);

            if let Some(url) = &new_pac_url {
                debug!("Setting PAC URL: {}", url);
            } else {
                debug!("Clearing PAC URL");
            }

            resolver.update_pac_url(new_pac_url.clone()).await?;

            if let Some(index) = matched_rule_index {
                if let Some(cmd) = &rules[index].when_match {
                    warn!("Running when_match command: {}", cmd);
                    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).output();
                }
            } else {
                for rule in rules {
                    if let Some(cmd) = &rule.when_no_match {
                        warn!("Running when_no_match command: {}", cmd);
                        let _ = std::process::Command::new("sh").arg("-c").arg(cmd).output();
                    }
                }
            }

            *last_pac_url = new_pac_url;
        } else {
            debug!("Network unchanged, skipping PAC update");
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
        pub update_count: AtomicUsize,
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
            default_route_interface: "*".to_string(),
            interface_ip_subnet: None,
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
            default_route_interface: "nonexistent_interface_xyz".to_string(),
            interface_ip_subnet: None,
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

    #[tokio::test]
    async fn test_gateway_listener_with_ip_subnet_match() {
        let interface = netdev::get_default_interface().expect("No default interface");

        if interface.ipv4.is_empty() {
            return;
        }

        let first_ip = interface.ipv4[0];
        let subnet_str = first_ip.to_string();

        let rules = vec![GatewayRule {
            default_route_interface: "*".to_string(),
            interface_ip_subnet: Some(subnet_str),
            pac_url: "http://test-ip.pac".to_string(),
            when_match: None,
            when_no_match: None,
        }];

        let resolver = Arc::new(MockResolver::new());
        let listener = GatewayListener::new(rules, resolver.clone());

        let handle = listener.start().expect("Failed to start listener");

        tokio::time::sleep(Duration::from_millis(100)).await;

        let pac_url = resolver.get_last_pac_url().await;
        assert!(pac_url.is_some());
        assert_eq!(pac_url.unwrap(), "http://test-ip.pac");

        handle.abort();
    }

    #[tokio::test]
    async fn test_gateway_listener_with_ip_subnet_no_match() {
        let rules = vec![GatewayRule {
            default_route_interface: "*".to_string(),
            interface_ip_subnet: Some("1.2.3.0/24".to_string()),
            pac_url: "http://test-ip.pac".to_string(),
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

    #[tokio::test]
    async fn test_gateway_listener_without_ip_subnet_backwards_compat() {
        let rules = vec![GatewayRule {
            default_route_interface: "*".to_string(),
            interface_ip_subnet: None,
            pac_url: "http://test-compat.pac".to_string(),
            when_match: None,
            when_no_match: None,
        }];

        let resolver = Arc::new(MockResolver::new());
        let listener = GatewayListener::new(rules, resolver.clone());

        let handle = listener.start().expect("Failed to start listener");

        tokio::time::sleep(Duration::from_millis(100)).await;

        let pac_url = resolver.get_last_pac_url().await;
        assert!(pac_url.is_some());
        assert_eq!(pac_url.unwrap(), "http://test-compat.pac");

        handle.abort();
    }

    #[tokio::test]
    async fn test_gateway_listener_skips_update_when_network_unchanged() {
        let rules = vec![GatewayRule {
            default_route_interface: "*".to_string(),
            interface_ip_subnet: None,
            pac_url: "http://test.pac".to_string(),
            when_match: None,
            when_no_match: None,
        }];

        let resolver = Arc::new(MockResolver::new());
        let listener = GatewayListener::new(rules, resolver.clone());

        let handle = listener.start().expect("Failed to start listener");

        tokio::time::sleep(Duration::from_millis(100)).await;
        let initial_count = resolver.update_count.load(Ordering::SeqCst);
        assert_eq!(initial_count, 1);

        tokio::time::sleep(Duration::from_secs(6)).await;
        let final_count = resolver.update_count.load(Ordering::SeqCst);
        assert_eq!(final_count, 1);

        handle.abort();
    }
}
