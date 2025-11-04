use std::sync::Arc;
use uuid::Uuid;

use super::{
    ConnectDecision, ConnectRequest, ConnectionInfo, Credentials, ProxyRequest, ProxyResponse, ProxyRoute, Result,
};
use crate::ports::{CredentialsPort, HttpClientPort, ProxyResolverPort, TrackingPort};

#[derive(Clone)]
pub struct ProxyService {
    resolver: Arc<dyn ProxyResolverPort>,
    credentials: Arc<dyn CredentialsPort>,
    tracker: Arc<dyn TrackingPort>,
    http_client: Arc<dyn HttpClientPort>,
}

impl ProxyService {
    pub fn new(
        resolver: Arc<dyn ProxyResolverPort>,
        credentials: Arc<dyn CredentialsPort>,
        tracker: Arc<dyn TrackingPort>,
        http_client: Arc<dyn HttpClientPort>,
    ) -> Self {
        Self {
            resolver,
            credentials,
            tracker,
            http_client,
        }
    }

    pub async fn handle_http_request(&self, request: &ProxyRequest) -> Result<ProxyResponse> {
        let route = self.resolver.resolve_route(&request.target_url).await?;
        let credentials = self.get_credentials_for_route(&route).await?;

        let conn_info = ConnectionInfo::new(
            request.method.as_str().to_string(),
            request.target_url.to_string(),
            route.scheme().to_string(),
        );
        let conn_id = conn_info.id;
        self.tracker.track_connection(conn_info).await?;

        let response = self.http_client.execute(request, &route, credentials.as_ref()).await?;

        self.tracker.close_connection(conn_id).await?;

        Ok(response)
    }

    pub async fn handle_connect_request(&self, request: &ConnectRequest) -> Result<ConnectDecision> {
        let route = self.resolver.resolve_route(&request.target_url).await?;

        if let ProxyRoute::Blocked { reason } = &route {
            return Ok(ConnectDecision::Rejected { reason: reason.clone() });
        }

        let credentials = self.get_credentials_for_route(&route).await?;

        let conn_info = ConnectionInfo::new(
            "CONNECT".to_string(),
            request.target_url.to_string(),
            route.scheme().to_string(),
        );
        let conn_id = conn_info.id;
        self.tracker.track_connection(conn_info).await?;

        Ok(ConnectDecision::Accept {
            route,
            credentials,
            connection_id: conn_id,
        })
    }

    pub async fn close_connection(&self, id: Uuid) -> Result<()> {
        self.tracker.close_connection(id).await
    }

    pub async fn get_credentials_for_route(&self, route: &ProxyRoute) -> Result<Option<Credentials>> {
        match route {
            ProxyRoute::Upstream { proxy_url, .. } => {
                if let Some(host) = proxy_url.host_str() {
                    self.credentials.get_credentials(host).await
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProxyMethod;
    use async_trait::async_trait;
    use http;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use url::Url;

    mod mocks {
        use super::*;

        #[derive(Clone)]
        pub struct MockResolver {
            pub route: ProxyRoute,
        }

        impl MockResolver {
            pub fn new(route: ProxyRoute) -> Self {
                Self { route }
            }
        }

        #[async_trait]
        impl ProxyResolverPort for MockResolver {
            async fn resolve_route(&self, _: &Url) -> Result<ProxyRoute> {
                Ok(self.route.clone())
            }

            async fn resolve_all_routes(&self, _: &Url) -> Result<Vec<ProxyRoute>> {
                Ok(vec![self.route.clone()])
            }

            async fn update_pac_url(&self, _: Option<String>) -> Result<()> {
                Ok(())
            }
        }

        pub struct MockCredentials {
            pub credentials: Option<Credentials>,
        }

        impl MockCredentials {
            pub fn new(credentials: Option<Credentials>) -> Self {
                Self { credentials }
            }
        }

        #[async_trait]
        impl CredentialsPort for MockCredentials {
            async fn get_credentials(&self, _: &str) -> Result<Option<Credentials>> {
                Ok(self.credentials.clone())
            }

            async fn clear_cache(&self) -> Result<()> {
                Ok(())
            }
        }

        #[derive(Clone)]
        pub struct MockTracker {
            pub track_call_count: Arc<AtomicUsize>,
            pub close_call_count: Arc<AtomicUsize>,
        }

        impl MockTracker {
            pub fn new() -> Self {
                Self {
                    track_call_count: Arc::new(AtomicUsize::new(0)),
                    close_call_count: Arc::new(AtomicUsize::new(0)),
                }
            }

            pub fn track_calls(&self) -> usize {
                self.track_call_count.load(Ordering::SeqCst)
            }

            pub fn close_calls(&self) -> usize {
                self.close_call_count.load(Ordering::SeqCst)
            }
        }

        #[async_trait]
        impl TrackingPort for MockTracker {
            async fn track_connection(&self, _: ConnectionInfo) -> Result<()> {
                self.track_call_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            async fn close_connection(&self, _: Uuid) -> Result<()> {
                self.close_call_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            async fn get_active_connections(&self) -> Result<Vec<ConnectionInfo>> {
                Ok(vec![])
            }
        }

        #[derive(Clone)]
        pub struct MockHttpClient {
            pub status: http::StatusCode,
        }

        impl MockHttpClient {
            pub fn new(status: http::StatusCode) -> Self {
                Self { status }
            }
        }

        #[async_trait]
        impl HttpClientPort for MockHttpClient {
            async fn execute(
                &self,
                _request: &ProxyRequest,
                _route: &ProxyRoute,
                _credentials: Option<&Credentials>,
            ) -> Result<ProxyResponse> {
                Ok(ProxyResponse::new(self.status))
            }
        }
    }

    mod helpers {
        use super::*;

        pub fn create_proxy_request(method: ProxyMethod, url: &str) -> ProxyRequest {
            ProxyRequest::new(method, url.parse().unwrap())
        }

        pub fn create_connect_request(url: &str) -> ConnectRequest {
            ConnectRequest::new(url.parse().unwrap())
        }
    }

    mod handle_http_request_tests {
        use super::*;
        use helpers::*;
        use mocks::*;

        #[tokio::test]
        async fn direct_route() {
            let resolver = MockResolver::new(ProxyRoute::Direct);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker.clone()),
                Arc::new(http_client),
            );

            let request = create_proxy_request(ProxyMethod::Get, "http://example.com");
            let response = service.handle_http_request(&request).await.unwrap();

            assert_eq!(response.status, http::StatusCode::OK);
            assert_eq!(tracker.track_calls(), 1);
            assert_eq!(tracker.close_calls(), 1);
        }

        #[tokio::test]
        async fn different_http_methods() {
            let methods = vec![
                ProxyMethod::Get,
                ProxyMethod::Post,
                ProxyMethod::Put,
                ProxyMethod::Delete,
                ProxyMethod::Head,
            ];

            for method in methods {
                let resolver = MockResolver::new(ProxyRoute::Direct);
                let credentials = MockCredentials::new(None);
                let tracker = MockTracker::new();
                let http_client = MockHttpClient::new(http::StatusCode::OK);

                let service = ProxyService::new(
                    Arc::new(resolver),
                    Arc::new(credentials),
                    Arc::new(tracker.clone()),
                    Arc::new(http_client),
                );

                let request = create_proxy_request(method, "http://example.com");
                let response = service.handle_http_request(&request).await.unwrap();

                assert_eq!(response.status.as_u16(), 200);
                assert_eq!(tracker.track_calls(), 1);
            }
        }

        #[tokio::test]
        async fn various_status_codes() {
            let status_codes = vec![
                http::StatusCode::OK,
                http::StatusCode::CREATED,
                http::StatusCode::BAD_REQUEST,
                http::StatusCode::UNAUTHORIZED,
                http::StatusCode::NOT_FOUND,
                http::StatusCode::INTERNAL_SERVER_ERROR,
            ];

            for status in status_codes {
                let resolver = MockResolver::new(ProxyRoute::Direct);
                let credentials = MockCredentials::new(None);
                let tracker = MockTracker::new();
                let http_client = MockHttpClient::new(status.clone());

                let service = ProxyService::new(
                    Arc::new(resolver),
                    Arc::new(credentials),
                    Arc::new(tracker),
                    Arc::new(http_client),
                );

                let request = create_proxy_request(ProxyMethod::Get, "http://example.com");
                let response = service.handle_http_request(&request).await.unwrap();

                assert_eq!(response.status, status);
            }
        }

        #[tokio::test]
        async fn https_requests() {
            let resolver = MockResolver::new(ProxyRoute::Direct);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker),
                Arc::new(http_client),
            );

            let request = create_proxy_request(ProxyMethod::Get, "https://example.com/path");
            let response = service.handle_http_request(&request).await.unwrap();

            assert_eq!(response.status, http::StatusCode::OK);
        }

        #[tokio::test]
        async fn connection_tracking() {
            let resolver = MockResolver::new(ProxyRoute::Direct);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker.clone()),
                Arc::new(http_client),
            );

            assert_eq!(tracker.track_calls(), 0);
            assert_eq!(tracker.close_calls(), 0);

            let request = create_proxy_request(ProxyMethod::Get, "http://example.com");
            let _ = service.handle_http_request(&request).await.unwrap();

            assert_eq!(tracker.track_calls(), 1);
            assert_eq!(tracker.close_calls(), 1);
        }
    }

    mod handle_connect_request_tests {
        use super::*;
        use helpers::*;
        use mocks::*;

        #[tokio::test]
        async fn direct_route() {
            let resolver = MockResolver::new(ProxyRoute::Direct);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker.clone()),
                Arc::new(http_client),
            );

            let request = create_connect_request("https://example.com:443");
            let decision = service.handle_connect_request(&request).await.unwrap();

            match decision {
                ConnectDecision::Accept {
                    route,
                    credentials: _,
                    connection_id: _,
                } => {
                    assert!(route.is_direct());
                    assert_eq!(tracker.track_calls(), 1);
                }
                ConnectDecision::Rejected { .. } => panic!("Should not be rejected"),
            }
        }

        #[tokio::test]
        async fn blocked_route() {
            let blocked_route = ProxyRoute::Blocked {
                reason: "Test block reason".to_string(),
            };
            let resolver = MockResolver::new(blocked_route);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker.clone()),
                Arc::new(http_client),
            );

            let request = create_connect_request("https://example.com:443");
            let decision = service.handle_connect_request(&request).await.unwrap();

            match decision {
                ConnectDecision::Accept { .. } => panic!("Should be rejected"),
                ConnectDecision::Rejected { reason } => {
                    assert_eq!(reason, "Test block reason");
                    assert_eq!(tracker.track_calls(), 0);
                }
            }
        }

        #[tokio::test]
        async fn various_ports() {
            let ports = vec!["443", "8080", "3128", "9090"];

            for port in ports {
                let resolver = MockResolver::new(ProxyRoute::Direct);
                let credentials = MockCredentials::new(None);
                let tracker = MockTracker::new();
                let http_client = MockHttpClient::new(http::StatusCode::OK);

                let service = ProxyService::new(
                    Arc::new(resolver),
                    Arc::new(credentials),
                    Arc::new(tracker),
                    Arc::new(http_client),
                );

                let url = format!("https://example.com:{}", port);
                let request = create_connect_request(&url);
                let decision = service.handle_connect_request(&request).await.unwrap();

                match decision {
                    ConnectDecision::Accept { route, .. } => {
                        assert!(route.is_direct());
                    }
                    ConnectDecision::Rejected { .. } => panic!("Should not be rejected"),
                }
            }
        }

        #[tokio::test]
        async fn connection_id_returned() {
            let resolver = MockResolver::new(ProxyRoute::Direct);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker),
                Arc::new(http_client),
            );

            let request = create_connect_request("https://example.com:443");
            let decision = service.handle_connect_request(&request).await.unwrap();

            match decision {
                ConnectDecision::Accept { connection_id, .. } => {
                    assert_ne!(connection_id.as_bytes(), &[0u8; 16]);
                }
                ConnectDecision::Rejected { .. } => panic!("Should not be rejected"),
            }
        }
    }

    mod other_tests {
        use super::*;
        use mocks::*;

        #[tokio::test]
        async fn close_connection() {
            let resolver = MockResolver::new(ProxyRoute::Direct);
            let credentials = MockCredentials::new(None);
            let tracker = MockTracker::new();
            let http_client = MockHttpClient::new(http::StatusCode::OK);

            let service = ProxyService::new(
                Arc::new(resolver),
                Arc::new(credentials),
                Arc::new(tracker.clone()),
                Arc::new(http_client),
            );

            let conn_id = Uuid::new_v4();
            let result = service.close_connection(conn_id).await;

            assert!(result.is_ok());
            assert_eq!(tracker.close_calls(), 1);
        }

        #[tokio::test]
        async fn get_credentials_direct_route() {
            let service = ProxyService::new(
                Arc::new(MockResolver::new(ProxyRoute::Direct)),
                Arc::new(MockCredentials::new(None)),
                Arc::new(MockTracker::new()),
                Arc::new(MockHttpClient::new(http::StatusCode::OK)),
            );

            let creds = service.get_credentials_for_route(&ProxyRoute::Direct).await.unwrap();

            assert!(creds.is_none());
        }

        #[tokio::test]
        async fn get_credentials_blocked_route() {
            let service = ProxyService::new(
                Arc::new(MockResolver::new(ProxyRoute::Direct)),
                Arc::new(MockCredentials::new(None)),
                Arc::new(MockTracker::new()),
                Arc::new(MockHttpClient::new(http::StatusCode::OK)),
            );

            let blocked_route = ProxyRoute::Blocked {
                reason: "test".to_string(),
            };
            let creds = service.get_credentials_for_route(&blocked_route).await.unwrap();

            assert!(creds.is_none());
        }
    }
}
