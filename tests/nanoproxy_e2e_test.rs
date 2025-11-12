mod e2e_utils;

use e2e_utils::{IntermediateProxy, TestNanoproxyServer};
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

#[cfg(test)]
#[tokio::test]
async fn test_connect_request_through_upstream_proxy_to_correct_host() {
    // Start an intermediate proxy
    let intermediate = IntermediateProxy::new(19996)
        .await
        .expect("Failed to create intermediate proxy");
    let intermediate_addr = intermediate
        .local_addr()
        .expect("Failed to get intermediate proxy address");
    let _intermediate_handle = intermediate.run().await;

    sleep(Duration::from_millis(100)).await;

    // Create PAC file that routes through the intermediate proxy
    let pac_script = format!(
        r#"
function FindProxyForURL(url, host) {{
    return "PROXY {}";
}}
"#,
        intermediate_addr
    );
    let pac_file = std::env::temp_dir().join(format!("nanoproxy_test_host_validation_{}.pac", std::process::id()));
    std::fs::write(&pac_file, &pac_script).expect("Failed to write PAC file");

    // Start nanoproxy with the PAC file
    let nanoproxy = TestNanoproxyServer::start(18886, Some(&pac_file))
        .await
        .expect("Failed to start nanoproxy");

    sleep(Duration::from_millis(200)).await;

    // Connect to nanoproxy
    let mut stream = TcpStream::connect(nanoproxy.addr())
        .await
        .expect("Should be able to connect to nanoproxy");

    // Request CONNECT to httpbin.org:80
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

    // Now make an HTTP request through the tunnel to verify we're talking to httpbin.org
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

    // Verify we're talking to httpbin.org by checking for its characteristic response
    assert!(
        http_response.contains("\"Host\": \"httpbin.org\""),
        "Response should be from httpbin.org, got: {}",
        http_response
    );

    // Clean up
    std::fs::remove_file(&pac_file).ok();
}

#[cfg(test)]
#[tokio::test]
async fn test_http_request_through_upstream_proxy() {
    // Start an intermediate proxy
    let intermediate = IntermediateProxy::new(19997)
        .await
        .expect("Failed to create intermediate proxy");
    let intermediate_addr = intermediate
        .local_addr()
        .expect("Failed to get intermediate proxy address");
    let _intermediate_handle = intermediate.run().await;

    sleep(Duration::from_millis(100)).await;

    // Create PAC file that routes through the intermediate proxy
    let pac_script = format!(
        r#"
function FindProxyForURL(url, host) {{
    return "PROXY {}";
}}
"#,
        intermediate_addr
    );
    let pac_file = std::env::temp_dir().join(format!("nanoproxy_test_http_request_{}.pac", std::process::id()));
    std::fs::write(&pac_file, &pac_script).expect("Failed to write PAC file");

    // Start nanoproxy with the PAC file
    let nanoproxy = TestNanoproxyServer::start(18885, Some(&pac_file))
        .await
        .expect("Failed to start nanoproxy");

    sleep(Duration::from_millis(200)).await;

    // Connect to nanoproxy
    let mut stream = TcpStream::connect(nanoproxy.addr())
        .await
        .expect("Should be able to connect to nanoproxy");

    // Send an HTTP GET request (not HTTPS/CONNECT) with full URL
    let http_request = "GET http://httpbin.org/headers HTTP/1.1\r\nHost: httpbin.org\r\nConnection: close\r\n\r\n";
    stream
        .write_all(http_request.as_bytes())
        .await
        .expect("Should be able to write HTTP request");

    // Read the HTTP response
    let mut response_data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break, // EOF
            Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
            Ok(Err(e)) => panic!("Read error: {}", e),
            Err(_) => break, // Timeout
        }
    }

    let http_response = String::from_utf8_lossy(&response_data);
    println!("HTTP response:\n{}", http_response);

    // Verify we got a valid HTTP response
    assert!(
        http_response.contains("HTTP/1.1 200") || http_response.contains("HTTP/1."),
        "Should receive valid HTTP response, got: {}",
        http_response
    );

    // Verify we're talking to httpbin.org by checking for its characteristic response
    assert!(
        http_response.contains("\"Host\": \"httpbin.org\"") || http_response.contains("httpbin"),
        "Response should be from httpbin.org, got: {}",
        http_response
    );

    // Clean up
    std::fs::remove_file(&pac_file).ok();
}
