use std::fmt;

#[derive(Debug, Clone)]
#[allow(dead_code)] // Some variants prepared for future use
pub enum ProxyError {
    InvalidUri(String),
    MissingHost,
    ConnectionFailed(String),
    TunnelFailed(String),
    ResolutionFailed(String),
    AuthenticationFailed(String),
    InvalidRequest(String),
    UpstreamError(String),
    Timeout,
    Unknown(String),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyError::InvalidUri(msg) => write!(f, "Invalid URI: {}", msg),
            ProxyError::MissingHost => write!(f, "Missing host in request"),
            ProxyError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            ProxyError::TunnelFailed(msg) => write!(f, "Tunnel failed: {}", msg),
            ProxyError::ResolutionFailed(msg) => write!(f, "Proxy resolution failed: {}", msg),
            ProxyError::AuthenticationFailed(msg) => write!(f, "Authentication failed: {}", msg),
            ProxyError::InvalidRequest(msg) => write!(f, "Invalid request: {}", msg),
            ProxyError::UpstreamError(msg) => write!(f, "Upstream proxy error: {}", msg),
            ProxyError::Timeout => write!(f, "Operation timed out"),
            ProxyError::Unknown(msg) => write!(f, "Unknown error: {}", msg),
        }
    }
}

impl std::error::Error for ProxyError {}

pub type Result<T> = std::result::Result<T, ProxyError>;
