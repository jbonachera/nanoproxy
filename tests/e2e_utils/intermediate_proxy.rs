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
        let mut buffer = vec![0; 1024];
        match socket.read(&mut buffer).await {
            Ok(n) if n > 0 => {
                let request = String::from_utf8_lossy(&buffer[..n]);
                let lines: Vec<&str> = request.lines().collect();

                if let Some(first_line) = lines.first() {
                    let parts: Vec<&str> = first_line.split_whitespace().collect();
                    if parts.len() >= 2 && parts[0] == "CONNECT" {
                        let target = parts[1];

                        let (host, port) = if let Some(colon_pos) = target.rfind(':') {
                            let (h, p) = target.split_at(colon_pos);
                            (h, &p[1..])
                        } else {
                            (target, "443")
                        };

                        match TcpStream::connect(format!("{}:{}", host, port)).await {
                            Ok(mut target_stream) => {
                                // Send 200 response
                                let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
                                if socket.write_all(response).await.is_ok() {
                                    // Relay data bidirectionally
                                    Self::relay(&mut socket, &mut target_stream).await;
                                }
                            }
                            Err(_) => {
                                let response = b"HTTP/1.1 502 Bad Gateway\r\n\r\n";
                                let _ = socket.write_all(response).await;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
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
