// tests/proxy_lifecycle_integration.rs — Verifies dynamic runtime enable/disable toggling

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

use hexbuffer_proxy::{Body, HttpContext, HttpHandler, ProxyBuilder, RequestOrResponse};

struct InterceptCounterHandler {
    interceptions: Arc<AtomicUsize>,
}

#[async_trait]
impl HttpHandler for InterceptCounterHandler {
    async fn handle_request(
        &self,
        _ctx: &mut HttpContext,
        _req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        self.interceptions.fetch_add(1, Ordering::SeqCst);
        let res = Response::builder()
            .status(StatusCode::OK)
            .body(Body::Full(Bytes::from("mitm intercepted")))
            .unwrap();
        Ok(RequestOrResponse::Response(res))
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
async fn test_proxy_initial_disabled_state() {
    // 1. Raw TCP server simulating destination
    let raw_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let raw_addr = raw_listener.local_addr().unwrap();

    let raw_handle = tokio::spawn(async move {
        let (mut socket, _) = raw_listener.accept().await.unwrap();
        let mut buf = [0u8; 32];
        let n = socket.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"PING");
        socket.write_all(b"PONG").await.unwrap();
    });

    let ca = common::create_test_ca("lifecycle_disabled");
    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca).with_enabled(false)).await;

    // 2. Connect to proxy and issue CONNECT
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let connect_req = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
        raw_addr.ip(),
        raw_addr.port(),
        raw_addr.ip(),
        raw_addr.port()
    );
    client.write_all(connect_req.as_bytes()).await.unwrap();

    let mut buf = [0u8; 64];
    let n = client.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200 Connection Established"));

    // 3. Since proxy is disabled, it should pass raw TCP directly to raw_listener
    client.write_all(b"PING").await.unwrap();
    let mut pong = [0u8; 16];
    let n = client.read(&mut pong).await.unwrap();
    assert_eq!(&pong[..n], b"PONG");

    proxy_handle.abort();
    raw_handle.abort();
}

#[tokio::test]
async fn test_proxy_runtime_toggle() {
    common::init_crypto();

    // 1. Raw TCP server for passthrough when proxy is disabled
    let raw_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let raw_addr = raw_listener.local_addr().unwrap();

    let raw_handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = raw_listener.accept().await {
            let mut buf = [0u8; 32];
            if let Ok(n) = socket.read(&mut buf).await
                && &buf[..n] == b"RAW_REQ"
            {
                let _ = socket.write_all(b"RAW_RESP").await;
            }
        }
    });

    let interceptions = Arc::new(AtomicUsize::new(0));
    let handler = InterceptCounterHandler {
        interceptions: Arc::clone(&interceptions),
    };

    let ca = common::create_test_ca("lifecycle_toggle");
    let client_config = common::client_config_trusting_ca(&ca);

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();

    let proxy = ProxyBuilder::new()
        .with_ca(ca)
        .with_enabled(true)
        .with_http_handler(handler)
        .build()
        .unwrap();

    let proxy_control = proxy.clone();
    let proxy_task = tokio::spawn(async move {
        let _ = proxy.start_with_listener(proxy_listener).await;
    });

    // ── Phase 1: Enabled — MITM interception active ──────────────
    assert!(proxy_control.is_enabled());

    let mut client1 = TcpStream::connect(proxy_addr).await.unwrap();
    client1
        .write_all(b"CONNECT toggle.internal:443 HTTP/1.1\r\nHost: toggle.internal:443\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let n = client1.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200 Connection Established"));

    let connector = TlsConnector::from(Arc::new(client_config.clone()));
    let server_name = ServerName::try_from("toggle.internal".to_string()).unwrap();
    let mut tls1 = connector.connect(server_name, client1).await.unwrap();

    tls1.write_all(b"GET /test HTTP/1.1\r\nHost: toggle.internal\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut res_buf = Vec::new();
    tls1.read_to_end(&mut res_buf).await.unwrap();
    assert!(String::from_utf8_lossy(&res_buf).contains("mitm intercepted"));
    assert_eq!(interceptions.load(Ordering::SeqCst), 1);

    // ── Phase 2: Disable proxy at runtime ────────────────────────
    proxy_control.disable();
    assert!(!proxy_control.is_enabled());

    let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
    let connect_req = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
        raw_addr.ip(),
        raw_addr.port(),
        raw_addr.ip(),
        raw_addr.port()
    );
    client2.write_all(connect_req.as_bytes()).await.unwrap();
    let mut buf2 = [0u8; 64];
    let n2 = client2.read(&mut buf2).await.unwrap();
    assert!(
        String::from_utf8_lossy(&buf2[..n2]).starts_with("HTTP/1.1 200 Connection Established")
    );

    // Since disabled, raw tunnel was established
    client2.write_all(b"RAW_REQ").await.unwrap();
    let mut reply = [0u8; 16];
    let nr = client2.read(&mut reply).await.unwrap();
    assert_eq!(&reply[..nr], b"RAW_RESP");
    assert_eq!(interceptions.load(Ordering::SeqCst), 1); // Interceptions count remains 1

    // ── Phase 3: Re-enable proxy at runtime ──────────────────────
    proxy_control.enable();
    assert!(proxy_control.is_enabled());

    let mut client3 = TcpStream::connect(proxy_addr).await.unwrap();
    client3
        .write_all(b"CONNECT toggle.internal:443 HTTP/1.1\r\nHost: toggle.internal:443\r\n\r\n")
        .await
        .unwrap();
    let mut buf3 = [0u8; 64];
    let n3 = client3.read(&mut buf3).await.unwrap();
    assert!(
        String::from_utf8_lossy(&buf3[..n3]).starts_with("HTTP/1.1 200 Connection Established")
    );

    let server_name3 = ServerName::try_from("toggle.internal".to_string()).unwrap();
    let mut tls3 = connector.connect(server_name3, client3).await.unwrap();
    tls3.write_all(b"GET /test HTTP/1.1\r\nHost: toggle.internal\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut res_buf3 = Vec::new();
    tls3.read_to_end(&mut res_buf3).await.unwrap();
    assert!(String::from_utf8_lossy(&res_buf3).contains("mitm intercepted"));
    assert_eq!(interceptions.load(Ordering::SeqCst), 2);

    proxy_task.abort();
    raw_handle.abort();
}
