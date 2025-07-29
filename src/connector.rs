use crate::connection::ProxyConnection;
use crate::resolver::ProxyResolver;
use act_zero::{call, Addr};
use futures::Future;
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
    type Future =
        Pin<Box<dyn Future<Output = core::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _: &mut task::Context<'_>,
    ) -> Poll<core::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let url = uri.to_string().parse().unwrap();
            let proxy_urls: Vec<Url> = call!(resolver.get_all_proxies_for_url(url))
                .await
                .unwrap_or_else(|_| vec!["direct://".parse().unwrap()]);
            
            let aut = uri.authority().unwrap();
            
            // If we have at least one proxy, try it with timeout
            if !proxy_urls.is_empty() {
                let first_proxy_url = &proxy_urls[0];
                
                match first_proxy_url.scheme() {
                    "direct" => {
                        let addr = format!(
                            "{}:{}",
                            aut.host(),
                            aut.port_u16().unwrap_or(80)
                        );
                        return Ok(TcpStream::connect(addr)
                            .await?
                            .into())
                            .map(|v: ProxyConnection<TcpStream>| v.into_direct());
                    },
                    "http" => {
                        let addr = ProxyResolver::resolve_proxy_for_addr(first_proxy_url.clone());
                        
                        // Try to connect to the first proxy with a 500ms timeout
                        match timeout(Duration::from_millis(80), TcpStream::connect(&addr)).await {
                            Ok(result) => {
                                return match result {
                                    Ok(stream) => Ok(stream.into())
                                        .map(|v: ProxyConnection<TcpStream>| v.into_proxy()),
                                    Err(e) => Err(e),
                                };
                            },
                            Err(_) => {
                                log::info!("Proxy timeout, falling back to second proxy");
                                // Timeout occurred, try the second proxy if available
                                if proxy_urls.len() > 1 {
                                    let second_proxy_url = &proxy_urls[1];
                                    if second_proxy_url.scheme() == "http" {
                                        let second_addr = ProxyResolver::resolve_proxy_for_addr(second_proxy_url.clone());
                                        return Ok(TcpStream::connect(second_addr)
                                            .await?
                                            .into())
                                            .map(|v: ProxyConnection<TcpStream>| v.into_proxy());
                                    } else if second_proxy_url.scheme() == "direct" {
                                        let addr = format!(
                                            "{}:{}",
                                            aut.host(),
                                            aut.port_u16().unwrap_or(80)
                                        );
                                        return Ok(TcpStream::connect(addr)
                                            .await?
                                            .into())
                                            .map(|v: ProxyConnection<TcpStream>| v.into_direct());
                                    }
                                }
                                
                                // If no second proxy or second proxy failed, try the first proxy again without timeout
                                return Ok(TcpStream::connect(addr)
                                    .await?
                                    .into())
                                    .map(|v: ProxyConnection<TcpStream>| v.into_proxy());
                            }
                        }
                    },
                    _ => panic!(),
                }
            }
            
            // Default to direct connection if no proxies
            let addr = format!(
                "{}:{}",
                aut.host(),
                aut.port_u16().unwrap_or(80)
            );
            Ok(TcpStream::connect(addr)
                .await?
                .into())
                .map(|v: ProxyConnection<TcpStream>| v.into_direct())
        })
    }
}
