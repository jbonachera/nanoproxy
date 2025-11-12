#![cfg(test)]
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;

use nanoproxy::adapters::{
    ConnectionTracker, CredentialProvider, HyperConnector, HyperProxyAdapter, PacProxyResolver, ReqwestHttpClient,
};
use nanoproxy::domain::ProxyService;
use nanoproxy::ports::{CredentialsPort, ProxyResolverPort, TrackingPort};

pub struct TestNanoproxyServer {
    addr: SocketAddr,
    _server_handle: JoinHandle<()>,
}

impl TestNanoproxyServer {
    pub async fn start(port: u16, pac_file_path: Option<&std::path::Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;

        let resolver = Arc::new(PacProxyResolver::new());

        if let Some(pac_path) = pac_file_path {
            let pac_url = format!("file://{}", pac_path.display());
            resolver.update_pac_url(Some(pac_url)).await?;
        }

        let credentials: Arc<dyn CredentialsPort> = Arc::new(CredentialProvider::new(vec![]));
        let tracker = Arc::new(ConnectionTracker::new());
        let tracker_port: Arc<dyn TrackingPort> = tracker.clone();

        tracker.start_cleanup();

        let connector = HyperConnector::new(resolver.clone());
        let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .http1_title_case_headers(true)
            .http1_preserve_header_case(true)
            .build(connector.clone());

        let http_client = Arc::new(ReqwestHttpClient::new());

        let proxy_service = Arc::new(ProxyService::new(
            resolver.clone(),
            credentials.clone(),
            tracker_port.clone(),
            http_client,
        ));

        let adapter = Arc::new(HyperProxyAdapter::new(proxy_service, client));

        let server_handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);
                        let adapter = adapter.clone();

                        tokio::spawn(async move {
                            let service_fn = service_fn(move |req| {
                                let adapter = adapter.clone();
                                async move { Ok::<_, hyper::Error>(adapter.handle(req).await) }
                            });

                            if let Err(_err) = ServerBuilder::new(hyper_util::rt::TokioExecutor::new())
                                .http1()
                                .preserve_header_case(true)
                                .title_case_headers(true)
                                .serve_connection_with_upgrades(io, service_fn)
                                .await
                            {
                                // Silently handle errors in test
                            }
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            addr,
            _server_handle: server_handle,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}
