use crate::domain::{Credentials, Result};
use async_trait::async_trait;

/// Port for managing authentication credentials
#[async_trait]
pub trait CredentialsPort: Send + Sync {
    /// Get credentials for a specific host
    ///
    /// Returns None if no credentials are configured for this host
    async fn get_credentials(&self, host: &str) -> Result<Option<Credentials>>;

    /// Clear credentials cache (if any)
    #[allow(dead_code)] // Utility method for future use
    async fn clear_cache(&self) -> Result<()>;
}
