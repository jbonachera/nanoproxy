mod adapters;
mod domain;
mod ports;

use clap::Parser;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use log::{error, info, LevelFilter};
use rlimit::{getrlimit, setrlimit, Resource};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use adapters::{
    BeaconPoller, ConnectionTracker, CredentialProvider, GatewayListener, HyperConnector, HyperProxyAdapter,
    PacProxyResolver, ReqwestHttpClient, ResolvConfListener,
};
use domain::{AuthRule, GatewayRule, PacRule, ProxyService, ResolvConfRule};

#[derive(Debug, Serialize, Deserialize)]
struct SystemConfiguration {
    max_connections: u64,
    #[serde(default)]
    log_level: String,
}

fn parse_log_level(level: &str) -> LevelFilter {
    match level.to_lowercase().as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        "off" => LevelFilter::Off,
        _ => LevelFilter::Info,
    }
}

impl Default for SystemConfiguration {
    fn default() -> Self {
        Self {
            max_connections: 1024,
            log_level: "info".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ProxyConfig {
    #[serde(default)]
    system: SystemConfiguration,

    #[serde(default)]
    detection_type: Option<String>,

    #[serde(default)]
    auth_rules: Option<Vec<AuthRule>>,

    #[serde(default)]
    pac_rules: Option<Vec<PacRule>>,

    #[serde(default)]
    resolvconf_rules: Option<Vec<ResolvConfRule>>,

    #[serde(default)]
    gateway_rules: Option<Vec<GatewayRule>>,
}

#[derive(Parser, Debug)]
#[clap(version = env!("NANOPROXY_VERSION"), author = env!("CARGO_PKG_AUTHORS"))]
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

    // Initialize logger with configured level, respecting RUST_LOG env var
    env_logger::Builder::from_default_env()
        .filter_level(parse_log_level(&cfg.system.log_level))
        .init();

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

    // Start network detection based on detection_type
    let detection_type = cfg.detection_type.as_deref().unwrap_or("dns");
    match detection_type {
        "dns" => {
            info!("Starting DNS-based detection");
            if let Some(resolvconf_rules) = cfg.resolvconf_rules {
                let listener = ResolvConfListener::new(resolvconf_rules, resolver.clone());
                listener.start()?;
            }
        }
        "route" => {
            info!("Starting Gateway-based detection");
            if let Some(gateway_rules) = cfg.gateway_rules {
                let listener = GatewayListener::new(gateway_rules, resolver.clone());
                listener.start()?;
            }
        }
        other => {
            log::warn!("Unknown detection_type '{}', defaulting to 'dns'", other);
            if let Some(resolvconf_rules) = cfg.resolvconf_rules {
                let listener = ResolvConfListener::new(resolvconf_rules, resolver.clone());
                listener.start()?;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn create_test_config(name: &str, content: &str) -> std::path::PathBuf {
        let temp_dir = std::env::temp_dir();
        let config_dir = temp_dir.join(format!("nanoproxy_test_{}", name));
        fs::create_dir_all(&config_dir).expect("Failed to create test config dir");

        let config_file = config_dir.join("nanoproxy.toml");
        let mut file = fs::File::create(&config_file).expect("Failed to create test config file");
        file.write_all(content.as_bytes()).expect("Failed to write test config");

        config_file
    }

    fn cleanup_test_config(config_file: &std::path::Path) {
        if let Some(parent) = config_file.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn test_config_with_detection_type_dns() {
        let toml_content = r#"detection_type = "dns"

[system]
max_connections = 1024
log_level = "info"

[[resolvconf_rules]]
resolver_subnet = "10.241.52.0/24"
pac_url = "http://pac.example.com/proxy.pac"
"#;

        let config_file = create_test_config("dns_detection", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert_eq!(config.detection_type, Some("dns".to_string()));
        assert!(config.resolvconf_rules.is_some());
        assert_eq!(config.resolvconf_rules.as_ref().unwrap().len(), 1);
        assert_eq!(
            config.resolvconf_rules.as_ref().unwrap()[0].resolver_subnet,
            "10.241.52.0/24"
        );
    }

    #[test]
    fn test_config_with_detection_type_route() {
        let toml_content = r#"detection_type = "route"

[system]
max_connections = 1024

[[gateway_rules]]
default_route_interface = "en0"
pac_url = "http://pac.example.com/proxy.pac"
"#;

        let config_file = create_test_config("route_detection", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert_eq!(config.detection_type, Some("route".to_string()));
        assert!(config.gateway_rules.is_some());
        assert_eq!(config.gateway_rules.as_ref().unwrap().len(), 1);
        assert_eq!(config.gateway_rules.as_ref().unwrap()[0].default_route_interface, "en0");
    }

    #[test]
    fn test_config_backwards_compatibility() {
        let toml_content = r#"[system]
max_connections = 2048

[[resolvconf_rules]]
resolver_subnet = "10.241.52.0/24"
pac_url = "http://pac.example.com/proxy.pac"
"#;

        let config_file = create_test_config("backwards_compat", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert_eq!(config.detection_type, None);
        assert!(config.resolvconf_rules.is_some());
        assert_eq!(config.system.max_connections, 2048);
    }

    #[test]
    fn test_gateway_rule_with_commands() {
        let toml_content = r#"detection_type = "route"

[[gateway_rules]]
default_route_interface = "en0"
pac_url = "http://pac.example.com/proxy.pac"
when_match = "echo 'Interface matched'"
when_no_match = "echo 'Interface not matched'"
"#;

        let config_file = create_test_config("gateway_commands", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert!(config.gateway_rules.is_some());
        let rules = config.gateway_rules.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].default_route_interface, "en0");
        assert_eq!(rules[0].interface_ip_subnet, None);
        assert_eq!(rules[0].pac_url, "http://pac.example.com/proxy.pac");
        assert_eq!(rules[0].when_match, Some("echo 'Interface matched'".to_string()));
        assert_eq!(rules[0].when_no_match, Some("echo 'Interface not matched'".to_string()));
    }

    #[test]
    fn test_gateway_rule_with_ip_subnet() {
        let toml_content = r#"detection_type = "route"

[[gateway_rules]]
default_route_interface = "en0"
interface_ip_subnet = "192.168.1.0/24"
pac_url = "http://pac.example.com/proxy.pac"
"#;

        let config_file = create_test_config("gateway_ip_subnet", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert!(config.gateway_rules.is_some());
        let rules = config.gateway_rules.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].default_route_interface, "en0");
        assert_eq!(rules[0].interface_ip_subnet, Some("192.168.1.0/24".to_string()));
        assert_eq!(rules[0].pac_url, "http://pac.example.com/proxy.pac");
    }

    #[test]
    fn test_multiple_gateway_rules() {
        let toml_content = r#"detection_type = "route"

[[gateway_rules]]
default_route_interface = "en0"
pac_url = "http://pac1.example.com/proxy.pac"

[[gateway_rules]]
default_route_interface = "utun*"
pac_url = "http://pac2.example.com/proxy.pac"
"#;

        let config_file = create_test_config("multiple_gateways", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert!(config.gateway_rules.is_some());
        let rules = config.gateway_rules.unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].default_route_interface, "en0");
        assert_eq!(rules[1].default_route_interface, "utun*");
    }

    #[test]
    fn test_mixed_rules_with_dns_detection() {
        let toml_content = r#"detection_type = "dns"

[[resolvconf_rules]]
resolver_subnet = "10.241.52.0/24"
pac_url = "http://dns-pac.example.com/proxy.pac"

[[gateway_rules]]
default_route_interface = "en0"
pac_url = "http://gateway-pac.example.com/proxy.pac"
"#;

        let config_file = create_test_config("mixed_dns", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert_eq!(config.detection_type, Some("dns".to_string()));
        assert!(config.resolvconf_rules.is_some());
        assert!(config.gateway_rules.is_some());
    }

    #[test]
    fn test_mixed_rules_with_route_detection() {
        let toml_content = r#"detection_type = "route"

[[resolvconf_rules]]
resolver_subnet = "10.241.52.0/24"
pac_url = "http://dns-pac.example.com/proxy.pac"

[[gateway_rules]]
default_route_interface = "en0"
pac_url = "http://gateway-pac.example.com/proxy.pac"
"#;

        let config_file = create_test_config("mixed_route", toml_content);
        let config: ProxyConfig = confy::load_path(&config_file).expect("Failed to load config");
        cleanup_test_config(&config_file);

        assert_eq!(config.detection_type, Some("route".to_string()));
        assert!(config.resolvconf_rules.is_some());
        assert!(config.gateway_rules.is_some());
    }
}
