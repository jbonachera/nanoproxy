#![cfg(test)]
#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct HostPreservingProxy {
    listener: TcpListener,
    target_override: Option<SocketAddr>,
}

impl HostPreservingProxy {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        Ok(Self { listener, target_override: None })
    }

    pub async fn new_with_target(target: SocketAddr) -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        Ok(Self { listener, target_override: Some(target) })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap()
    }

    pub async fn run(self) -> JoinHandle<()> {
        let target_override = self.target_override;
        tokio::spawn(async move {
            loop {
                match self.listener.accept().await {
                    Ok((socket, _)) => {
                        tokio::spawn(Self::handle_connection(socket, target_override));
                    }
                    Err(_) => break,
                }
            }
        })
    }

    async fn handle_connection(mut socket: TcpStream, target_override: Option<SocketAddr>) {
        let mut buffer = vec![0; 4096];
        match socket.read(&mut buffer).await {
            Ok(n) if n > 0 => {
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                let lines: Vec<&str> = request.lines().collect();
                if let Some(first_line) = lines.first() {
                    let parts: Vec<&str> = first_line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let (target_host, target_port, path) = match target_override {
                            Some(addr) => (addr.ip().to_string(), addr.port().to_string(), extract_path(parts[1])),
                            None => parse_absolute_url(parts[1]),
                        };
                        forward_request(&mut socket, &request, &target_host, &target_port, &path).await;
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_path(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("http://") {
        if let Some(slash_pos) = rest.find('/') {
            return rest[slash_pos..].to_string();
        }
    }
    "/".to_string()
}

fn parse_absolute_url(url: &str) -> (String, String, String) {
    if let Some(rest) = url.strip_prefix("http://") {
        if let Some(slash_pos) = rest.find('/') {
            let (host_port, path) = rest.split_at(slash_pos);
            if let Some(colon_pos) = host_port.find(':') {
                let (h, p) = host_port.split_at(colon_pos);
                return (h.to_string(), p[1..].to_string(), path.to_string());
            }
            return (host_port.to_string(), "80".to_string(), path.to_string());
        }
        if let Some(colon_pos) = rest.find(':') {
            let (h, p) = rest.split_at(colon_pos);
            return (h.to_string(), p[1..].to_string(), "/".to_string());
        }
        return (rest.to_string(), "80".to_string(), "/".to_string());
    }
    (String::new(), "80".to_string(), url.to_string())
}

async fn forward_request(socket: &mut TcpStream, request: &str, host: &str, port: &str, path: &str) {
    match TcpStream::connect(format!("{}:{}", host, port)).await {
        Ok(mut target_stream) => {
            let forwarded = to_origin_form(request, path);
            if target_stream.write_all(forwarded.as_bytes()).await.is_ok() {
                let mut response_data = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), target_stream.read(&mut chunk)).await {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
                        _ => break,
                    }
                }
                let _ = socket.write_all(&response_data).await;
            }
        }
        Err(_) => {
            let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        }
    }
}

fn to_origin_form(request: &str, path: &str) -> String {
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let first_line_parts: Vec<&str> = lines[0].split_whitespace().collect();
    if first_line_parts.len() < 2 {
        return request.to_string();
    }

    let method = first_line_parts[0];
    let version = if first_line_parts.len() >= 3 { first_line_parts[2] } else { "HTTP/1.1" };

    let mut result = format!("{} {} {}\r\n", method, path, version);

    for line in lines.iter().skip(1) {
        if line.is_empty() {
            break;
        }
        result.push_str(line);
        result.push_str("\r\n");
    }

    result.push_str("\r\n");
    result
}
