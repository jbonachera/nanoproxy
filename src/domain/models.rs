pub use http::StatusCode;
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub route: ProxyRoute,
    pub credentials: Option<Credentials>,
    pub connection_id: uuid::Uuid,
}

#[derive(Debug, Clone)]
pub struct ProxyResponse {
    pub status: StatusCode,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub tunnel_required: Option<TunnelInfo>,
}

impl ProxyResponse {
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: Vec::new(),
            tunnel_required: None,
        }
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_tunnel(mut self, tunnel_info: TunnelInfo) -> Self {
        self.tunnel_required = Some(tunnel_info);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Connect,
    Patch,
    Trace,
    Other(String),
}

impl ProxyMethod {
    pub fn as_str(&self) -> &str {
        match self {
            ProxyMethod::Get => "GET",
            ProxyMethod::Post => "POST",
            ProxyMethod::Put => "PUT",
            ProxyMethod::Delete => "DELETE",
            ProxyMethod::Head => "HEAD",
            ProxyMethod::Options => "OPTIONS",
            ProxyMethod::Connect => "CONNECT",
            ProxyMethod::Patch => "PATCH",
            ProxyMethod::Trace => "TRACE",
            ProxyMethod::Other(s) => s.as_str(),
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "GET" => ProxyMethod::Get,
            "POST" => ProxyMethod::Post,
            "PUT" => ProxyMethod::Put,
            "DELETE" => ProxyMethod::Delete,
            "HEAD" => ProxyMethod::Head,
            "OPTIONS" => ProxyMethod::Options,
            "CONNECT" => ProxyMethod::Connect,
            "PATCH" => ProxyMethod::Patch,
            "TRACE" => ProxyMethod::Trace,
            other => ProxyMethod::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyRequest {
    pub method: ProxyMethod,
    pub target_url: Url,
    pub headers: HashMap<String, String>,
    #[allow(dead_code)]
    pub version: HttpVersion,
}

impl ProxyRequest {
    pub fn new(method: ProxyMethod, target_url: Url) -> Self {
        Self {
            method,
            target_url,
            headers: HashMap::new(),
            version: HttpVersion::Http11,
        }
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HttpVersion {
    Http10,
    Http11,
    Http2,
    Http3,
}

#[derive(Debug, Clone)]
pub struct ConnectRequest {
    pub target_url: Url,
    pub headers: HashMap<String, String>,
}

impl ConnectRequest {
    pub fn new(target_url: Url) -> Self {
        Self {
            target_url,
            headers: HashMap::new(),
        }
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ProxyRoute {
    Direct,
    Upstream { proxy_url: Url },
    Blocked { reason: String },
}
impl std::fmt::Display for ProxyRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyRoute::Direct => write!(f, "direct"),
            ProxyRoute::Upstream { proxy_url, .. } => write!(f, "upstream {}", proxy_url),
            ProxyRoute::Blocked { reason } => write!(f, "blocked: {}", reason),
        }
    }
}

impl ProxyRoute {
    pub fn scheme(&self) -> &str {
        match self {
            ProxyRoute::Direct => "direct",
            ProxyRoute::Upstream { proxy_url, .. } => proxy_url.scheme(),
            ProxyRoute::Blocked { .. } => "blocked",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl Credentials {
    pub fn new(username: String, password: String) -> Self {
        Self { username, password }
    }

    pub fn to_basic_auth(&self) -> String {
        use base64::Engine;
        let credentials = format!("{}:{}", self.username, self.password);
        format!("Basic {}", base64::prelude::BASE64_STANDARD.encode(credentials))
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ConnectionInfo {
    pub id: uuid::Uuid,
    pub method: String,
    pub target: String,
    pub route: String,
    pub opened_at: std::time::Instant,
    pub closed_at: Option<std::time::Instant>,
}

impl ConnectionInfo {
    pub fn new(method: String, target: String, route: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            method,
            target,
            route,
            opened_at: std::time::Instant::now(),
            closed_at: None,
        }
    }

    #[allow(dead_code)]
    pub fn close(&mut self) {
        self.closed_at = Some(std::time::Instant::now());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthRule {
    pub remote_pattern: String,
    pub username: String,
    pub password_command: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PacRule {
    pub beacon_host: String,
    pub pac_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvConfRule {
    pub resolver_subnet: String,
    pub pac_url: String,
    pub when_match: Option<String>,
    pub when_no_match: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayRule {
    pub default_gateway_subnet: String,
    pub pac_url: String,
    pub when_match: Option<String>,
    pub when_no_match: Option<String>,
}
