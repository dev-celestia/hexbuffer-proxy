// tests/http_integration.rs — End-to-end Plain HTTP integration tests

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use hexbuffer_proxy::{Body, HttpContext, HttpHandler, ProxyBuilder, RequestOrResponse};

#[tokio::test]
async fn test_plain_http_forward_passthrough() {
    let (upstream_addr, upstream_handle) = common::spawn_mock_http_server(|req| async move {
        assert_eq!(req.uri().path(), "/test-path");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain")
            .body(Full::new(Bytes::from("hello upstream")))
            .unwrap()
    })
    .await;

    let ca = common::create_test_ca("http_passthrough");
    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca)).await;

    // Send HTTP request to proxy targeting upstream server
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let raw_request = format!(
        "GET /test-path HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        upstream_addr
    );
    client.write_all(raw_request.as_bytes()).await.unwrap();

    let mut response_buf = Vec::new();
    client.read_to_end(&mut response_buf).await.unwrap();
    let response_str = String::from_utf8_lossy(&response_buf);

    assert!(response_str.starts_with("HTTP/1.1 200 OK"));
    assert!(response_str.contains("hello upstream"));

    proxy_handle.abort();
    upstream_handle.abort();
}

struct ModifyingHandler;

#[async_trait]
impl HttpHandler for ModifyingHandler {
    async fn handle_request(
        &self,
        _ctx: &mut HttpContext,
        mut req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        req.headers_mut()
            .insert("X-Proxy-Intercepted", "true".parse().unwrap());
        Ok(RequestOrResponse::Request(req))
    }

    async fn handle_response(
        &self,
        _ctx: &mut HttpContext,
        res: Response<Body>,
    ) -> hexbuffer_proxy::Result<Response<Body>> {
        let (mut parts, _) = res.into_parts();
        parts
            .headers
            .insert("X-Proxy-Response-Injected", "v1.0".parse().unwrap());
        Ok(Response::from_parts(
            parts,
            Body::Full(Bytes::from("mutated response body")),
        ))
    }
}

#[tokio::test]
async fn test_plain_http_request_and_response_modification() {
    let upstream_called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&upstream_called);

    let (upstream_addr, upstream_handle) = common::spawn_mock_http_server(move |req| {
        let called = Arc::clone(&called_clone);
        async move {
            called.store(true, Ordering::SeqCst);
            // Verify that proxy handler injected the request header
            assert_eq!(req.headers().get("X-Proxy-Intercepted").unwrap(), "true");
            Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("original response")))
                .unwrap()
        }
    })
    .await;

    let handler = ModifyingHandler;

    let ca = common::create_test_ca("http_mod");
    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca).with_http_handler(handler)).await;

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let raw_request = format!(
        "GET /modify HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        upstream_addr
    );
    client.write_all(raw_request.as_bytes()).await.unwrap();

    let mut response_buf = Vec::new();
    client.read_to_end(&mut response_buf).await.unwrap();
    let response_str = String::from_utf8_lossy(&response_buf);

    assert!(upstream_called.load(Ordering::SeqCst));
    assert!(
        response_str.contains("X-Proxy-Response-Injected: v1.0")
            || response_str.contains("x-proxy-response-injected: v1.0")
    );
    assert!(response_str.contains("mutated response body"));
    assert!(!response_str.contains("original response"));

    proxy_handle.abort();
    upstream_handle.abort();
}

struct ShortCircuitHandler;

#[async_trait]
impl HttpHandler for ShortCircuitHandler {
    async fn handle_request(
        &self,
        _ctx: &mut HttpContext,
        _req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        let blocked = Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(Body::Full(Bytes::from("{\"blocked\":true}")))
            .unwrap();
        Ok(RequestOrResponse::Response(blocked))
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
async fn test_plain_http_short_circuit() {
    let upstream_call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = Arc::clone(&upstream_call_count);

    let (upstream_addr, upstream_handle) = common::spawn_mock_http_server(move |_| {
        let count = Arc::clone(&count_clone);
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("upstream reached")))
                .unwrap()
        }
    })
    .await;

    let ca = common::create_test_ca("http_short_circuit");
    let (proxy_addr, proxy_handle) = common::spawn_test_proxy(
        ProxyBuilder::new()
            .with_ca(ca)
            .with_http_handler(ShortCircuitHandler),
    )
    .await;

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let raw_request = format!(
        "GET /blocked HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        upstream_addr
    );
    client.write_all(raw_request.as_bytes()).await.unwrap();

    let mut response_buf = Vec::new();
    client.read_to_end(&mut response_buf).await.unwrap();
    let response_str = String::from_utf8_lossy(&response_buf);

    assert_eq!(upstream_call_count.load(Ordering::SeqCst), 0);
    assert!(response_str.starts_with("HTTP/1.1 403 Forbidden"));
    assert!(response_str.contains("{\"blocked\":true}"));

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn test_plain_http_post_with_body() {
    let received_body = Arc::new(tokio::sync::Mutex::new(String::new()));
    let body_clone = Arc::clone(&received_body);

    let (upstream_addr, upstream_handle) = common::spawn_mock_http_server(move |mut req| {
        let body_ref = Arc::clone(&body_clone);
        async move {
            let collected = req.body_mut().collect().await.unwrap().to_bytes();
            let mut lock = body_ref.lock().await;
            *lock = String::from_utf8_lossy(&collected).to_string();

            Response::builder()
                .status(StatusCode::CREATED)
                .body(Full::new(Bytes::from("created ok")))
                .unwrap()
        }
    })
    .await;

    let ca = common::create_test_ca("http_post");
    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca)).await;

    let post_payload = "{\"name\":\"test_item\",\"val\":42}";
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let raw_request = format!(
        "POST /items HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        upstream_addr,
        post_payload.len(),
        post_payload
    );
    client.write_all(raw_request.as_bytes()).await.unwrap();

    let mut response_buf = Vec::new();
    client.read_to_end(&mut response_buf).await.unwrap();
    let response_str = String::from_utf8_lossy(&response_buf);

    assert!(response_str.starts_with("HTTP/1.1 201 Created"));
    let final_body = received_body.lock().await;
    assert_eq!(*final_body, post_payload);

    proxy_handle.abort();
    upstream_handle.abort();
}
