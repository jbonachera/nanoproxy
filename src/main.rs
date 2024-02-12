mod connection;
mod connector;
mod credentials;
mod pac;
mod resolver;
mod tracker;

use std::convert::Infallible;
use std::net::SocketAddr;

use connector::ProxyConnector;
use credentials::{CredentialProvider, ProxyAuthRule};
use headers::HeaderValue;
use hyper::service::{make_service_fn, service_fn};
use hyper::{http, Body, Client, Method, Request, Response, Server};

use resolver::{ProxyPACRule, ProxyResolver};
use serde::{Deserialize, Serialize};
use std::env;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tracker::ConnectionTracker;
use url::Url;
use uuid::Uuid;

use tokio::net::TcpStream;

use act_zero::runtimes::tokio::spawn_actor;
use act_zero::*;
use clap::Parser;
use tracing::{error, instrument};
use tracing_subscriber;

use crate::tracker::StreamInfo;

#[derive(Debug, Serialize, Deserialize)]
struct ProxyConfig {
    auth_rules: Vec<ProxyAuthRule>,
    pac_rules: Vec<ProxyPACRule>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            auth_rules: vec![ProxyAuthRule::default()],
            pac_rules: vec![ProxyPACRule::default()],
        }
    }
}

#[derive(Parser, Debug)]
#[clap(version = env!("CARGO_PKG_VERSION"), author = env!("CARGO_PKG_AUTHORS"))]
pub struct Opts {
    #[clap(long, short = 'p', default_value = "8888")]
    port: u16,
    #[clap(long, default_value = "false")]
    no_greeting: bool,
}

fn host_addr(uri: &http::Uri) -> Option<String> {
    uri.authority().and_then(|auth| Some(auth.to_string()))
}

fn remote_host(req: &Request<Body>) -> Url {
    if Method::CONNECT == req.method() {
        if let Some(addr) = host_addr(req.uri()) {
            return format!("https://{addr}/").parse().expect("valid host");
        }
    }
    return format!(
        "http://{}/",
        req.headers()
            .get("host")
            .expect("host header")
            .to_str()
            .expect("string")
    )
    .parse()
    .expect("valid host headers");
}

pub struct ClientSession {
    started_at: Instant,
    id: Uuid,
    resolver: Addr<ProxyResolver>,
    connection_tracker: Addr<ConnectionTracker>,
    credentials: Addr<CredentialProvider>,
}

impl std::fmt::Debug for ClientSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::write(f, format_args!("ClientSession"))
    }
}

impl Actor for ClientSession {}
impl ClientSession {
    fn new(
        connection_tracker: Addr<ConnectionTracker>,
        resolver: Addr<ProxyResolver>,
        credentials: Addr<CredentialProvider>,
    ) -> Self {
        ClientSession {
            id: Uuid::new_v4(),
            started_at: Instant::now(),
            resolver,
            connection_tracker,
            credentials,
        }
    }
}

impl ClientSession {
    #[instrument]
    async fn proxy(
        &mut self,
        client: Client<ProxyConnector>,
        req: Request<Body>,
    ) -> ActorResult<Response<Body>> {
        let remote = remote_host(&req);
        let upstream_url: Url = call!(self.resolver.resolve_proxy_for_url(remote))
            .await
            .unwrap();

        let remote_host_addr = host_addr(req.uri()).unwrap_or("_".to_string());
        call!(self.connection_tracker.push(StreamInfo {
            id: self.id,
            method: req.method().to_string(),
            remote: req.uri().to_string(),
            upstream: upstream_url.to_string(),
            opened_at: self.started_at,
            closed_at: None,
        }))
        .await
        .unwrap();
        let res = Produces::ok(match upstream_url.scheme() {
            "direct" => self.forward(client, req, remote_host_addr).await?,
            "http" => {
                self.forward_upstream(
                    client,
                    req,
                    call!(self
                        .credentials
                        .credentials_for(upstream_url.host_str().unwrap().to_string()))
                    .await
                    .unwrap(),
                )
                .await?
            }
            _ => panic!(),
        });

        res
    }
    #[instrument]
    async fn forward(
        &self,
        client: Client<ProxyConnector>,
        req: Request<Body>,
        remote_host: String,
    ) -> core::result::Result<Response<Body>, hyper::Error> {
        if Method::CONNECT == req.method() {
            let id = self.id.clone();
            let connection_tracker = self.connection_tracker.clone();

            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(mut upgraded) => {
                        let mut server = TcpStream::connect(remote_host)
                            .await
                            .expect("remote connection failed");
                        if let Err(_) = tunnel(&mut upgraded, &mut server).await {};
                    }
                    Err(_) => {}
                }
                call!(connection_tracker.remove(id)).await.unwrap();
            });

            Ok(Response::new(Body::empty()))
        } else {
            let resp = client.request(req).await;
            call!(self.connection_tracker.remove(self.id))
                .await
                .unwrap();

            resp
        }
    }
    #[instrument]
    async fn forward_upstream(
        &self,
        client: Client<ProxyConnector>,
        mut req: Request<Body>,
        creds: Option<String>,
    ) -> core::result::Result<Response<Body>, hyper::Error> {
        let creds = creds.unwrap_or("".to_string());
        if creds.len() > 0 {
            req.headers_mut().insert(
                http::header::PROXY_AUTHORIZATION,
                HeaderValue::from_bytes(creds.as_bytes()).expect("msg"),
            );
        }
        if Method::CONNECT == req.method() {
            let mut server_req = Request::connect(req.uri()).body(Body::empty()).unwrap();
            if creds.len() > 0 {
                server_req.headers_mut().insert(
                    http::header::PROXY_AUTHORIZATION,
                    HeaderValue::from_bytes(creds.as_bytes()).expect("msg"),
                );
            }
            let id = self.id.clone();
            let connection_tracker = self.connection_tracker.clone();

            tokio::task::spawn(async move {
                let res = client.request(server_req).await.expect("msg");
                match hyper::upgrade::on(res).await {
                    Ok(mut server) => match hyper::upgrade::on(req).await {
                        Ok(mut upgraded) => {
                            if let Err(e) = tunnel(&mut upgraded, &mut server).await {
                                error!("server io error: {}", e);
                            };
                        }
                        Err(e) => error!("upgrade error: {}", e),
                    },
                    Err(e) => error!("upgrade error: {}", e),
                }
                call!(connection_tracker.remove(id)).await.unwrap();
            });
            Ok(Response::new(Body::empty()))
        } else {
            let resp = client.request(req).await;
            call!(self.connection_tracker.remove(self.id))
                .await
                .unwrap();
            resp
        }
    }
}

async fn tunnel<A, B>(upgraded: &mut A, server: &mut B) -> std::io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin + ?Sized,
    B: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let (_, _) = tokio::io::copy_bidirectional(upgraded, server).await?;
    Ok(())
}

impl ProxyResolver {}

#[tokio::main]
async fn main() {
    let cfg = confy::load::<ProxyConfig>("nanoproxy", "nanoproxy").expect("failed to load config");
    tracing_subscriber::fmt::init();
    let args = Opts::parse();
    let listen_addr = SocketAddr::from(([127, 0, 0, 1], args.port));

    let credentials = spawn_actor(CredentialProvider::from_auth_rules(cfg.auth_rules));
    let connection_tracker = spawn_actor(ConnectionTracker::default());
    let resolver = spawn_actor(ProxyResolver::from_beacon_rules(cfg.pac_rules));

    let connector = ProxyConnector::from(resolver.clone());

    let client = Client::builder()
        .http1_title_case_headers(true)
        .http1_preserve_header_case(true)
        .build(connector);

    let make_service = make_service_fn(move |_| {
        let client = client.clone();
        let resolver = resolver.clone();
        let credentials = credentials.clone();
        let connection_tracker = connection_tracker.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let session = spawn_actor(ClientSession::new(
                    connection_tracker.clone(),
                    resolver.clone(),
                    credentials.clone(),
                ));
                call!(session.proxy(client.clone(), req))
            }))
        }
    });

    let server = Server::bind(&listen_addr)
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .serve(make_service);
    if !args.no_greeting {
        println!(
            "🚀 Nanoproxy server is running on http://{}:{}.",
            server.local_addr().ip(),
            server.local_addr().port()
        );
        println!(
            "Configuration loaded from {:#?}",
            confy::get_configuration_file_path("nanoproxy", "nanoproxy")
                .expect("failed to load config")
        );
        println!("");
        println!(
            "export http_proxy=http://{}:{};",
            server.local_addr().ip(),
            server.local_addr().port()
        );
        println!(
            "export https_proxy=http://{}:{};",
            server.local_addr().ip(),
            server.local_addr().port()
        );
        println!(
            "export all_proxy=http://{}:{};",
            server.local_addr().ip(),
            server.local_addr().port()
        );
        println!(
        "export no_proxy=localhost,127.0.0.0/8,*.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16;");
        println!("");
        println!("Connection logs will appear below.");
    }
    if let Err(e) = server.await {
        error!("server error: {}", e);
    }
}
