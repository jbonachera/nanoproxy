use crate::domain::{ConnectionInfo, Result};
use async_trait::async_trait;
use uuid::Uuid;

/// Port for tracking active connections
#[async_trait]
pub trait TrackingPort: Send + Sync {
    /// Register a new connection
    async fn track_connection(&self, info: ConnectionInfo) -> Result<()>;

    /// Mark a connection as closed
    async fn close_connection(&self, id: Uuid) -> Result<()>;

    /// Get all active connections
    #[allow(dead_code)] // Utility method for future use (monitoring dashboard)
    async fn get_active_connections(&self) -> Result<Vec<ConnectionInfo>>;
}
