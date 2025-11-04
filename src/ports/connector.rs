use crate::domain::{ProxyRoute, Result};
use async_trait::async_trait;

/// Port for establishing network connections
#[async_trait]
#[allow(dead_code)] // Prepared for testing and alternative implementations
pub trait ConnectorPort: Send + Sync {
    /// Connect to a target host through the specified route
    ///
    /// Returns a connection handle that can be used for communication
    async fn connect(&self, target: &str, route: &ProxyRoute) -> Result<Box<dyn Connection>>;
}

/// Trait representing an established connection
#[async_trait]
#[allow(dead_code)] // Prepared for testing and alternative implementations
pub trait Connection: Send + Sync {
    /// Check if connection is still alive
    fn is_connected(&self) -> bool;

    /// Get connection metadata
    fn metadata(&self) -> ConnectionMetadata;
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Prepared for future use
pub struct ConnectionMetadata {
    pub is_proxy: bool,
    pub target_addr: String,
}
