use async_trait::async_trait;
use http_body_util::{combinators::BoxBody, BodyExt, Empty};
use hyper::{body::Bytes, header::HeaderValue, Method, Request};
use hyper_util::client::legacy::Client;
use std::collections::HashMap;

use super::connector::HyperConnector;
use crate::domain::{
    Credentials, HttpStatus, ProxyError, ProxyMethod, ProxyRequest, ProxyResponse, ProxyRoute, Result,
};
use crate::ports::HttpClientPort;

type Body = BoxBody<Bytes, hyper::Error>;

pub struct HyperHttpClient {
    connector: HyperConnector,
    client: Client<HyperConnector, Body>,
}

impl HyperHttpClient {
    pub fn new(connector: HyperConnector, client: Client<HyperConnector, Body>) -> Self {
        Self { connector, client }
    }

    fn build_hyper_request(
        &self,
        domain_req: &ProxyRequest,
        route: &ProxyRoute,
        credentials: Option<&Credentials>,
    ) -> Result<Request<Body>> {
        let method = convert_method(&domain_req.method);
        let uri: hyper::Uri = domain_req
            .target_url
            .as_str()
            .parse()
            .map_err(|e| ProxyError::InvalidUri(format!("{}", e)))?;

        let mut req = Request::builder().method(method).uri(&uri);

        for (key, value) in &domain_req.headers {
            if let Ok(header_name) = key.parse::<hyper::header::HeaderName>() {
                if let Ok(header_value) = value.parse::<HeaderValue>() {
                    req = req.header(header_name, header_value);
                }
            }
        }

        if let ProxyRoute::Upstream { .. } = route {
            if let Some(creds) = credentials {
                req = req.header(
                    hyper::http::header::PROXY_AUTHORIZATION,
                    HeaderValue::from_str(&creds.to_basic_auth())
                        .map_err(|e| ProxyError::InvalidRequest(format!("{}", e)))?,
                );
            }
        }

        let body = Empty::<Bytes>::new().map_err(|never| match never {}).boxed();
        req.body(body).map_err(|e| ProxyError::InvalidRequest(format!("{}", e)))
    }
}

#[async_trait]
impl HttpClientPort for HyperHttpClient {
    async fn execute(
        &self,
        request: &ProxyRequest,
        route: &ProxyRoute,
        credentials: Option<&Credentials>,
    ) -> Result<ProxyResponse> {
        let hyper_req = self.build_hyper_request(request, route, credentials)?;

        self.connector.set_route_for_uri(hyper_req.uri(), route.clone());

        let hyper_resp = self
            .client
            .request(hyper_req)
            .await
            .map_err(|e| ProxyError::ConnectionFailed(format!("{}", e)))?;

        let status = HttpStatus::from_u16(hyper_resp.status().as_u16());

        let headers: HashMap<String, String> = hyper_resp
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
            .collect();

        let body_bytes = hyper_resp
            .into_body()
            .collect()
            .await
            .map_err(|e| ProxyError::ConnectionFailed(format!("{}", e)))?
            .to_bytes()
            .to_vec();

        Ok(ProxyResponse::new(status).with_headers(headers).with_body(body_bytes))
    }
}

fn convert_method(method: &ProxyMethod) -> Method {
    match method {
        ProxyMethod::Get => Method::GET,
        ProxyMethod::Post => Method::POST,
        ProxyMethod::Put => Method::PUT,
        ProxyMethod::Delete => Method::DELETE,
        ProxyMethod::Head => Method::HEAD,
        ProxyMethod::Options => Method::OPTIONS,
        ProxyMethod::Connect => Method::CONNECT,
        ProxyMethod::Patch => Method::PATCH,
        ProxyMethod::Trace => Method::TRACE,
        ProxyMethod::Other(s) => Method::from_bytes(s.as_bytes()).unwrap_or(Method::GET),
    }
}
