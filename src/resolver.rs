use async_trait::async_trait;
use reqwest::ClientBuilder;
use serde::{Deserialize, Serialize};
use std::{error, net::ToSocketAddrs, num::NonZeroUsize, time::Duration};
use tracing::info;
use url::Url;

use act_zero::{runtimes::tokio::Timer, timer::Tick, Actor, ActorResult, Addr, Produces, WeakAddr};
use lru::LruCache;

use crate::pac;

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyPACRule {
    beacon_host: String,
    pac_url: String,
}

impl Default for ProxyPACRule {
    fn default() -> Self {
        Self {
            beacon_host: "proxy.example.net:8080".into(),
            pac_url: "http://pac.example.net:8080/proxy.pac".into(),
        }
    }
}

pub struct ProxyResolver {
    pac_cache: LruCache<String, String>,
    pac_rules: Vec<ProxyPACRule>,
    pac_url: Option<String>,
    timer: Timer,
    addr: WeakAddr<Self>,
}

#[async_trait]
impl Actor for ProxyResolver {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        // Store our own address for later
        self.addr = addr.downgrade();

        self.timer
            .set_interval_weak(self.addr.clone(), Duration::from_secs(3));
        Produces::ok(())
    }
}
impl Default for ProxyResolver {
    fn default() -> Self {
        ProxyResolver {
            pac_cache: LruCache::new(NonZeroUsize::new(5).unwrap()),
            pac_rules: vec![],
            pac_url: None,
            addr: WeakAddr::default(),
            timer: Timer::default(),
        }
    }
}

#[async_trait]
impl Tick for ProxyResolver {
    async fn tick(&mut self) -> ActorResult<()> {
        if self.timer.tick() {
            return self.refresh_pac_url().await;
        }
        Produces::ok(())
    }
}

impl ProxyResolver {
    pub fn from_beacon_rules(rules: Vec<ProxyPACRule>) -> Self {
        let mut credential_provider = Self::default();
        credential_provider.pac_rules = rules;
        credential_provider
    }

    async fn load_pac(&mut self, pac_url: &str) -> Result<String, Box<dyn error::Error>> {
        info!("attempting to download PAC file at {pac_url}");
        let pac_file = ClientBuilder::new()
            .no_proxy()
            .build()?
            .get(pac_url)
            .send()
            .await?
            .text()
            .await?;
        self.pac_cache.put(pac_url.to_string(), pac_file.clone());
        info!("loaded PAC file ({}B)", pac_file.len());
        Ok(pac_file)
    }
    async fn resolve_proxy_from_pac(
        &mut self,
        pac_url: &str,
        url: &Url,
    ) -> Result<String, Box<dyn error::Error>> {
        let pac_file = match self.pac_cache.get(pac_url) {
            Some(v) => v.to_owned(),
            None => self.load_pac(pac_url).await?,
        };
        pac::proxy_for_url(pac_file, url)
    }

    fn select_pac_url(&self) -> Option<String> {
        for (_, rule) in self.pac_rules.iter().enumerate() {
            match rule.beacon_host.to_socket_addrs() {
                Ok(_) => return Some(rule.pac_url.clone()),
                Err(_) => continue,
            }
        }
        None
    }

    pub async fn refresh_pac_url(&mut self) -> ActorResult<()> {
        self.pac_url = self.select_pac_url();
        Produces::ok(())
    }

    pub async fn resolve_proxy_for_url(&mut self, url: Url) -> ActorResult<Url> {
        match self.pac_url.clone() {
            Some(pac_url) => {
                return Produces::ok(
                    self.resolve_proxy_from_pac(&pac_url, &url)
                        .await
                        .unwrap()
                        .parse()?,
                )
            }
            None => {}
        }

        Produces::ok("direct://".parse()?)
    }
    pub fn resolve_proxy_for_addr(upstream_url: Url) -> String {
        format!(
            "{}:{}",
            upstream_url.host().unwrap(),
            upstream_url.port().unwrap_or(80)
        )
        .to_string()
    }
}
