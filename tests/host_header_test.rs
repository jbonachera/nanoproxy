mod e2e_utils;

use e2e_utils::{HeaderEchoServer, HostPreservingProxy, TestNanoproxyServer};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

#[cfg(test)]
#[tokio::test]
async fn test_host_header_preserved_on_direct_route() {
    let echo_server = HeaderEchoServer::new().await.expect("Failed to start echo server");
    let echo_addr = echo_server.local_addr();
    let _echo_handle = echo_server.run().await;

    let nanoproxy = TestNanoproxyServer::start(0, None)
        .await
        .expect("Failed to start nanoproxy");

    sleep(Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(nanoproxy.addr())
        .await
        .expect("Failed to connect to nanoproxy");

    let request = format!(
        "GET http://{}/ HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        echo_addr, echo_addr
    );
    stream.write_all(request.as_bytes()).await.expect("Failed to write request");

    let mut response_data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
            _ => break,
        }
    }

    let response = String::from_utf8_lossy(&response_data);
    let expected_host = format!("host: {}", echo_addr);
    assert!(
        response.contains(&expected_host),
        "Response body should contain '{}', got:\n{}",
        expected_host,
        response
    );
}

#[cfg(test)]
#[tokio::test]
async fn test_host_header_preserved_through_upstream_proxy() {
    let echo_server = HeaderEchoServer::new().await.expect("Failed to start echo server");
    let echo_addr = echo_server.local_addr();
    let _echo_handle = echo_server.run().await;

    let preserving_proxy = HostPreservingProxy::new().await.expect("Failed to start host-preserving proxy");
    let proxy_addr = preserving_proxy.local_addr();
    let _proxy_handle = preserving_proxy.run().await;

    sleep(Duration::from_millis(50)).await;

    let pac_script = format!(
        r#"function FindProxyForURL(url, host) {{ return "PROXY {}"; }}"#,
        proxy_addr
    );
    let pac_file = std::env::temp_dir().join(format!("nanoproxy_host_header_test_{}.pac", std::process::id()));
    std::fs::write(&pac_file, &pac_script).expect("Failed to write PAC file");

    let nanoproxy = TestNanoproxyServer::start(0, Some(&pac_file))
        .await
        .expect("Failed to start nanoproxy");

    sleep(Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(nanoproxy.addr())
        .await
        .expect("Failed to connect to nanoproxy");

    let request = format!(
        "GET http://{}/ HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        echo_addr, echo_addr
    );
    stream.write_all(request.as_bytes()).await.expect("Failed to write request");

    let mut response_data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
            _ => break,
        }
    }

    let response = String::from_utf8_lossy(&response_data);
    let expected_host = format!("host: {}", echo_addr);
    assert!(
        response.contains(&expected_host),
        "Response body should contain '{}', got:\n{}",
        expected_host,
        response
    );

    std::fs::remove_file(&pac_file).ok();
}

#[cfg(test)]
#[tokio::test]
async fn test_host_header_preserved_for_named_host_with_port() {
    let echo_server = HeaderEchoServer::new().await.expect("Failed to start echo server");
    let echo_addr = echo_server.local_addr();
    let _echo_handle = echo_server.run().await;

    let intercepting_proxy = HostPreservingProxy::new_with_target(echo_addr)
        .await
        .expect("Failed to start intercepting proxy");
    let proxy_addr = intercepting_proxy.local_addr();
    let _proxy_handle = intercepting_proxy.run().await;

    sleep(Duration::from_millis(50)).await;

    let pac_script = format!(
        r#"function FindProxyForURL(url, host) {{ return "PROXY {}"; }}"#,
        proxy_addr
    );
    let pac_file = std::env::temp_dir().join(format!("nanoproxy_named_host_test_{}.pac", std::process::id()));
    std::fs::write(&pac_file, &pac_script).expect("Failed to write PAC file");

    let nanoproxy = TestNanoproxyServer::start(0, Some(&pac_file))
        .await
        .expect("Failed to start nanoproxy");

    sleep(Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(nanoproxy.addr())
        .await
        .expect("Failed to connect to nanoproxy");

    let request = "GET http://app.internal.example.com:8080/api HTTP/1.1\r\nHost: app.internal.example.com:8080\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await.expect("Failed to write request");

    let mut response_data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => response_data.extend_from_slice(&chunk[..n]),
            _ => break,
        }
    }

    let response = String::from_utf8_lossy(&response_data);
    assert!(
        response.contains("host: app.internal.example.com:8080"),
        "Response body should contain 'host: app.internal.example.com:8080', got:\n{}",
        response
    );

    std::fs::remove_file(&pac_file).ok();
}
