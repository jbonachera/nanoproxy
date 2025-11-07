mod adapters;
mod domain;
mod ports;

use clap::Parser;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use log::error;
use rlimit::{getrlimit, setrlimit, Resource};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use adapters::{
    BeaconPoller, ConnectionTracker, CredentialProvider, HyperConnector, HyperProxyAdapter, PacProxyResolver,
    ReqwestHttpClient, ResolvConfListener,
};
use domain::{AuthRule, PacRule, ProxyService, ResolvConfRule};

#[derive(Debug, Serialize, Deserialize)]
struct SystemConfiguration {
    max_connections: u64,
}

impl Default for SystemConfiguration {
    fn default() -> Self {
        Self { max_connections: 1024 }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ProxyConfig {
    #[serde(default)]
    system: SystemConfiguration,

    #[serde(default)]
    auth_rules: Option<Vec<AuthRule>>,

    #[serde(default)]
    pac_rules: Option<Vec<PacRule>>,

    #[serde(default)]
    resolvconf_rules: Option<Vec<ResolvConfRule>>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            system: SystemConfiguration::default(),
            auth_rules: None,
            pac_rules: None,
            resolvconf_rules: None,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let cfg = confy::load::<ProxyConfig>("nanoproxy", "nanoproxy")?;
    env_logger::init();

    let args = Opts::parse();
    let listen_addr = SocketAddr::from(([127, 0, 0, 1], args.port));

    // Set up resource limits
    let (_, hard_limit) = getrlimit(Resource::NOFILE)?;
    let max_connections = if cfg.system.max_connections < hard_limit {
        cfg.system.max_connections
    } else {
        hard_limit
    };
    setrlimit(Resource::NOFILE, max_connections, hard_limit)?;

    // Create ports (dependency injection)
    let resolver: Arc<dyn ports::ProxyResolverPort> = Arc::new(PacProxyResolver::new());

    let auth_rules = cfg.auth_rules.unwrap_or_default();
    let credentials: Arc<dyn ports::CredentialsPort> = Arc::new(CredentialProvider::new(auth_rules));

    let tracker = Arc::new(ConnectionTracker::new());
    let tracker_port: Arc<dyn ports::TrackingPort> = tracker.clone();

    // Start background tasks
    tracker.start_cleanup();

    // Start beacon poller if configured
    if let Some(pac_rules) = cfg.pac_rules {
        let poller = BeaconPoller::new(pac_rules, resolver.clone());
        poller.start();
    }

    // Start resolvconf listener if configured
    if let Some(resolvconf_rules) = cfg.resolvconf_rules {
        let listener = ResolvConfListener::new(resolvconf_rules, resolver.clone());
        listener.start()?;
    }

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

    // Bind listener
    let listener = TcpListener::bind(&listen_addr).await?;

    if !args.no_greeting {
        print_greeting(&listener);
    }

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let adapter = adapter.clone();

        tokio::spawn(async move {
            let service_fn = service_fn(move |req| {
                let adapter = adapter.clone();
                async move { Ok::<_, hyper::Error>(adapter.handle(req).await) }
            });

            if let Err(err) = ServerBuilder::new(hyper_util::rt::TokioExecutor::new())
                .http1()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection_with_upgrades(io, service_fn)
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}

fn print_greeting(listener: &TcpListener) {
    let addr = listener.local_addr().unwrap();
    println!(
        "🚀 Nanoproxy server is running on http://{}:{}.",
        addr.ip(),
        addr.port()
    );
    println!(
        "Configuration loaded from {:#?}",
        confy::get_configuration_file_path("nanoproxy", "nanoproxy").expect("failed to load config")
    );
    println!();
    println!("export http_proxy=http://{}:{};", addr.ip(), addr.port());
    println!("export https_proxy=http://{}:{};", addr.ip(), addr.port());
    println!("export all_proxy=http://{}:{};", addr.ip(), addr.port());
    println!("export HTTP_PROXY=http://{}:{};", addr.ip(), addr.port());
    println!("export HTTPS_PROXY=http://{}:{};", addr.ip(), addr.port());
    println!("export ALL_PROXY=http://{}:{};", addr.ip(), addr.port());
    println!("export no_proxy=localhost,127.0.0.0/8,*.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16;");
    println!();
    println!("Connection logs will appear below.");
}
