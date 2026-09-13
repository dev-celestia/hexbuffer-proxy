// tests/tls_passthrough_integration.rs — Verifies raw TCP tunnel passthrough when TLS interception is bypassed

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use http::{Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use hexbuffer_proxy::{Body, HttpContext, HttpHandler, ProxyBuilder, RequestOrResponse};

struct PinnedBypassHandler {
    bypass_host: String,
    request_intercepted: Arc<AtomicBool>,
}

#[async_trait]
impl HttpHandler for PinnedBypassHandler {
    async fn should_intercept_tls(&self, host: &str) -> bool {
        // Bypass TLS interception for the configured host
        host != self.bypass_host
    }

    async fn handle_request(
        &self,
        _ctx: &mut HttpContext,
        _req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        self.request_intercepted.store(true, Ordering::SeqCst);
        panic!("handle_request should NOT be called for bypassed TLS passthrough");
    }

    async fn handle_response(
        &self,
        _ctx: &mut HttpContext,
        res: Response<Body>,
    ) -> hexbuffer_proxy::Result<Response<Body>> {
        Ok(res)
    }
}

#[tokio::test]
async fn test_tls_passthrough_raw_tcp_tunnel() {
    // 1. Spawn a raw TCP server (simulating a pinned TLS server or non-HTTP service)
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    let server_received_payload = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let payload_clone = Arc::clone(&server_received_payload);

    let upstream_handle = tokio::spawn(async move {
        let (mut socket, _) = upstream_listener.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let n = socket.read(&mut buf).await.unwrap();
        buf.truncate(n);
        *payload_clone.lock().await = buf;

        // Echo response back
        socket.write_all(b"RAW_UPSTREAM_REPLY_456").await.unwrap();
    });

    // 2. Configure proxy to bypass interception for upstream host
    let request_intercepted = Arc::new(AtomicBool::new(false));
    let handler = PinnedBypassHandler {
        bypass_host: upstream_addr.ip().to_string(),
        request_intercepted: Arc::clone(&request_intercepted),
    };

    let ca = common::create_test_ca("passthrough");
    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca).with_http_handler(handler)).await;

    // 3. Connect to proxy and request CONNECT tunnel
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let connect_msg = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
        upstream_addr.ip(),
        upstream_addr.port(),
        upstream_addr.ip(),
        upstream_addr.port()
    );
    client.write_all(connect_msg.as_bytes()).await.unwrap();

    let mut response_line = [0u8; 64];
    let n = client.read(&mut response_line).await.unwrap();
    let res_text = String::from_utf8_lossy(&response_line[..n]);
    assert!(res_text.starts_with("HTTP/1.1 200 Connection Established"));

    // 4. Send raw data over the established tunnel
    let secret_raw_data = b"RAW_CLIENT_BYTES_123";
    client.write_all(secret_raw_data).await.unwrap();

    // 5. Read raw reply through the tunnel
    let mut reply_buf = vec![0u8; 64];
    let n = client.read(&mut reply_buf).await.unwrap();
    reply_buf.truncate(n);

    assert_eq!(&reply_buf, b"RAW_UPSTREAM_REPLY_456");
    assert_eq!(*server_received_payload.lock().await, secret_raw_data);
    assert!(!request_intercepted.load(Ordering::SeqCst));

    proxy_handle.abort();
    upstream_handle.abort();
}
