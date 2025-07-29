use async_trait::async_trait;
use ipnet::Ipv4Net;
use log::{debug, warn};
use reqwest::ClientBuilder;
use resolv_conf::ScopedIp;
use serde::{Deserialize, Serialize};
use std::{error, net::ToSocketAddrs, num::NonZeroUsize, time::Duration};
use tracing::info;
use url::Url;

use act_zero::{call, runtimes::tokio::Timer, timer::Tick, Actor, ActorResult, Addr, Produces, WeakAddr};
use lru::LruCache;

use crate::pac;
use notify_debouncer_mini::new_debouncer;
use std::io::Read;


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
    pac_url: Option<String>,
    addr: WeakAddr<Self>,
}

#[async_trait]
impl Actor for ProxyResolver {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        // Store our own address for later
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}
impl Default for ProxyResolver {
    fn default() -> Self {
        ProxyResolver {
            pac_cache: LruCache::new(NonZeroUsize::new(5).unwrap()),
            pac_url: None,
            addr: WeakAddr::default(),
        }
    }
}


impl ProxyResolver {
    async fn load_pac(&mut self, pac_url: &str) -> Result<String, Box<dyn error::Error>> {
        debug!("attempting to download PAC file at {pac_url}");
        let pac_file = ClientBuilder::new()
            .no_proxy()
            .build()?
            .get(pac_url)
            .send()
            .await?
            .text()
            .await?;
        self.pac_cache.put(pac_url.to_string(), pac_file.clone());
        info!("loaded PAC file from {} ({} bytes)",pac_url, pac_file.len());
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
        let proxies = pac::proxy_for_url(pac_file, url)?;
        if proxies.is_empty() {
            Ok("direct://".to_owned())
        } else {
            Ok(proxies[0].clone())
        }
    }
    
    pub async fn resolve_all_proxies_for_url(
        &mut self,
        url: &Url,
    ) -> Result<Vec<String>, Box<dyn error::Error>> {
        match self.pac_url.clone() {
            Some(pac_url) => {
                let pac_file = match self.pac_cache.get(&pac_url) {
                    Some(v) => v.to_owned(),
                    None => self.load_pac(&pac_url).await?,
                };
                pac::proxy_for_url(pac_file, url)
            }
            None => Ok(vec!["direct://".to_owned()]),
        }
    }

    async fn load_pac_url(&mut self, url: Option<String>) -> ActorResult<()> {
        self.pac_url = url;
        self.pac_cache.clear();
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
    
    pub async fn get_all_proxies_for_url(&mut self, url: Url) -> ActorResult<Vec<Url>> {
        match self.pac_url.clone() {
            Some(pac_url) => {
                let proxies = self.resolve_all_proxies_for_url(&url).await.unwrap_or_else(|_| vec!["direct://".to_owned()]);
                let proxy_urls: Vec<Url> = proxies.into_iter()
                    .filter_map(|p| p.parse().ok())
                    .collect();
                
                if proxy_urls.is_empty() {
                    Produces::ok(vec!["direct://".parse()?])
                } else {
                    Produces::ok(proxy_urls)
                }
            }
            None => {
                Produces::ok(vec!["direct://".parse()?])
            }
        }
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


pub struct BeaconPoller {
    pac_rules: Vec<ProxyPACRule>,
    timer: Timer,
    addr: WeakAddr<Self>,
    resolver: Addr<ProxyResolver>
}

#[async_trait]
impl Actor for BeaconPoller {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();

        self.timer
            .set_interval_weak(self.addr.clone(), Duration::from_secs(3));
        Produces::ok(())
    }
}


impl Default for BeaconPoller {
    fn default() -> Self {
        BeaconPoller {
            pac_rules: vec![],
            addr: WeakAddr::default(),
            resolver: Addr::default(),
            timer: Timer::default(),
        }
    }
}

#[async_trait]
impl Tick for BeaconPoller {
    async fn tick(&mut self) -> ActorResult<()> {
        if self.timer.tick() {
            return self.refresh_pac_url().await;
        }
        Produces::ok(())
    }
}

impl BeaconPoller {
    pub fn from_beacon_rules(rules: Vec<ProxyPACRule>, resolver: Addr<ProxyResolver>) -> Self {
        let mut s = Self::default();
        s.pac_rules = rules;
        s.resolver = resolver;
        s
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
        call!(self.resolver.load_pac_url(self.select_pac_url())).await?;
        Produces::ok(())
    }

}



pub struct ResolvConfListener {
    resolvconf_rules: Vec<ResolvConfRule>,
    addr: WeakAddr<Self>,
    resolver: Addr<ProxyResolver>
}

#[async_trait]
impl Actor for ResolvConfListener {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.refresh_rules().await?;
        tokio::spawn(async move {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut debouncer = new_debouncer(Duration::from_secs(1), tx).unwrap();
            debouncer
            .watcher()
            .watch(std::path::Path::new("/etc/resolv.conf"), notify::RecursiveMode::NonRecursive)
            .unwrap();

            for result in rx {
                match result {
                    Ok(_) => {
                      match call!(addr.refresh_rules()).await {
                        Ok(_) => {},
                        Err(error) => log::info!("Failed to parse resolv.conf {error:?}"),
                      }
                    }
                    Err(error) => log::info!("Error {error:?}"),
                }
            }


        });
        Produces::ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolvConfRule {
    resolver_subnet: String,
    pac_url: String,
    when_match: Option<String>,
    when_no_match: Option<String>,
}

impl Default for ResolvConfRule {
    fn default() -> Self {
        Self {
            resolver_subnet: "10.0.0.0/24".into(),
            pac_url: "http://pac.example.net:8080/proxy.pac".into(),
            when_match: Some("echo ok".into()),
            when_no_match: Some("echo ko".into()),
        }
    }
}

impl ResolvConfListener {
    async fn refresh_rules(&self) -> ActorResult<()> {
        let mut buf = Vec::with_capacity(4096);
        let mut f = std::fs::File::open("/etc/resolv.conf").unwrap();
        f.read_to_end(&mut buf).unwrap();

        let cfg: resolv_conf::Config = resolv_conf::Config::parse(&buf).unwrap();
        let mut matched = false;
        for ip in cfg.get_nameservers_or_local() {
            if !matched {
                match ip {
                    ScopedIp::V4(ip) => {
                        for (_, rule) in self.resolvconf_rules.iter().enumerate() {
                            let net: Ipv4Net = rule.resolver_subnet.parse().unwrap();
                            if net.contains(&ip) {
                                call!(self.resolver.load_pac_url(Some(rule.pac_url.clone()))).await?;
                                matched = true;
                                match &rule.when_match {
                                    Some(v) => {
                                    warn!("running command {}", v);
                                    std::process::Command::new("sh")
                                        .arg("-c")
                                        .arg(&v)
                                        .output()
                                        .expect("when_match command failed");
                                    },
                                    None => {}
                                }
                            } else {
                                match &rule.when_no_match {
                                    Some(v) => {
                                    warn!("running command {}", v);
                                    std::process::Command::new("sh")
                                        .arg("-c")
                                        .arg(&v)
                                        .output()
                                        .expect("when_no_match command failed");
                                    },
                                    None => {}
                                }
                            }
                        }
                    }
                    ScopedIp::V6(ip, _scope) => {
                        println!("ignoring ipv6 {} in resolvconf", ip)
                    }
                }
            }
        }
        if !matched {
            call!(self.resolver.load_pac_url(None)).await?;
        }
        Produces::ok(())
    }
    pub fn from_rules(rules: Vec<ResolvConfRule>, resolver: Addr<ProxyResolver>) -> Self {
        let mut s = Self::default();
        s.resolvconf_rules = rules;
        s.resolver = resolver;
        s
    }
}
impl Default for ResolvConfListener {
    fn default() -> Self {
        ResolvConfListener {
            resolvconf_rules: vec![],
            addr: WeakAddr::default(),
            resolver: Addr::default(),
        }
    }
}

