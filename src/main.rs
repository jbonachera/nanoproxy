mod connection;
mod connector;
mod credentials;
mod pac;
mod resolver;

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
use url::Url;

use tokio::net::TcpStream;

use act_zero::runtimes::tokio::spawn_actor;
use act_zero::*;
use clap::Parser;
use tracing::{error, info, instrument};
use tracing_subscriber;

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
    resolver: Addr<ProxyResolver>,
    credentials: Addr<CredentialProvider>,
}

impl std::fmt::Debug for ClientSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::write(f, format_args!("ClientSession"))
    }
}

impl Actor for ClientSession {}
impl ClientSession {
    fn new(resolver: Addr<ProxyResolver>, credentials: Addr<CredentialProvider>) -> Self {
        ClientSession {
            started_at: Instant::now(),
            resolver,
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

        info!(
            "request processed in {}µs",
            self.started_at.elapsed().as_micros()
        );
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
            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(mut upgraded) => {
                        let mut server = TcpStream::connect(remote_host)
                            .await
                            .expect("remote connection failed");
                        if let Err(_) = tunnel(&mut upgraded, &mut server).await {};
                    }
                    Err(e) => error!("upgrade error: {}", e),
                }
            });

            Ok(Response::new(Body::empty()))
        } else {
            client.request(req).await
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
            });
            Ok(Response::new(Body::empty()))
        } else {
            client.request(req).await
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
    let cfg = confy::load::<ProxyConfig>("nanoproxy").expect("failed to load config");
    tracing_subscriber::fmt::init();
    let args = Opts::parse();
    let listen_addr = SocketAddr::from(([127, 0, 0, 1], args.port));

    let credentials = spawn_actor(CredentialProvider::from_auth_rules(cfg.auth_rules));
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
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let session =
                    spawn_actor(ClientSession::new(resolver.clone(), credentials.clone()));
                call!(session.proxy(client.clone(), req))
            }))
        }
    });

    let server = Server::bind(&listen_addr)
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .serve(make_service);

    println!("Listening on http://{}", listen_addr);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}