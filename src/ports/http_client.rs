use crate::domain::{Credentials, ProxyRequest, ProxyResponse, ProxyRoute, Result};
use async_trait::async_trait;

#[async_trait]
pub trait HttpClientPort: Send + Sync {
    async fn execute(
        &self,
        request: &ProxyRequest,
        route: &ProxyRoute,
        credentials: Option<&Credentials>,
    ) -> Result<ProxyResponse>;
}
