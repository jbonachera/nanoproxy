#![cfg(test)]
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

pub struct IntermediateProxy {
    listener: TcpListener,
    shutdown: Arc<AtomicBool>,
}

impl IntermediateProxy {
    pub async fn new(port: u16) -> Result<Self, Box<dyn std::error::Error>> {
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            listener,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    pub async fn run(self) -> JoinHandle<()> {
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                match tokio::time::timeout(Duration::from_millis(100), self.listener.accept()).await {
                    Ok(Ok((socket, _addr))) => {
                        tokio::spawn(Self::handle_connection(socket));
                    }
                    Ok(Err(_)) => break,
                    Err(_) => continue,
                }
            }
        })
    }

    async fn handle_connection(mut socket: TcpStream) {
        let mut buffer = vec![0; 4096];
        match socket.read(&mut buffer).await {
            Ok(n) if n > 0 => {
                let request = String::from_utf8_lossy(&buffer[..n]);
                eprintln!("[IntermediateProxy] Received request:\n{}", request);
                let lines: Vec<&str> = request.lines().collect();

                if let Some(first_line) = lines.first() {
                    let parts: Vec<&str> = first_line.split_whitespace().collect();

                    if parts.len() >= 2 && parts[0] == "CONNECT" {
                        Self::handle_connect(&mut socket, parts[1]).await;
                    } else if parts.len() >= 2
                        && (parts[0] == "GET"
                            || parts[0] == "POST"
                            || parts[0] == "PUT"
                            || parts[0] == "DELETE"
                            || parts[0] == "HEAD")
                    {
                        Self::handle_http_request(&mut socket, &request, first_line).await;
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_connect(socket: &mut TcpStream, target: &str) {
        let (host, port) = if let Some(colon_pos) = target.rfind(':') {
            let (h, p) = target.split_at(colon_pos);
            (h, &p[1..])
        } else {
            (target, "443")
        };

        eprintln!("[IntermediateProxy] CONNECT to {}:{}", host, port);
        match TcpStream::connect(format!("{}:{}", host, port)).await {
            Ok(mut target_stream) => {
                // Send 200 response
                let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
                if socket.write_all(response).await.is_ok() {
                    // Relay data bidirectionally
                    Self::relay(socket, &mut target_stream).await;
                }
            }
            Err(_) => {
                let response = b"HTTP/1.1 502 Bad Gateway\r\n\r\n";
                let _ = socket.write_all(response).await;
            }
        }
    }

    async fn handle_http_request(socket: &mut TcpStream, request: &str, first_line: &str) {
        let parts: Vec<&str> = first_line.split_whitespace().collect();
        if parts.len() < 2 {
            return;
        }

        let url_str = parts[1];
        let (host, port, path) = Self::parse_http_url(url_str);

        eprintln!("[IntermediateProxy] HTTP {} to {}:{}{}", parts[0], host, port, path);

        match TcpStream::connect(format!("{}:{}", host, port)).await {
            Ok(mut target_stream) => {
                // Reconstruct the request with the correct host and path
                let http_request = Self::reconstruct_http_request(request, &path, &host);
                if target_stream.write_all(http_request.as_bytes()).await.is_ok() {
                    // Read the response from the target server
                    let mut response_data = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        match tokio::time::timeout(Duration::from_secs(5), target_stream.read(&mut chunk)).await {
                            Ok(Ok(0)) => break,
                            Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
                            Ok(Err(_)) => break,
                            Err(_) => break,
                        }
                    }

                    // Send response back to client
                    let _ = socket.write_all(&response_data).await;
                }
            }
            Err(_) => {
                let response = b"HTTP/1.1 502 Bad Gateway\r\n\r\n";
                let _ = socket.write_all(response).await;
            }
        }
    }

    fn parse_http_url(url: &str) -> (String, String, String) {
        // Parse URLs like http://example.com/path or just /path
        let (host, port, path) = if let Some(url) = url.strip_prefix("http://") {
            if let Some(slash_pos) = url.find('/') {
                let (host_port, path) = url.split_at(slash_pos);
                if let Some(colon_pos) = host_port.find(':') {
                    let (h, p) = host_port.split_at(colon_pos);
                    (h.to_string(), p[1..].to_string(), path.to_string())
                } else {
                    (host_port.to_string(), "80".to_string(), path.to_string())
                }
            } else if let Some(colon_pos) = url.find(':') {
                let (h, p) = url.split_at(colon_pos);
                (h.to_string(), p[1..].to_string(), "/".to_string())
            } else {
                (url.to_string(), "80".to_string(), "/".to_string())
            }
        } else if let Some(url) = url.strip_prefix("https://") {
            if let Some(slash_pos) = url.find('/') {
                let (host_port, path) = url.split_at(slash_pos);
                if let Some(colon_pos) = host_port.find(':') {
                    let (h, p) = host_port.split_at(colon_pos);
                    (h.to_string(), p[1..].to_string(), path.to_string())
                } else {
                    (host_port.to_string(), "443".to_string(), path.to_string())
                }
            } else if let Some(colon_pos) = url.find(':') {
                let (h, p) = url.split_at(colon_pos);
                (h.to_string(), p[1..].to_string(), "/".to_string())
            } else {
                (url.to_string(), "443".to_string(), "/".to_string())
            }
        } else {
            (String::new(), "80".to_string(), url.to_string())
        };

        (host, port, path)
    }

    fn reconstruct_http_request(original_request: &str, path: &str, host: &str) -> String {
        let lines: Vec<&str> = original_request.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let first_line_parts: Vec<&str> = lines[0].split_whitespace().collect();
        if first_line_parts.len() < 2 {
            return original_request.to_string();
        }

        let method = first_line_parts[0];
        let http_version = if first_line_parts.len() >= 3 {
            first_line_parts[2]
        } else {
            "HTTP/1.1"
        };

        let mut reconstructed = format!("{} {} {}\r\n", method, path, http_version);

        // Copy headers, updating Host if present
        let mut host_found = false;
        for line in &lines[1..] {
            if line.is_empty() {
                break;
            }
            if line.starts_with("Host:") || line.starts_with("host:") {
                reconstructed.push_str(&format!("Host: {}\r\n", host));
                host_found = true;
            } else {
                reconstructed.push_str(line);
                reconstructed.push_str("\r\n");
            }
        }

        if !host_found {
            reconstructed.push_str(&format!("Host: {}\r\n", host));
        }

        reconstructed.push_str("\r\n");
        reconstructed
    }

    async fn relay(client: &mut TcpStream, server: &mut TcpStream) {
        let (mut client_read, mut client_write) = client.split();
        let (mut server_read, mut server_write) = server.split();

        let client_to_server = async {
            let mut buf = [0; 4096];
            loop {
                match client_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if server_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        let server_to_client = async {
            let mut buf = [0; 4096];
            loop {
                match server_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if client_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        tokio::select! {
            _ = client_to_server => {},
            _ = server_to_client => {},
        }
    }
}
