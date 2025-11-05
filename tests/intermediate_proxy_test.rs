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

    // Open CONNECT session to httpbin.org:80
    let connect_request = "CONNECT httpbin.org:80 HTTP/1.1\r\nHost: httpbin.org:80\r\n\r\n";
    stream
        .write_all(connect_request.as_bytes())
        .await
        .expect("Should be able to write CONNECT request");

    let mut buffer = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .expect("Should receive response within timeout")
        .expect("Should be able to read response");

    assert!(n > 0, "Response should not be empty");

    let response = String::from_utf8_lossy(&buffer[..n]);
    println!("CONNECT response: {}", response);

    assert!(response.contains("200"), "Expected 200 OK, got: {}", response);

    // Now make an HTTP request through the tunnel to verify we're talking to the right host
    let http_request = "GET /headers HTTP/1.1\r\nHost: httpbin.org\r\nConnection: close\r\n\r\n";
    stream
        .write_all(http_request.as_bytes())
        .await
        .expect("Should be able to write HTTP request through tunnel");

    // Read the HTTP response
    let mut response_data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break, // EOF
            Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
            Ok(Err(e)) => panic!("Read error: {}", e),
            Err(_) => break, // Timeout, we got all data
        }
    }

    let http_response = String::from_utf8_lossy(&response_data);
    println!("HTTP response:\n{}", http_response);

    // Verify we got a valid HTTP response
    assert!(
        http_response.contains("HTTP/1.1 200 OK") || http_response.contains("HTTP/1.1"),
        "Should receive valid HTTP response, got: {}",
        http_response
    );

    // Verify we're talking to httpbin.org
    assert!(
        http_response.contains("\"Host\": \"httpbin.org\""),
        "Response should be from httpbin.org and contain Host header, got: {}",
        http_response
    );
}
