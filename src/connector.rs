use crate::connection::ProxyConnection;
use crate::resolver::ProxyResolver;
use act_zero::{call, Addr};
use futures::Future;
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::Uri;
use tower_service::Service;
use url::Url;

use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Clone)]
pub struct ProxyConnector {
    resolver: Addr<ProxyResolver>,
}

impl From<Addr<ProxyResolver>> for ProxyConnector {
    fn from(resolver: Addr<ProxyResolver>) -> Self {
        ProxyConnector { resolver }
    }
}

impl Service<Uri> for ProxyConnector {
    type Response = ProxyConnection<TokioIo<TcpStream>>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let url = match uri.to_string().parse() {
                Ok(url) => url,
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Invalid URI: {}", e),
                    ));
                }
            };
            let proxy_urls: Vec<Url> = call!(resolver.get_all_proxies_for_url(url))
                .await
                .unwrap_or_else(|_| vec!["direct://".parse().unwrap()]);

            let authority = match uri.authority() {
                Some(auth) => auth,
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "URI missing authority",
                    ));
                }
            };

            // If we have at least one proxy, try it with timeout
            if !proxy_urls.is_empty() {
                let first_proxy_url = &proxy_urls[0];

                match first_proxy_url.scheme() {
                    "direct" => {
                        let port = authority.port_u16().unwrap_or(80);
                        let addr = format!("{}:{}", authority.host(), port);
                        return Self::connect_with_timeout_direct(&addr).await;
                    }
                    "http" => {
                        let addr = ProxyResolver::resolve_proxy_for_addr(first_proxy_url.clone());

                        return match Self::connect_with_timeout_through_proxy(&addr).await {
                            result @ Ok(_) => result,
                            Err(_) => {
                                // Timeout occurred, try the second upstream if available
                                if proxy_urls.len() > 1 {
                                    let second_proxy_url = &proxy_urls[1];
                                    log::info!("Proxy timeout, falling back to proxy {second_proxy_url}");
                                    if second_proxy_url.scheme() == "http" {
                                        return Self::connect_with_timeout_through_proxy(
                                            &ProxyResolver::resolve_proxy_for_addr(second_proxy_url.clone()),
                                        )
                                        .await;
                                    } else if second_proxy_url.scheme() == "direct" {
                                        let port = authority.port_u16().unwrap_or(80);
                                        return Self::connect_with_timeout_through_proxy(&format!(
                                            "{}:{}",
                                            authority.host(),
                                            port
                                        ))
                                        .await;
                                    }
                                }

                                // If no second upstream, fallback on the first proxy
                                return Self::connect_with_timeout_through_proxy(&addr).await;
                            }
                        };
                    }
                    _ => panic!(),
                }
            }

            // Default to direct connection if no proxies
            let port = authority.port_u16().unwrap_or(80);
            return Self::connect_with_timeout_direct(&format!("{}:{}", authority.host(), port)).await;
        })
    }
}

impl ProxyConnector {
    const TIMEOUT_DURATION: Duration = Duration::from_millis(200);

    async fn connect_with_timeout_through_proxy(addr: &String) -> Result<ProxyConnection<TokioIo<TcpStream>>, Error> {
        timeout(Self::TIMEOUT_DURATION, TcpStream::connect(&addr))
            .await?
            .map(|stream| TokioIo::new(stream).into())
            .map(|v: ProxyConnection<TokioIo<TcpStream>>| v.into_proxy())
    }

    async fn connect_with_timeout_direct(addr: &String) -> Result<ProxyConnection<TokioIo<TcpStream>>, Error> {
        timeout(Self::TIMEOUT_DURATION, TcpStream::connect(&addr))
            .await?
            .map(|stream| TokioIo::new(stream).into())
            .map(|v: ProxyConnection<TokioIo<TcpStream>>| v.into_direct())
    }
}
