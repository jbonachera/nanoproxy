use crate::connection::ProxyConnection;
use crate::resolver::ProxyResolver;
use act_zero::{call, Addr};
use futures::Future;
use std::io::Error;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::Duration;

use hyper::service::Service;
use hyper::Uri;
use url::Url;

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
    type Response = ProxyConnection<TcpStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = core::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut task::Context<'_>) -> Poll<core::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let url = uri.to_string().parse().unwrap();
            let proxy_urls: Vec<Url> = call!(resolver.get_all_proxies_for_url(url))
                .await
                .unwrap_or_else(|_| vec!["direct://".parse().unwrap()]);

            let authority = uri.authority().unwrap();

            // If we have at least one proxy, try it with timeout
            if !proxy_urls.is_empty() {
                let first_proxy_url = &proxy_urls[0];

                match first_proxy_url.scheme() {
                    "direct" => {
                        let addr = format!("{}:{}", authority.host(), authority.port_u16().unwrap_or(80));
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
                                        return Self::connect_with_timeout_through_proxy(&format!(
                                            "{}:{}",
                                            authority.host(),
                                            authority.port_u16().unwrap_or(80)
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
            return Self::connect_with_timeout_direct(&format!(
                "{}:{}",
                authority.host(),
                authority.port_u16().unwrap_or(80)
            ))
            .await;
        })
    }
}

impl ProxyConnector {
    const TIMEOUT_DURATION: Duration = Duration::from_millis(200);

    async fn connect_with_timeout_through_proxy(addr: &String) -> Result<ProxyConnection<TcpStream>, Error> {
        timeout(Self::TIMEOUT_DURATION, TcpStream::connect(&addr))
            .await?
            .map(|stream| stream.into())
            .map(|v: ProxyConnection<TcpStream>| v.into_proxy())
    }

    async fn connect_with_timeout_direct(addr: &String) -> Result<ProxyConnection<TcpStream>, Error> {
        timeout(Self::TIMEOUT_DURATION, TcpStream::connect(&addr))
            .await?
            .map(|stream| stream.into())
            .map(|v: ProxyConnection<TcpStream>| v.into_direct())
    }
}
