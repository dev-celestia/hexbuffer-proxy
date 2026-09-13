use std::sync::Arc;

use base64::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::RootCertStore;

use hexbuffer_proxy::CertificationAuthority;

use crate::models::{PROXY_PORT, UPSTREAM_PORT};

pub async fn trigger_plain_http_test() {
    tokio::spawn(async move {
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{PROXY_PORT}")).await {
            let payload = r#"{"client": "Hexbuffer Dashboard", "type": "plain_http_test"}"#;
            let req = format!(
                "POST /api/echo HTTP/1.1\r\nHost: 127.0.0.1:{UPSTREAM_PORT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(req.as_bytes()).await;
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
        }
    });
}

pub async fn trigger_mock_test() {
    tokio::spawn(async move {
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{PROXY_PORT}")).await {
            let req = format!(
                "GET /mock/synthetic-short-circuit HTTP/1.1\r\nHost: 127.0.0.1:{UPSTREAM_PORT}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(req.as_bytes()).await;
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
        }
    });
}

pub async fn trigger_https_mitm_test(ca: Arc<CertificationAuthority>) {
    tokio::spawn(async move {
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{PROXY_PORT}")).await {
            let connect_req = "CONNECT secure.internal.local:443 HTTP/1.1\r\nHost: secure.internal.local:443\r\n\r\n";
            let _ = stream.write_all(connect_req.as_bytes()).await;

            let mut connect_res = [0u8; 64];
            if stream.read(&mut connect_res).await.unwrap_or(0) > 0 {
                let b64: String = ca
                    .ca_cert_pem()
                    .lines()
                    .filter(|l| !l.starts_with("-----"))
                    .collect();
                if let Ok(der) = BASE64_STANDARD.decode(b64) {
                    let mut roots = RootCertStore::empty();
                    let _ = roots.add(rustls_pki_types::CertificateDer::from(der));
                    let client_config = tokio_rustls::rustls::ClientConfig::builder()
                        .with_root_certificates(roots)
                        .with_no_client_auth();

                    let connector = TlsConnector::from(Arc::new(client_config));
                    if let Ok(server_name) =
                        rustls_pki_types::ServerName::try_from("secure.internal.local".to_string())
                        && let Ok(mut tls) = connector.connect(server_name, stream).await
                    {
                        let http_req = "GET /mock/synthetic-short-circuit HTTP/1.1\r\nHost: secure.internal.local\r\nConnection: close\r\n\r\n";
                        let _ = tls.write_all(http_req.as_bytes()).await;
                        let mut buf = Vec::new();
                        let _ = tls.read_to_end(&mut buf).await;
                    }
                }
            }
        }
    });
}

pub async fn trigger_ws_test() {
    tokio::spawn(async move {
        if let Ok(stream) = TcpStream::connect(format!("127.0.0.1:{PROXY_PORT}")).await {
            let ws_url = format!("ws://127.0.0.1:{UPSTREAM_PORT}/ws");
            if let Ok((mut client_ws, _)) = tokio_tungstenite::client_async(ws_url, stream).await {
                let _ = client_ws
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        "Live WebSocket Test from Dashboard!".into(),
                    ))
                    .await;
                let _ = client_ws.next().await;
                let _ = client_ws.close(None).await;
            }
        }
    });
}
