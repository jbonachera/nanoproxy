use crate::ports::ProxyResolverPort;
use futures::Future;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tower_service::Service;
use url::Url;

#[derive(Clone)]
pub struct HyperConnector {
    resolver: Arc<dyn ProxyResolverPort>,
}

impl HyperConnector {
    pub fn new(resolver: Arc<dyn ProxyResolverPort>) -> Self {
        Self { resolver }
    }

    const TIMEOUT_DURATION: Duration = Duration::from_millis(200);
}

impl Service<Uri> for HyperConnector {
    type Response = TokioIo<TcpStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let resolver = self.resolver.clone();

        Box::pin(async move {
            let authority = uri
                .authority()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Missing authority"))?;

            let url: Url = uri
                .to_string()
                .parse()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let routes = resolver
                .resolve_all_routes(&url)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            for route in routes {
                let addr = match route {
                    crate::domain::ProxyRoute::Direct => {
                        let port = authority.port_u16().unwrap_or(80);
                        format!("{}:{}", authority.host(), port)
                    }
                    crate::domain::ProxyRoute::Upstream { proxy_url, .. } => {
                        format!(
                            "{}:{}",
                            proxy_url.host_str().unwrap_or(""),
                            proxy_url.port().unwrap_or(80)
                        )
                    }
                    crate::domain::ProxyRoute::Blocked { reason } => {
                        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, reason));
                    }
                };

                // Try connecting with timeout
                match timeout(Self::TIMEOUT_DURATION, TcpStream::connect(&addr)).await {
                    Ok(Ok(stream)) => return Ok(TokioIo::new(stream)),
                    Ok(Err(e)) => {
                        log::debug!("Failed to connect to {}: {}", addr, e);
                        continue;
                    }
                    Err(_) => {
                        log::debug!("Timeout connecting to {}", addr);
                        continue;
                    }
                }
            }

            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "All proxy routes failed",
            ))
        })
    }
}
