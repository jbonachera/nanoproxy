use super::pac_evaluator::{evaluate_pac, parse_proxy_route};
use crate::domain::{ProxyError, ProxyRoute, Result};
use crate::ports::ProxyResolverPort;
use async_trait::async_trait;
use log::debug;
use lru::LruCache;
use reqwest::ClientBuilder;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use url::Url;

/// Actor-based PAC resolver implementation
pub struct PacProxyResolver {
    pac_cache: Arc<RwLock<LruCache<String, String>>>,
    pac_url: Arc<RwLock<Option<String>>>,
}

impl PacProxyResolver {
    pub fn new() -> Self {
        Self {
            pac_cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(5).unwrap()))),
            pac_url: Arc::new(RwLock::new(None)),
        }
    }

    async fn load_pac(&self, pac_url: &str) -> Result<String> {
        debug!("Attempting to download PAC file at {}", pac_url);

        let pac_file = ClientBuilder::new()
            .no_proxy()
            .build()
            .map_err(|e| ProxyError::ResolutionFailed(format!("HTTP client error: {}", e)))?
            .get(pac_url)
            .send()
            .await
            .map_err(|e| ProxyError::ResolutionFailed(format!("PAC download error: {}", e)))?
            .text()
            .await
            .map_err(|e| ProxyError::ResolutionFailed(format!("PAC read error: {}", e)))?;

        let mut cache = self.pac_cache.write().await;
        cache.put(pac_url.to_string(), pac_file.clone());

        info!("Loaded PAC file from {} ({} bytes)", pac_url, pac_file.len());
        Ok(pac_file)
    }

    async fn get_pac_file(&self, pac_url: &str) -> Result<String> {
        // Try cache first
        {
            let mut cache = self.pac_cache.write().await;
            if let Some(cached) = cache.get(pac_url) {
                return Ok(cached.clone());
            }
        }

        // Load from network
        self.load_pac(pac_url).await
    }

    async fn resolve_with_pac(&self, pac_url: &str, target_url: &Url) -> Result<Vec<ProxyRoute>> {
        let pac_file = self.get_pac_file(pac_url).await?;
        let proxy_urls = evaluate_pac(&pac_file, target_url)?;

        proxy_urls.into_iter().map(|url| parse_proxy_route(&url)).collect()
    }
}

#[async_trait]
impl ProxyResolverPort for PacProxyResolver {
    async fn resolve_route(&self, target_url: &Url) -> Result<ProxyRoute> {
        let pac_url = self.pac_url.read().await;

        match pac_url.as_ref() {
            Some(url) => {
                let routes = self.resolve_with_pac(url, target_url).await?;
                routes
                    .into_iter()
                    .next()
                    .ok_or_else(|| ProxyError::ResolutionFailed("No routes found".into()))
            }
            None => Ok(ProxyRoute::Direct),
        }
    }

    async fn resolve_all_routes(&self, target_url: &Url) -> Result<Vec<ProxyRoute>> {
        let pac_url = self.pac_url.read().await;

        match pac_url.as_ref() {
            Some(url) => self.resolve_with_pac(url, target_url).await,
            None => Ok(vec![ProxyRoute::Direct]),
        }
    }

    async fn update_pac_url(&self, pac_url: Option<String>) -> Result<()> {
        let mut url_guard = self.pac_url.write().await;
        *url_guard = pac_url;

        // Clear cache when PAC URL changes
        let mut cache = self.pac_cache.write().await;
        cache.clear();

        Ok(())
    }
}

impl Default for PacProxyResolver {
    fn default() -> Self {
        Self::new()
    }
}
