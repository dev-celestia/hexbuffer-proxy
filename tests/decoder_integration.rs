// tests/decoder_integration.rs — End-to-end testing for application-level decoder plugin

#![cfg(feature = "decoder")]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use hexbuffer_proxy::decoder::{DecodeHandler, decode_response, encode_body};
use hexbuffer_proxy::{Body, HttpContext, HttpHandler, ProxyBuilder, RequestOrResponse};

struct RequestInspectHandler {
    observed_decoded_text: Arc<tokio::sync::Mutex<String>>,
    content_encoding_stripped: Arc<AtomicBool>,
}

#[async_trait]
impl HttpHandler for RequestInspectHandler {
    async fn handle_request(
        &self,
        _ctx: &mut HttpContext,
        req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        let has_encoding = req.headers().contains_key("Content-Encoding");
        self.content_encoding_stripped
            .store(!has_encoding, Ordering::SeqCst);

        let body_bytes = req.into_body().into_bytes().await.unwrap();
        *self.observed_decoded_text.lock().await = String::from_utf8_lossy(&body_bytes).to_string();

        let ok_res = Response::builder()
            .status(StatusCode::OK)
            .body(Body::Full(Bytes::from("inspection passed")))
            .unwrap();
        Ok(RequestOrResponse::Response(ok_res))
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
async fn test_decoder_handler_decompresses_gzip_request() {
    let observed_text = Arc::new(tokio::sync::Mutex::new(String::new()));
    let encoding_stripped = Arc::new(AtomicBool::new(false));

    let inspect_handler = RequestInspectHandler {
        observed_decoded_text: Arc::clone(&observed_text),
        content_encoding_stripped: Arc::clone(&encoding_stripped),
    };

    let ca = common::create_test_ca("decoder_req");
    let (proxy_addr, proxy_handle) = common::spawn_test_proxy(
        ProxyBuilder::new()
            .with_ca(ca)
            // DecodeHandler runs first, then InspectHandler sees decoded body
            .with_http_handler(DecodeHandler)
            .add_http_handler(inspect_handler),
    )
    .await;

    // Compress payload using encode_body
    let plain_text = "{\"user\":\"alice\",\"action\":\"login\",\"timestamp\":1700000000}";
    let compressed_body = encode_body(Body::Full(Bytes::from(plain_text)), "gzip", None).unwrap();
    let compressed_bytes = compressed_body.into_bytes().await.unwrap();

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let raw_request_head = format!(
        "POST /login HTTP/1.1\r\nHost: example.internal\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        compressed_bytes.len()
    );
    client.write_all(raw_request_head.as_bytes()).await.unwrap();
    client.write_all(&compressed_bytes).await.unwrap();

    let mut resp_buf = Vec::new();
    client.read_to_end(&mut resp_buf).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp_buf);

    assert!(resp_str.starts_with("HTTP/1.1 200 OK"));
    assert!(
        encoding_stripped.load(Ordering::SeqCst),
        "Content-Encoding header must be stripped"
    );
    let received = observed_text.lock().await;
    assert_eq!(*received, plain_text);

    proxy_handle.abort();
}

#[tokio::test]
async fn test_all_codecs_roundtrip_encode_decode() {
    let codecs = ["gzip", "deflate", "br", "zstd"];
    let sample = "The quick brown fox jumps over the lazy dog 1234567890 times with high entropy and repetition!";

    for codec in codecs {
        let compressed = encode_body(Body::Full(Bytes::from(sample)), codec, None)
            .unwrap_or_else(|e| panic!("compression failed for {codec}: {e}"));

        let compressed_bytes = compressed.into_bytes().await.unwrap();
        assert!(!compressed_bytes.is_empty());

        let res = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Encoding", codec)
            .body(Body::Full(compressed_bytes))
            .unwrap();

        let decoded = decode_response(res)
            .await
            .unwrap_or_else(|e| panic!("decompression failed for {codec}: {e}"));
        assert!(!decoded.headers().contains_key("Content-Encoding"));

        let decompressed_bytes = decoded.into_body().into_bytes().await.unwrap();
        assert_eq!(
            String::from_utf8_lossy(&decompressed_bytes),
            sample,
            "roundtrip mismatch for codec: {codec}"
        );
    }
}
