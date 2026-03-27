#![cfg(test)]
#![allow(dead_code)]

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct HeaderEchoServer {
    listener: TcpListener,
}

impl HeaderEchoServer {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        Ok(Self { listener })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap()
    }

    pub async fn run(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match self.listener.accept().await {
                    Ok((socket, _)) => {
                        tokio::spawn(Self::handle_connection(socket));
                    }
                    Err(_) => break,
                }
            }
        })
    }

    async fn handle_connection(mut socket: TcpStream) {
        let mut buffer = vec![0; 4096];
        match socket.read(&mut buffer).await {
            Ok(n) if n > 0 => {
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                let body = collect_headers_as_body(&request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
            _ => {}
        }
    }
}

fn collect_headers_as_body(request: &str) -> String {
    request
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .map(|line| format!("{}\n", line.to_lowercase()))
        .collect()
}
