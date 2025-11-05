mod e2e_utils;

use e2e_utils::nanoproxy_server::TestNanoproxyServer;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

#[cfg(test)]
#[tokio::test]
async fn test_direct_https_connect_to_google() {
    let nanoproxy = TestNanoproxyServer::start(18887, None)
        .await
        .expect("Failed to start nanoproxy");

    sleep(Duration::from_millis(200)).await;

    let mut stream = TcpStream::connect(nanoproxy.addr())
        .await
        .expect("Should be able to connect to nanoproxy");

    let connect_request = "CONNECT google.com:443 HTTP/1.1\r\nHost: google.com:443\r\n\r\n";
    stream
        .write_all(connect_request.as_bytes())
        .await
        .expect("Should be able to write CONNECT request");

    let mut buffer = [0u8; 1024];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .expect("Should receive response within timeout")
        .expect("Should be able to read response");

    assert!(n > 0, "Response should not be empty");

    let response = String::from_utf8_lossy(&buffer[..n]);
    assert!(
        response.contains("200"),
        "Expected 200 OK for direct CONNECT, got: {}",
        response
    );
}
