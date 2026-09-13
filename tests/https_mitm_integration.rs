// tests/https_mitm_integration.rs — End-to-end HTTPS MITM interception tests

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use hexbuffer_proxy::{Body, HttpContext, HttpHandler, ProxyBuilder, RequestOrResponse};

struct HttpsMockHandler {
    request_count: Arc<AtomicUsize>,
}

#[async_trait]
impl HttpHandler for HttpsMockHandler {
    async fn handle_request(
        &self,
        ctx: &mut HttpContext,
        req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        self.request_count.fetch_add(1, Ordering::SeqCst);
        assert!(ctx.is_https, "ctx.is_https must be true for HTTPS MITM");
        assert_eq!(ctx.host, "secure.api.internal");

        let path = req.uri().path();
        if path == "/v1/auth" {
            let res = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Full(Bytes::from("{\"token\":\"secret-token-123\"}")))
                .unwrap();
            Ok(RequestOrResponse::Response(res))
        } else if path == "/v1/upload" {
            let body_bytes = req.into_body().into_bytes().await.unwrap();
            let res = Response::builder()
                .status(StatusCode::CREATED)
                .header("X-Bytes-Received", body_bytes.len().to_string())
                .body(Body::Full(Bytes::from("uploaded successfully")))
                .unwrap();
            Ok(RequestOrResponse::Response(res))
        } else {
            let res = Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::Full(Bytes::from("not found")))
                .unwrap();
            Ok(RequestOrResponse::Response(res))
        }
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
async fn test_https_mitm_connect_handshake_and_interception() {
    let req_counter = Arc::new(AtomicUsize::new(0));
    let handler = HttpsMockHandler {
        request_count: Arc::clone(&req_counter),
    };

    let ca = common::create_test_ca("https_mitm");
    let client_config = common::client_config_trusting_ca(&ca);

    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca).with_http_handler(handler)).await;

    // 1. Connect TCP to proxy
    let mut tcp_stream = TcpStream::connect(proxy_addr).await.unwrap();

    // 2. Issue CONNECT request
    let connect_req =
        "CONNECT secure.api.internal:443 HTTP/1.1\r\nHost: secure.api.internal:443\r\n\r\n";
    tcp_stream.write_all(connect_req.as_bytes()).await.unwrap();

    // 3. Read 200 Connection Established
    let mut connect_res = [0u8; 128];
    let n = tcp_stream.read(&mut connect_res).await.unwrap();
    let res_text = String::from_utf8_lossy(&connect_res[..n]);
    assert!(res_text.starts_with("HTTP/1.1 200 Connection Established"));

    // 4. Perform TLS handshake with the proxy using forged certificate
    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = ServerName::try_from("secure.api.internal".to_string()).unwrap();
    let mut tls_stream = connector.connect(server_name, tcp_stream).await.unwrap();

    // 5. Send encrypted HTTP request inside the TLS tunnel
    let http_req =
        "GET /v1/auth HTTP/1.1\r\nHost: secure.api.internal\r\nConnection: close\r\n\r\n";
    tls_stream.write_all(http_req.as_bytes()).await.unwrap();

    // 6. Read decrypted response
    let mut resp_buf = Vec::new();
    tls_stream.read_to_end(&mut resp_buf).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp_buf);

    assert!(resp_str.starts_with("HTTP/1.1 200 OK"));
    assert!(resp_str.contains("{\"token\":\"secret-token-123\"}"));
    assert_eq!(req_counter.load(Ordering::SeqCst), 1);

    proxy_handle.abort();
}

#[tokio::test]
async fn test_https_mitm_post_large_payload() {
    let req_counter = Arc::new(AtomicUsize::new(0));
    let handler = HttpsMockHandler {
        request_count: Arc::clone(&req_counter),
    };

    let ca = common::create_test_ca("https_mitm_post");
    let client_config = common::client_config_trusting_ca(&ca);

    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca).with_http_handler(handler)).await;

    let mut tcp_stream = TcpStream::connect(proxy_addr).await.unwrap();
    let connect_req =
        "CONNECT secure.api.internal:443 HTTP/1.1\r\nHost: secure.api.internal:443\r\n\r\n";
    tcp_stream.write_all(connect_req.as_bytes()).await.unwrap();

    let mut connect_res = [0u8; 128];
    let n = tcp_stream.read(&mut connect_res).await.unwrap();
    assert!(
        String::from_utf8_lossy(&connect_res[..n])
            .starts_with("HTTP/1.1 200 Connection Established")
    );

    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = ServerName::try_from("secure.api.internal".to_string()).unwrap();
    let mut tls_stream = connector.connect(server_name, tcp_stream).await.unwrap();

    // 64 KB payload
    let payload = vec![b'x'; 65536];
    let http_req = format!(
        "POST /v1/upload HTTP/1.1\r\nHost: secure.api.internal\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    tls_stream.write_all(http_req.as_bytes()).await.unwrap();
    tls_stream.write_all(&payload).await.unwrap();

    let mut resp_buf = Vec::new();
    tls_stream.read_to_end(&mut resp_buf).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp_buf);

    assert!(resp_str.starts_with("HTTP/1.1 201 Created"));
    assert!(
        resp_str.contains("X-Bytes-Received: 65536")
            || resp_str.contains("x-bytes-received: 65536")
    );
    assert!(resp_str.contains("uploaded successfully"));
    assert_eq!(req_counter.load(Ordering::SeqCst), 1);

    proxy_handle.abort();
}
