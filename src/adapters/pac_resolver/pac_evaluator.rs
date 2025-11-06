use crate::domain::{ProxyError, ProxyRoute, Result};
use boa_engine::{Context, Source};
use url::Url;

const PAC_UTILS: &str = include_str!("../../pac_utils.js");

pub fn evaluate_pac(pac_file: &str, url: &Url) -> Result<Vec<String>> {
    let host = url
        .host_str()
        .ok_or_else(|| ProxyError::InvalidUri("Missing host".into()))?;

    let pac_payload = format!("{}\n{}", pac_file, PAC_UTILS);

    let mut context = Context::default();

    // Execute the PAC script in the context
    context
        .eval(Source::from_bytes(&pac_payload))
        .map_err(|e| ProxyError::ResolutionFailed(format!("PAC script error: {}", e)))?;

    // Call FindProxyForURL function
    let url_str = url.to_string();
    let result = context
        .eval(Source::from_bytes(&format!(
            "FindProxyForURL('{}', '{}')",
            escape_js_string(&url_str),
            escape_js_string(host)
        )))
        .map_err(|e| ProxyError::ResolutionFailed(format!("PAC execution error: {}", e)))?;

    let result_string = result
        .to_string(&mut context)
        .map_err(|e| ProxyError::ResolutionFailed(format!("Failed to convert result: {}", e)))?
        .to_std_string()
        .map_err(|_| ProxyError::ResolutionFailed("Failed to convert result to string".into()))?;

    let proxies: Vec<String> = result_string
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

pub fn parse_proxy_route(proxy_url: &str) -> Result<ProxyRoute> {
    let url: Url = proxy_url
        .parse()
        .map_err(|e| ProxyError::InvalidUri(format!("Invalid proxy URL: {}", e)))?;

    match url.scheme() {
        "direct" => Ok(ProxyRoute::Direct),
        "http" | "https" => Ok(ProxyRoute::Upstream { proxy_url: url }),
        scheme => Err(ProxyError::InvalidUri(format!("Unsupported scheme: {}", scheme))),
    }
}

/// Escape special characters in JavaScript strings
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PAC: &str = r#"
//
// Generic PAC file for testing
//

// Proxy definitions
var INTERNAL_PROXY = "PROXY internal.proxy.local:3128; PROXY backup.proxy.local:3128; ";
var EXTERNAL_PROXY = "PROXY external.proxy.local:3128; ";
var STREAMING_PROXY = "PROXY streaming.proxy.local:3128; ";
var DIRECT_ROUTE = "DIRECT";

// Domains that should bypass proxy (direct access)
var DirectDomains = new Array(
    "localhost",
    "127.0.0.1",
    "internal.local",
    "intranet.example.com",
    "git.example.com"
);

// Streaming services
var StreamingDomains = new Array(
    "teams.microsoft.com",
    "webex.com",
    "jitsi.org"
);

// Internal corporate domains
var InternalDomains = new Array(
    "example.com",
    "corp.example.com",
    "api.example.com"
);

// Helper function to convert IP to number
function IPnumber(IPaddress) {
    var ip = IPaddress.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
    if (ip) {
        return (+ip[1] << 24) + (+ip[2] << 16) + (+ip[3] << 8) + (+ip[4]);
    }
    return null;
}

// Helper to create IP mask
function IPmask(maskSize) {
    return -1 << (32 - maskSize);
}

// Check if IP is in CIDR range
function IPinCIDR(IPaddress, cidrRange) {
    var parts = cidrRange.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)\/(\d+)$/);
    if (parts) {
        var network = (+parts[1] << 24) + (+parts[2] << 16) + (+parts[3] << 8) + (+parts[4]);
        var netmask = IPmask(+parts[5]);
        return ((IPnumber(IPaddress) & netmask) == network);
    }
    return false;
}

// Check if host matches domain (including subdomains)
function dnsDomainIs(host, domain) {
    return (host.length >= domain.length &&
        host.substring(host.length - domain.length) == domain);
}

// Check if it's a simple hostname (no dots)
function isPlainHostName(host) {
    return !host.includes(".");
}

// Main PAC function
function FindProxyForURL(url, host) {
    // Check direct access domains first
    for (var i = 0; i < DirectDomains.length; i++) {
        if (dnsDomainIs(host, DirectDomains[i])) {
            return DIRECT_ROUTE;
        }
    }

    // Check streaming domains
    for (var i = 0; i < StreamingDomains.length; i++) {
        if (dnsDomainIs(host, StreamingDomains[i])) {
            return STREAMING_PROXY;
        }
    }

    // Check internal domains
    for (var i = 0; i < InternalDomains.length; i++) {
        if (dnsDomainIs(host, InternalDomains[i])) {
            return INTERNAL_PROXY;
        }
    }

    // Plain hostnames (no dot) get internal proxy
    if (isPlainHostName(host)) {
        return INTERNAL_PROXY;
    }

    // Default: use external proxy
    return EXTERNAL_PROXY;
}
"#;

    #[test]
    fn test_direct_access_by_domain() {
        let url = Url::parse("http://intranet.example.com/path").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert_eq!(result, vec!["direct://"]);
    }

    #[test]
    fn test_internal_proxy_domain() {
        let url = Url::parse("http://api.example.com/v1/data").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(result.iter().any(|r| r.contains("internal.proxy.local:3128")));
    }

    #[test]
    fn test_streaming_proxy_domain() {
        let url = Url::parse("http://teams.microsoft.com/meeting").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(result.iter().any(|r| r.contains("streaming.proxy.local:3128")));
    }

    #[test]
    fn test_external_proxy_default() {
        let url = Url::parse("http://unknown.external.com/path").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(result.iter().any(|r| r.contains("external.proxy.local:3128")));
    }

    #[test]
    fn test_plain_hostname_internal_proxy() {
        let url = Url::parse("http://localhost/path").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(result
            .iter()
            .any(|r| r.contains("internal.proxy.local:3128") || r == "direct://"));
    }

    #[test]
    fn test_multiple_proxy_results() {
        let url = Url::parse("http://internal.local/test").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert_eq!(result, vec!["direct://"]);
    }

    #[test]
    fn test_https_url_handling() {
        let url = Url::parse("https://api.example.com/secure").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_parse_proxy_route_direct() {
        let route = parse_proxy_route("direct://").unwrap();
        assert!(matches!(route, ProxyRoute::Direct));
    }

    #[test]
    fn test_parse_proxy_route_http() {
        let route = parse_proxy_route("http://proxy.example.com:3128").unwrap();
        match route {
            ProxyRoute::Upstream { proxy_url, .. } => {
                assert_eq!(proxy_url.host_str(), Some("proxy.example.com"));
                assert_eq!(proxy_url.port(), Some(3128));
            }
            _ => panic!("Expected Upstream route"),
        }
    }

    #[test]
    fn test_parse_proxy_route_https() {
        let route = parse_proxy_route("https://secure.proxy.local:3128").unwrap();
        match route {
            ProxyRoute::Upstream { proxy_url, .. } => {
                assert_eq!(proxy_url.scheme(), "https");
            }
            _ => panic!("Expected Upstream route"),
        }
    }

    #[test]
    fn test_parse_proxy_route_invalid() {
        let result = parse_proxy_route("ftp://invalid.proxy:3128");
        assert!(result.is_err());
    }

    #[test]
    fn test_url_with_special_characters() {
        let url = Url::parse("http://test.example.com/path?query=value&foo=bar").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_subdomain_matching() {
        let url = Url::parse("http://sub.api.example.com/data").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(result.iter().any(|r| r.contains("internal.proxy.local:3128")));
    }

    #[test]
    fn test_webex_streaming_domain() {
        let url = Url::parse("http://webex.com/meeting/123").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert!(result.iter().any(|r| r.contains("streaming.proxy.local:3128")));
    }

    #[test]
    fn test_git_direct_access() {
        let url = Url::parse("http://git.example.com/repo").unwrap();
        let result = evaluate_pac(TEST_PAC, &url).unwrap();
        assert_eq!(result, vec!["direct://"]);
    }
}
