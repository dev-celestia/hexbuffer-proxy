// tests/common/mod.rs — Shared fixtures and helpers for hexbuffer-proxy tests
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Once;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::rustls::crypto::aws_lc_rs::default_provider;

use hexbuffer_proxy::{CertificationAuthority, ProxyBuilder};

static CRYPTO_INIT: Once = Once::new();

/// Ensure rustls crypto provider is installed once.
pub fn init_crypto() {
    CRYPTO_INIT.call_once(|| {
        let _ = default_provider().install_default();
    });
}

/// Create a test CA rooted in a unique temporary directory to avoid conflicts.
pub fn create_test_ca(test_name: &str) -> CertificationAuthority {
    let dir = std::env::temp_dir().join(format!(
        "hexbuffer_test_ca_{}_{}",
        test_name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    CertificationAuthority::new_in(&dir)
}

/// Convert a PEM string into a CertificateDer.
pub fn ca_pem_to_cert_der(pem: &str) -> rustls_pki_types::CertificateDer<'static> {
    use base64::prelude::*;
    let b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = BASE64_STANDARD.decode(b64).expect("valid base64 in PEM");
    rustls_pki_types::CertificateDer::from(der)
}

/// Build a rustls ClientConfig that trusts the given CA.
pub fn client_config_trusting_ca(
    ca: &CertificationAuthority,
) -> tokio_rustls::rustls::ClientConfig {
    init_crypto();
    let mut root_store = tokio_rustls::rustls::RootCertStore::empty();
    let cert = ca_pem_to_cert_der(ca.ca_cert_pem());
    root_store
        .add(cert)
        .expect("valid CA cert added to root store");

    tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

/// Spawn a live test proxy on an ephemeral port.
/// Returns the bound `SocketAddr` and background `JoinHandle`.
pub async fn spawn_test_proxy(builder: ProxyBuilder) -> (SocketAddr, JoinHandle<()>) {
    init_crypto();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let proxy = builder.build().unwrap();
    let handle = tokio::spawn(async move {
        let _ = proxy.start_with_listener(listener).await;
    });

    (addr, handle)
}

/// Spawn a mock HTTP/1.1 upstream server on an ephemeral port.
pub async fn spawn_mock_http_server<F, Fut>(handler: F) -> (SocketAddr, JoinHandle<()>)
where
    F: Fn(Request<Incoming>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Response<Full<Bytes>>> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };

            let io = TokioIo::new(stream);
            let h = handler.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let h = h.clone();
                    async move { Ok::<_, std::convert::Infallible>(h(req).await) }
                });

                let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection(io, service)
                    .await;
            });
        }
    });

    (addr, handle)
}

/// Spawn a mock WebSocket server that echoes text/binary frames or runs custom logic.
pub async fn spawn_mock_ws_server<F, Fut>(on_message: F) -> (SocketAddr, JoinHandle<()>)
where
    F: Fn(tokio_tungstenite::tungstenite::Message) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Option<tokio_tungstenite::tungstenite::Message>>
        + Send
        + 'static,
{
    use futures_util::{SinkExt, StreamExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };

            let cb = on_message.clone();
            tokio::spawn(async move {
                let ws_stream = match tokio_tungstenite::accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(_) => return,
                };

                let (mut write, mut read) = ws_stream.split();
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(msg) if msg.is_close() => {
                            let _ = write.send(msg).await;
                            break;
                        }
                        Ok(msg) => {
                            if let Some(resp) = cb(msg).await
                                && write.send(resp).await.is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    (addr, handle)
}
