use crate::domain::{ProxyError, ProxyRoute, Result};
use js_sandbox::{JsValue, Script};
use url::Url;

const PAC_UTILS: &str = include_str!("../../pac_utils.js");

/// Evaluate a PAC file to get proxy URLs for a target URL
pub fn evaluate_pac(pac_file: &str, url: &Url) -> Result<Vec<String>> {
    let pac_payload = format!("{}\n{}", pac_file, PAC_UTILS);

    let mut script = Script::from_string(&pac_payload)
        .map_err(|e| ProxyError::ResolutionFailed(format!("PAC script error: {}", e)))?;

    let host = url
        .host_str()
        .ok_or_else(|| ProxyError::InvalidUri("Missing host".into()))?;

    let eval_result: JsValue = script
        .call("FindProxyForURL", (url.to_string(), host.to_string()))
        .map_err(|e| ProxyError::ResolutionFailed(format!("PAC execution error: {}", e)))?;

    let proxies: Vec<String> = eval_result
        .to_string()
        .replace('"', "")
        .split(';')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| {
            let parts: Vec<&str> = v.split_whitespace().collect();
            match parts.first() {
                Some(&"DIRECT") => "direct://".to_owned(),
                Some(&"PROXY") => {
                    if let Some(proxy) = parts.get(1) {
                        format!("http://{}", proxy)
                    } else {
                        "direct://".to_owned()
                    }
                }
                _ => "direct://".to_owned(),
            }
        })
        .collect();

    if proxies.is_empty() {
        Ok(vec!["direct://".to_owned()])
    } else {
        Ok(proxies)
    }
}

/// Convert PAC proxy URL string to ProxyRoute
pub fn parse_proxy_route(proxy_url: &str) -> Result<ProxyRoute> {
    let url: Url = proxy_url
        .parse()
        .map_err(|e| ProxyError::InvalidUri(format!("Invalid proxy URL: {}", e)))?;

    match url.scheme() {
        "direct" => Ok(ProxyRoute::Direct),
        "http" | "https" => Ok(ProxyRoute::Upstream {
            proxy_url: url,
            credentials: None,
        }),
        scheme => Err(ProxyError::InvalidUri(format!("Unsupported scheme: {}", scheme))),
    }
}
