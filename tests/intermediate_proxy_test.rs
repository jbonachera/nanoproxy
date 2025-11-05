mod e2e_utils;

use e2e_utils::intermediate_proxy::IntermediateProxy;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

#[cfg(test)]
#[tokio::test]
async fn test_intermediate_proxy_direct_connect_to_public_host() {
    let proxy = IntermediateProxy::new(19994)
        .await
        .expect("Failed to create intermediate proxy");

    let proxy_addr = proxy.local_addr().expect("Failed to get proxy address");
    let _handle = proxy.run().await;

    sleep(Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(proxy_addr)
        .await
        .expect("Should be able to connect to intermediate proxy");

    let connect_request = "CONNECT ifconfig.me:443 HTTP/1.1\r\nHost: ifconfig.me:443\r\n\r\n";
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
        response.contains("200") || response.contains("502"),
        "Expected 200 OK or 502 Bad Gateway, got: {}",
        response
    );

    if response.contains("200") {
        stream
            .write_all(&[0x16, 0x03, 0x01])
            .await
            .expect("Tunnel should be open and writable");
    }
}
