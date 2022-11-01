use std::{collections::HashMap, io};

use async_trait::async_trait;
use libproxy::ProxyFactory;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use url::Url;

use crate::pac::proxy_for_url;

#[async_trait]
trait HTTPDialer {
    async fn dial(
        &self,
        method: &String,
        path: &String,
        headers: &mut HashMap<String, String>,
    ) -> io::Result<TcpStream>;
}

#[derive(Clone, Copy)]
struct DirectLink {}

#[async_trait]
impl HTTPDialer for DirectLink {
    async fn dial(
        &self,
        method: &String,
        path: &String,
        headers: &mut HashMap<String, String>,
    ) -> io::Result<TcpStream> {
        let host_header = headers.get("host");
        let host = host_header.unwrap_or(&path.to_string()).to_string();
        let remote_port: &str = {
            if host.contains(":") {
                ""
            } else {
                ":80"
            }
        };
        let mut conn = TcpStream::connect(format!("{}{}", host, remote_port)).await?;
        let headers_str = serialize_headers(&headers);

        if method != "CONNECT" {
            conn.write(format!("{method} {path} HTTP/1.1\r\n{headers_str}\r\n\r\n").as_bytes())
                .await
                .map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("failed to send HTTP headers to {host}: {err}"),
                    )
                })?;
        }
        Ok(conn)
    }
}
#[derive(Clone)]
struct UpstreamDialer {
    pub upstream: String,
    upstream_credentials: Option<String>,
}

fn encode_credentials(username: &str, password: &str) -> String {
    format!("Basic {}", base64::encode(format!("{username}:{password}")))
}

impl UpstreamDialer {
    pub fn new<T>(host: T) -> Self
    where
        T: Into<String>,
    {
        UpstreamDialer {
            upstream: host.into(),
            upstream_credentials: None,
        }
    }
    pub fn set_credentials(&mut self, username: &str, password: &str) {
        let creds: String = encode_credentials(username, password);
        self.upstream_credentials = Some(creds);
    }
}

#[async_trait]
impl HTTPDialer for UpstreamDialer {
    async fn dial(
        &self,
        method: &String,
        path: &String,
        headers: &mut HashMap<String, String>,
    ) -> io::Result<TcpStream> {
        let mut conn = TcpStream::connect(&self.upstream).await?;
        match &self.upstream_credentials {
            Some(creds) => {
                headers.insert("Proxy-Authorization".to_string(), creds.to_string());
            }
            None => {}
        }
        let headers_str = serialize_headers(&headers);
        conn.write(format!("{method} {path} HTTP/1.1\r\n{headers_str}\r\n\r\n").as_bytes())
            .await
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("failed to send HTTP headers to upstream host: {err}"),
                )
            })?;
        if method == "CONNECT" {
            let mut res = String::new();
            let mut reader = BufReader::new(conn);

            reader.read_line(&mut res).await?;
            println!("{res}");

            Ok(reader.into_inner())
        } else {
            Ok(conn)
        }
    }
}

fn serialize_headers(headers: &HashMap<String, String>) -> String {
    headers
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<String>>()
        .join("\r\n")
}

pub async fn dial(
    factory: ProxyFactory,
    method: &String,
    path: &String,
    headers: &mut HashMap<String, String>,
) -> io::Result<(String, TcpStream)> {
    if !headers.contains_key("host") {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("missing Host header"),
        ));
    }
    let host_header = headers.get("host");
    let remote = host_header.unwrap_or(&path.to_string()).to_string();

    let (remote_host, remote_port) = {
        if remote.contains(":") {
            remote.split_once(":").unwrap()
        } else {
            (remote.as_str(), "80")
        }
    };

    let url_for_proxy = match remote_port {
        "443" => format!("https://{remote_host}:443/"),
        _ => format!("http://{remote_host}:443/"),
    };

    let proxies = proxy_for_url(
        "http://pac.mud.arkea.com:8080/sc/proxy.pac"
            .parse()
            .unwrap(),
        &url_for_proxy.parse().unwrap(),
    )
    .await
    .unwrap();

    let upstream_url = Url::parse(&proxies).map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to parse proxy URL {proxies}: {err}"),
        )
    })?;
    match upstream_url.scheme() {
        "http" => {
            let mut proxy = UpstreamDialer::new(format!(
                "{}:{}",
                upstream_url.host_str().unwrap(),
                upstream_url.port().unwrap()
            ));
            proxy.set_credentials("f0332", "toto1247");

            println!(
                "using upstream with injected credentials {}",
                proxy.upstream
            );

            return proxy
                .dial(method, path, headers)
                .await
                .map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("failed to dial {remote}: {err}"),
                    )
                })
                .map(|v| (remote, v));
        }
        _ => {
            let proxy = DirectLink {};
            return proxy
                .dial(method, path, headers)
                .await
                .map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("failed to dial {remote}: {err}"),
                    )
                })
                .map(|v| (remote, v));
        }
    }
}
