use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpStatus {
    Ok,
    Forbidden,
    NotFound,
    InternalServerError,
    Other(u16),
}

impl HttpStatus {
    pub fn from_u16(code: u16) -> Self {
        match code {
            200 => HttpStatus::Ok,
            403 => HttpStatus::Forbidden,
            404 => HttpStatus::NotFound,
            500 => HttpStatus::InternalServerError,
            other => HttpStatus::Other(other),
        }
    }

    pub fn as_u16(&self) -> u16 {
        match self {
            HttpStatus::Ok => 200,
            HttpStatus::Forbidden => 403,
            HttpStatus::NotFound => 404,
            HttpStatus::InternalServerError => 500,
            HttpStatus::Other(code) => *code,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyResponse {
    pub status: HttpStatus,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl ProxyResponse {
    pub fn new(status: HttpStatus) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: Vec::new(),
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
pub enum ProxyRoute {
    Direct,
    Upstream {
        proxy_url: Url,
        #[allow(dead_code)]
        credentials: Option<Credentials>,
    },
    #[allow(dead_code)]
    Blocked {
        reason: String,
    },
}

impl ProxyRoute {
    pub fn scheme(&self) -> &str {
        match self {
            ProxyRoute::Direct => "direct",
            ProxyRoute::Upstream { proxy_url, .. } => proxy_url.scheme(),
            ProxyRoute::Blocked { .. } => "blocked",
        }
    }

    pub fn is_direct(&self) -> bool {
        matches!(self, ProxyRoute::Direct)
    }

    pub fn is_upstream(&self) -> bool {
        matches!(self, ProxyRoute::Upstream { .. })
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, ProxyRoute::Blocked { .. })
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

    pub fn close(&mut self) {
        self.closed_at = Some(std::time::Instant::now());
    }

    pub fn duration(&self) -> Option<std::time::Duration> {
        self.closed_at.map(|closed| closed.duration_since(self.opened_at))
    }
}

#[derive(Debug)]
pub enum ConnectDecision {
    Accept {
        route: ProxyRoute,
        credentials: Option<Credentials>,
        connection_id: uuid::Uuid,
    },
    Rejected {
        reason: String,
    },
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
