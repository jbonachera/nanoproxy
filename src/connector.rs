use crate::connection::ProxyConnection;
use crate::resolver::ProxyResolver;
use act_zero::{call, Addr};
use futures::Future;
use std::pin::Pin;
use std::task::{self, Poll};

use hyper::service::Service;
use hyper::Uri;
use url::Url;

use tokio::net::TcpStream;

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
            let upstream_url: Url =
                call!(resolver.resolve_proxy_for_url(uri.to_string().parse().unwrap()))
                    .await
                    .unwrap();
            let aut = uri.authority().unwrap();
            match upstream_url.scheme() {
                "direct" => Ok(TcpStream::connect(format!(
                    "{}:{}",
                    aut.host(),
                    aut.port_u16().unwrap_or(80)
                ))
                .await?
                .into()),
                "http" => Ok(TcpStream::connect(ProxyResolver::resolve_proxy_for_addr(
                    upstream_url,
                ))
                .await?
                .into()),
                _ => panic!(),
            }
        })
    }
}
