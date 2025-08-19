mod connection;
mod connector;
mod credentials;
mod pac;
mod resolver;
mod tracker;

use std::net::SocketAddr;

use connector::ProxyConnector;
use credentials::{CredentialProvider, ProxyAuthRule};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{body::Incoming, http, Method, Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;

type Body = BoxBody<Bytes, hyper::Error>;

use resolver::{BeaconPoller, ProxyPACRule, ProxyResolver, ResolvConfListener, ResolvConfRule};
use serde::{Deserialize, Serialize};
use std::env;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tracker::ConnectionTracker;
use url::Url;
use uuid::Uuid;

use rlimit::{getrlimit, setrlimit, Resource};

use tokio::net::TcpStream;

use crate::tracker::StreamInfo;
use act_zero::runtimes::tokio::spawn_actor;
use act_zero::*;
use clap::Parser;
use hyper::header::HeaderValue;
use tracing::{error, instrument};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Serialize, Deserialize)]
struct SystemConfiguration {
    max_connections: u64,
}

impl Default for SystemConfiguration {
    fn default() -> SystemConfiguration {
        Self { max_connections: 1024 }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ProxyConfig {
    #[serde(default)]
    system: SystemConfiguration,

    #[serde(default)]
    auth_rules: Option<Vec<ProxyAuthRule>>,

    #[serde(default)]
    pac_rules: Option<Vec<ProxyPACRule>>,

    #[serde(default)]
    resolvconf_rules: Option<Vec<ResolvConfRule>>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            system: SystemConfiguration::default(),
            auth_rules: Some(vec![ProxyAuthRule::default()]),
            pac_rules: Some(vec![ProxyPACRule::default()]),
            resolvconf_rules: Some(vec![ResolvConfRule::default()]),
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

fn remote_host<T>(req: &Request<T>) -> Result<Url, Box<dyn std::error::Error>> {
    if Method::CONNECT == req.method() {
        if let Some(addr) = host_addr(req.uri()) {
            let url_str = format!("https://{addr}/");
            return url_str
                .parse()
                .map_err(|e| format!("Invalid CONNECT URI: {}", e).into());
        }
    }

    let host_header = req
        .headers()
        .get("host")
        .ok_or("Missing host header")?
        .to_str()
        .map_err(|e| format!("Invalid host header: {}", e))?;

    let url_str = format!("http://{}/", host_header);
    url_str
        .parse()
        .map_err(|e| format!("Invalid host header URL: {}", e).into())
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
        client: Client<ProxyConnector, Body>,
        req: Request<Incoming>,
    ) -> ActorResult<Response<Body>> {
        let req = req.map(|body| body.boxed());
        let remote = match remote_host(&req) {
            Ok(url) => url,
            Err(e) => {
                error!("Invalid URI in request: {}", e);
                return Produces::ok(
                    Response::builder()
                        .status(400)
                        .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                        .unwrap(),
                );
            }
        };
        let upstream_url: Url = call!(self.resolver.resolve_proxy_for_url(remote)).await.unwrap();

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
        client: Client<ProxyConnector, Body>,
        req: Request<Body>,
        remote_host: String,
    ) -> Result<Response<Body>, hyper_util::client::legacy::Error> {
        if Method::CONNECT == req.method() {
            let id = self.id.clone();
            let connection_tracker = self.connection_tracker.clone();

            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        let mut upgraded = TokioIo::new(upgraded);
                        let mut server = match TcpStream::connect(&remote_host).await {
                            Ok(s) => s,
                            Err(e) => {
                                error!("Failed to connect to remote host {}: {}", remote_host, e);
                                return;
                            }
                        };

                        if let Err(e) = tunnel(&mut upgraded, &mut server).await {
                            error!("Tunnel error for {}: {}", remote_host, e);
                        };
                    }
                    Err(err) => {
                        error!("failed to upgrade to CONNECT: {}", err);
                    }
                }
                call!(connection_tracker.remove(id)).await.unwrap();
            });

            let response = Response::builder()
                .status(200)
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap();

            Ok(response)
        } else {
            let resp = client.request(req).await.map(|res| res.map(|body| body.boxed()));
            call!(self.connection_tracker.remove(self.id)).await.unwrap();

            resp
        }
    }
    #[instrument]
    async fn forward_upstream(
        &self,
        client: Client<ProxyConnector, Body>,
        mut req: Request<Body>,
        creds: Option<String>,
    ) -> Result<Response<Body>, hyper_util::client::legacy::Error> {
        let creds = creds.unwrap_or("".to_string());
        if creds.len() > 0 {
            req.headers_mut().insert(
                http::header::PROXY_AUTHORIZATION,
                HeaderValue::from_bytes(creds.as_bytes()).expect("msg"),
            );
        }
        if Method::CONNECT == req.method() {
            let mut server_req = Request::connect(req.uri())
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap();
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
                let uri = req.uri().clone();
                match hyper::upgrade::on(res).await {
                    Ok(server) => match hyper::upgrade::on(req).await {
                        Ok(upgraded) => {
                            let mut upgraded = TokioIo::new(upgraded);
                            let mut server = TokioIo::new(server);
                            if let Err(e) = tunnel(&mut upgraded, &mut server).await {
                                error!("{}: server io error: {}", uri, e);
                            };
                        }
                        Err(e) => error!("server refused to upgrade to CONNECT {}: {}", uri, e),
                    },
                    Err(e) => error!("client refused to upgrade to CONNECT {}: {}", uri, e),
                }
                call!(connection_tracker.remove(id)).await.unwrap();
            });
            Ok(Response::builder()
                .status(200)
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap())
        } else {
            let resp = client.request(req).await.map(|res| res.map(|body| body.boxed()));
            call!(self.connection_tracker.remove(self.id)).await.unwrap();
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

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Opts::parse();
    let listen_addr = SocketAddr::from(([127, 0, 0, 1], args.port));

    let credentials = spawn_actor(CredentialProvider::from_auth_rules(
        cfg.auth_rules.unwrap_or(vec![ProxyAuthRule::default()]),
    ));
    let connection_tracker = spawn_actor(ConnectionTracker::default());
    let resolver = spawn_actor(ProxyResolver::default());
    if let Some(v) = cfg.pac_rules {
        let _beacon_poller = spawn_actor(BeaconPoller::from_beacon_rules(v, resolver.clone()));
    }
    if let Some(v) = cfg.resolvconf_rules {
        let _resolvconf_listener = spawn_actor(ResolvConfListener::from_rules(v, resolver.clone()));
    }

    let connector = ProxyConnector::from(resolver.clone());

    let client = Client::builder(hyper_util::rt::TokioExecutor::new())
        .http1_title_case_headers(true)
        .http1_preserve_header_case(true)
        .build(connector);

    let (_, hard_limit) = getrlimit(Resource::NOFILE).unwrap();

    let max_connections = if cfg.system.max_connections < hard_limit {
        cfg.system.max_connections
    } else {
        hard_limit
    };

    setrlimit(Resource::NOFILE, max_connections, hard_limit).unwrap();

    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    if !args.no_greeting {
        println!(
            "🚀 Nanoproxy server is running on http://{}:{}.",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );
        println!(
            "Configuration loaded from {:#?}",
            confy::get_configuration_file_path("nanoproxy", "nanoproxy").expect("failed to load config")
        );
        println!("");
        println!(
            "export http_proxy=http://{}:{};",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );
        println!(
            "export https_proxy=http://{}:{};",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );
        println!(
            "export all_proxy=http://{}:{};",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );
        println!(
            "export HTTP_PROXY=http://{}:{};",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );
        println!(
            "export HTTPS_PROXY=http://{}:{};",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );
        println!(
            "export ALL_PROXY=http://{}:{};",
            listener.local_addr().unwrap().ip(),
            listener.local_addr().unwrap().port()
        );

        println!("export no_proxy=localhost,127.0.0.0/8,*.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16;");
        println!("");
        println!("Connection logs will appear below.");
    }

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);

        let client = client.clone();
        let resolver = resolver.clone();
        let credentials = credentials.clone();
        let connection_tracker = connection_tracker.clone();

        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                let session = spawn_actor(ClientSession::new(
                    connection_tracker.clone(),
                    resolver.clone(),
                    credentials.clone(),
                ));
                call!(session.proxy(client.clone(), req))
            });

            if let Err(err) = ServerBuilder::new(hyper_util::rt::TokioExecutor::new())
                .http1()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection_with_upgrades(io, service)
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}
