use crate::domain::{ProxyError, ResolvConfRule, Result};
use crate::ports::ProxyResolverPort;
use ipnet::Ipv4Net;
use log::warn;
use notify_debouncer_mini::new_debouncer;
use resolv_conf::ScopedIp;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

/// Listener for /etc/resolv.conf changes
pub struct ResolvConfListener {
    rules: Vec<ResolvConfRule>,
    resolver: Arc<dyn ProxyResolverPort>,
}

impl ResolvConfListener {
    pub fn new(rules: Vec<ResolvConfRule>, resolver: Arc<dyn ProxyResolverPort>) -> Self {
        Self { rules, resolver }
    }

    /// Start listening to resolv.conf changes
    pub fn start(self) -> Result<tokio::task::JoinHandle<()>> {
        // Initial refresh
        let resolver_clone = self.resolver.clone();
        let rules_clone = self.rules.clone();

        tokio::spawn(async move {
            // Do initial refresh
            if let Err(e) = Self::refresh_rules_static(&rules_clone, &resolver_clone).await {
                log::error!("Failed initial resolv.conf refresh: {}", e);
            }

            // Set up file watcher
            let (tx, rx) = std::sync::mpsc::channel();
            let mut debouncer = match new_debouncer(Duration::from_secs(1), tx) {
                Ok(d) => d,
                Err(e) => {
                    log::error!("Failed to create debouncer: {}", e);
                    return;
                }
            };

            if let Err(e) = debouncer.watcher().watch(
                std::path::Path::new("/etc/resolv.conf"),
                notify_debouncer_mini::notify::RecursiveMode::NonRecursive,
            ) {
                log::error!("Failed to watch /etc/resolv.conf: {}", e);
                return;
            }

            for result in rx {
                match result {
                    Ok(_) => {
                        if let Err(e) = Self::refresh_rules_static(&rules_clone, &resolver_clone).await {
                            log::info!("Failed to parse resolv.conf: {:?}", e);
                        }
                    }
                    Err(error) => log::info!("File watch error: {:?}", error),
                }
            }
        });

        Ok(tokio::spawn(async {}))
    }

    async fn refresh_rules_static(rules: &[ResolvConfRule], resolver: &Arc<dyn ProxyResolverPort>) -> Result<()> {
        let mut buf = Vec::with_capacity(4096);
        let mut f = std::fs::File::open("/etc/resolv.conf")
            .map_err(|e| ProxyError::ResolutionFailed(format!("Cannot open resolv.conf: {}", e)))?;
        f.read_to_end(&mut buf)
            .map_err(|e| ProxyError::ResolutionFailed(format!("Cannot read resolv.conf: {}", e)))?;

        let cfg = resolv_conf::Config::parse(&buf)
            .map_err(|e| ProxyError::ResolutionFailed(format!("Cannot parse resolv.conf: {}", e)))?;

        let mut matched = false;
        for ip in cfg.get_nameservers_or_local() {
            if !matched {
                if let ScopedIp::V4(ip) = ip {
                    for rule in rules {
                        let net: Ipv4Net = rule
                            .resolver_subnet
                            .parse()
                            .map_err(|e| ProxyError::ResolutionFailed(format!("Invalid subnet: {}", e)))?;

                        if net.contains(&ip) {
                            resolver.update_pac_url(Some(rule.pac_url.clone())).await?;
                            matched = true;

                            if let Some(cmd) = &rule.when_match {
                                warn!("Running command: {}", cmd);
                                let _ = std::process::Command::new("sh").arg("-c").arg(cmd).output();
                            }
                        } else if let Some(cmd) = &rule.when_no_match {
                            warn!("Running command: {}", cmd);
                            let _ = std::process::Command::new("sh").arg("-c").arg(cmd).output();
                        }
                    }
                }
            }
        }

        if !matched {
            resolver.update_pac_url(None).await?;
        }

        Ok(())
    }
}
