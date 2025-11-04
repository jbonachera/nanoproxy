use crate::domain::{ProxyRoute, Result};
use async_trait::async_trait;
use url::Url;

/// Port for resolving which proxy to use for a given URL
#[async_trait]
pub trait ProxyResolverPort: Send + Sync {
    /// Resolve the proxy route for a given target URL
    async fn resolve_route(&self, target_url: &Url) -> Result<ProxyRoute>;

    /// Get all possible proxy routes for a URL (for failover)
    async fn resolve_all_routes(&self, target_url: &Url) -> Result<Vec<ProxyRoute>>;

    /// Update PAC configuration URL
    async fn update_pac_url(&self, pac_url: Option<String>) -> Result<()>;
}
