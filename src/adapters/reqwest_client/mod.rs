use async_trait::async_trait;
use std::collections::HashMap;

use crate::domain::{Credentials, ProxyError, ProxyRequest, ProxyResponse, ProxyRoute, Result};
use crate::ports::HttpClientPort;

pub struct ReqwestHttpClient;

impl ReqwestHttpClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClientPort for ReqwestHttpClient {
    async fn execute(
        &self,
        request: &ProxyRequest,
        route: &ProxyRoute,
        credentials: Option<&Credentials>,
    ) -> Result<ProxyResponse> {
        let mut builder = reqwest::Client::builder();

        match route {
            ProxyRoute::Upstream { proxy_url } => {
                let proxy_uri = proxy_url
                    .as_str()
                    .parse::<reqwest::Url>()
                    .map_err(|e| ProxyError::InvalidUri(format!("Invalid proxy URL: {}", e)))?;

                let mut proxy = reqwest::Proxy::http(proxy_uri)
                    .map_err(|e| ProxyError::InvalidUri(format!("Failed to create proxy: {}", e)))?;

                if let Some(creds) = credentials {
                    proxy = proxy.basic_auth(&creds.username, &creds.password);
                }

                builder = builder.proxy(proxy);
            }
            ProxyRoute::Direct => {
                // Explicitly disable system proxies for direct connections
                builder = builder.no_proxy();
            }
            ProxyRoute::Blocked { reason } => {
                return Err(ProxyError::ConnectionFailed(reason.clone()));
            }
        }

        let client = builder
            .build()
            .map_err(|e| ProxyError::ConnectionFailed(format!("Failed to build HTTP client: {}", e)))?;

        // Build request using target URL (reqwest will handle absolute-form for proxies automatically)
        let http_response = client
            .request(
                reqwest::Method::from_bytes(request.method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
                request.target_url.as_str(),
            )
            .headers(build_headers(&request.headers))
            .send()
            .await
            .map_err(|e| ProxyError::ConnectionFailed(format!("HTTP request failed: {}", e)))?;

        let status = http_response.status();

        let headers: HashMap<String, String> = http_response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
            .collect();

        let body = http_response
            .bytes()
            .await
            .map_err(|e| ProxyError::ConnectionFailed(format!("Failed to read response body: {}", e)))?
            .to_vec();

        Ok(ProxyResponse::new(status).with_headers(headers).with_body(body))
    }
}

fn build_headers(headers: &HashMap<String, String>) -> reqwest::header::HeaderMap {
    let mut header_map = reqwest::header::HeaderMap::new();

    for (key, value) in headers {
        if let (Ok(name), Ok(val)) = (
            key.parse::<reqwest::header::HeaderName>(),
            value.parse::<reqwest::header::HeaderValue>(),
        ) {
            header_map.insert(name, val);
        }
    }

    header_map
}
