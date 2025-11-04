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

    async fn get_credentials_for_route(&self, route: &ProxyRoute) -> Result<Option<Credentials>> {
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
    use url::Url;

    struct MockResolver;

    #[async_trait]
    impl ProxyResolverPort for MockResolver {
        async fn resolve_route(&self, _: &Url) -> Result<ProxyRoute> {
            Ok(ProxyRoute::Direct)
        }

        async fn resolve_all_routes(&self, _: &Url) -> Result<Vec<ProxyRoute>> {
            Ok(vec![ProxyRoute::Direct])
        }

        async fn update_pac_url(&self, _: Option<String>) -> Result<()> {
            Ok(())
        }
    }

    struct MockCredentials;

    #[async_trait]
    impl CredentialsPort for MockCredentials {
        async fn get_credentials(&self, _: &str) -> Result<Option<Credentials>> {
            Ok(None)
        }

        async fn clear_cache(&self) -> Result<()> {
            Ok(())
        }
    }

    struct MockTracker;

    #[async_trait]
    impl TrackingPort for MockTracker {
        async fn track_connection(&self, _: ConnectionInfo) -> Result<()> {
            Ok(())
        }

        async fn close_connection(&self, _: Uuid) -> Result<()> {
            Ok(())
        }

        async fn get_active_connections(&self) -> Result<Vec<ConnectionInfo>> {
            Ok(vec![])
        }
    }

    struct MockHttpClient;

    #[async_trait]
    impl HttpClientPort for MockHttpClient {
        async fn execute(
            &self,
            _request: &ProxyRequest,
            _route: &ProxyRoute,
            _credentials: Option<&Credentials>,
        ) -> Result<ProxyResponse> {
            use crate::domain::HttpStatus;
            Ok(ProxyResponse::new(HttpStatus::Ok))
        }
    }

    #[tokio::test]
    async fn test_handle_http_request_direct() {
        let service = ProxyService::new(
            Arc::new(MockResolver),
            Arc::new(MockCredentials),
            Arc::new(MockTracker),
            Arc::new(MockHttpClient),
        );

        let request = ProxyRequest::new(ProxyMethod::Get, "http://example.com".parse().unwrap());

        let response = service.handle_http_request(&request).await.unwrap();

        assert_eq!(response.status.as_u16(), 200);
    }

    #[tokio::test]
    async fn test_handle_connect_request() {
        let service = ProxyService::new(
            Arc::new(MockResolver),
            Arc::new(MockCredentials),
            Arc::new(MockTracker),
            Arc::new(MockHttpClient),
        );

        let request = ConnectRequest::new("https://example.com:443".parse().unwrap());

        let decision = service.handle_connect_request(&request).await.unwrap();

        match decision {
            ConnectDecision::Accept { route, .. } => {
                assert!(route.is_direct());
            }
            ConnectDecision::Rejected { .. } => panic!("Should not be rejected"),
        }
    }
}
