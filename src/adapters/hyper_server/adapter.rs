use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::body::Bytes;
use hyper::{body::Incoming, header::HeaderValue, Method, Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tracing::error;
use url::Url;

use super::connector::HyperConnector;
use crate::domain::{ConnectDecision, ConnectRequest, ProxyError, ProxyMethod, ProxyRequest, ProxyRoute, ProxyService};

type Body = BoxBody<Bytes, hyper::Error>;

pub struct HyperProxyAdapter {
    service: Arc<ProxyService>,
    client: Client<HyperConnector, Body>,
}

impl HyperProxyAdapter {
    pub fn new(service: Arc<ProxyService>, client: Client<HyperConnector, Body>) -> Self {
        Self { service, client }
    }

    pub async fn handle(&self, req: Request<Incoming>) -> Response<Body> {
        self.handle_internal(req).await.unwrap_or_else(|e| {
            error!("Proxy error: {}", e);
            Response::builder()
                .status(500)
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap()
        })
    }

    async fn handle_internal(&self, req: Request<Incoming>) -> Result<Response<Body>, Box<dyn std::error::Error>> {
        if req.method() == Method::CONNECT {
            self.handle_connect(req).await
        } else {
            self.handle_http(req).await
        }
    }

    async fn handle_http(&self, req: Request<Incoming>) -> Result<Response<Body>, Box<dyn std::error::Error>> {
        let proxy_req = self.convert_to_domain_request(&req)?;

        let domain_response = self.service.handle_http_request(&proxy_req).await?;

        let mut hyper_response = Response::builder().status(domain_response.status.as_u16());

        for (key, value) in domain_response.headers {
            if let Ok(header_name) = key.parse::<hyper::header::HeaderName>() {
                if let Ok(header_value) = value.parse::<hyper::header::HeaderValue>() {
                    hyper_response = hyper_response.header(header_name, header_value);
                }
            }
        }

        let body = Full::new(Bytes::from(domain_response.body))
            .map_err(|never| match never {})
            .boxed();

        Ok(hyper_response.body(body).unwrap())
    }

    async fn handle_connect(&self, req: Request<Incoming>) -> Result<Response<Body>, Box<dyn std::error::Error>> {
        let target_url = extract_target_url(&req)?;
        let headers = extract_headers(&req);
        let connect_req = ConnectRequest::new(target_url).with_headers(headers);

        let decision = self.service.handle_connect_request(&connect_req).await?;

        match decision {
            ConnectDecision::Rejected { reason } => {
                error!("CONNECT rejected: {}", reason);
                Ok(Response::builder()
                    .status(403)
                    .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                    .unwrap())
            }
            ConnectDecision::Accept {
                route,
                credentials,
                connection_id,
            } => {
                establish_tunnel(
                    req,
                    route,
                    credentials,
                    connection_id,
                    self.service.clone(),
                    self.client.clone(),
                )
                .await
            }
        }
    }

    fn convert_to_domain_request(&self, req: &Request<Incoming>) -> Result<ProxyRequest, Box<dyn std::error::Error>> {
        let method = convert_method(req.method());
        let target_url = extract_target_url(req)?;
        let headers = extract_headers(req);

        Ok(ProxyRequest::new(method, target_url).with_headers(headers))
    }
}

fn convert_method(method: &Method) -> ProxyMethod {
    match method {
        &Method::GET => ProxyMethod::Get,
        &Method::POST => ProxyMethod::Post,
        &Method::PUT => ProxyMethod::Put,
        &Method::DELETE => ProxyMethod::Delete,
        &Method::HEAD => ProxyMethod::Head,
        &Method::OPTIONS => ProxyMethod::Options,
        &Method::CONNECT => ProxyMethod::Connect,
        &Method::PATCH => ProxyMethod::Patch,
        &Method::TRACE => ProxyMethod::Trace,
        other => ProxyMethod::Other(other.to_string()),
    }
}

fn extract_target_url(req: &Request<Incoming>) -> Result<Url, ProxyError> {
    if req.method() == Method::CONNECT {
        if let Some(authority) = req.uri().authority() {
            let url_str = format!("https://{}/", authority);
            return url_str
                .parse()
                .map_err(|e| ProxyError::InvalidUri(format!("Invalid CONNECT URI: {}", e)));
        }
    }

    let host_header = req
        .headers()
        .get("host")
        .ok_or(ProxyError::MissingHost)?
        .to_str()
        .map_err(|e| ProxyError::InvalidRequest(format!("Invalid host header: {}", e)))?;

    let url_str = format!("http://{}/", host_header);
    url_str
        .parse()
        .map_err(|e| ProxyError::InvalidUri(format!("Invalid host header URL: {}", e)))
}

fn extract_headers(req: &Request<Incoming>) -> HashMap<String, String> {
    req.headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
        .collect()
}

async fn establish_tunnel(
    req: Request<Incoming>,
    route: ProxyRoute,
    credentials: Option<crate::domain::Credentials>,
    connection_id: uuid::Uuid,
    service: Arc<ProxyService>,
    client: Client<HyperConnector, Body>,
) -> Result<Response<Body>, Box<dyn std::error::Error>> {
    match route {
        ProxyRoute::Direct => {
            let remote_host = req
                .uri()
                .authority()
                .map(|a| a.to_string())
                .ok_or("Missing authority")?;

            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        let mut upgraded = TokioIo::new(upgraded);
                        match TcpStream::connect(&remote_host).await {
                            Ok(mut server) => {
                                if let Err(e) = tokio::io::copy_bidirectional(&mut upgraded, &mut server).await {
                                    error!("Tunnel error for {}: {}", remote_host, e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to connect to {}: {}", remote_host, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to upgrade to CONNECT: {}", e);
                    }
                }
                service.close_connection(connection_id).await.ok();
            });

            Ok(Response::builder()
                .status(200)
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap())
        }
        ProxyRoute::Upstream { .. } => {
            let mut server_req = Request::connect(req.uri())
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap();

            if let Some(creds) = credentials {
                server_req.headers_mut().insert(
                    hyper::http::header::PROXY_AUTHORIZATION,
                    HeaderValue::from_str(&creds.to_basic_auth())?,
                );
            }

            tokio::spawn(async move {
                match client.request(server_req).await {
                    Ok(res) => match hyper::upgrade::on(res).await {
                        Ok(server) => match hyper::upgrade::on(req).await {
                            Ok(upgraded) => {
                                let mut upgraded = TokioIo::new(upgraded);
                                let mut server = TokioIo::new(server);
                                if let Err(e) = tokio::io::copy_bidirectional(&mut upgraded, &mut server).await {
                                    error!("Tunnel error: {}", e);
                                }
                            }
                            Err(e) => error!("Client refused to upgrade: {}", e),
                        },
                        Err(e) => error!("Server refused to upgrade: {}", e),
                    },
                    Err(e) => error!("Failed to connect to upstream proxy: {}", e),
                }
                service.close_connection(connection_id).await.ok();
            });

            Ok(Response::builder()
                .status(200)
                .body(Empty::<Bytes>::new().map_err(|never| match never {}).boxed())
                .unwrap())
        }
        ProxyRoute::Blocked { reason } => Err(format!("Route blocked: {}", reason).into()),
    }
}
