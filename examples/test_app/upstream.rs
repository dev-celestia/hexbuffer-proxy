use std::convert::Infallible;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;

use crate::handlers::time_now_str;

pub async fn start_mock_upstream(port: u16) {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    println!("Mock Upstream server listening on 127.0.0.1:{port}");

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(c) => c,
                Err(_) => break,
            };

            tokio::spawn(async move {
                let mut peek_buf = [0u8; 512];
                let n = stream.peek(&mut peek_buf).await.unwrap_or(0);
                let peek_str = String::from_utf8_lossy(&peek_buf[..n]);

                if peek_str.contains("Upgrade: websocket")
                    || peek_str.contains("upgrade: websocket")
                {
                    if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                        let (mut write, mut read) = ws.split();
                        while let Some(Ok(msg)) = read.next().await {
                            if msg.is_close() {
                                let _ = write.send(msg).await;
                                break;
                            }
                            if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
                                let reply = format!("Upstream Echo: {t}");
                                if write
                                    .send(tokio_tungstenite::tungstenite::Message::Text(
                                        reply.into(),
                                    ))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    let io = TokioIo::new(stream);
                    let service = service_fn(|req: Request<Incoming>| async move {
                        let path = req.uri().path();
                        match path {
                            "/api/echo" => {
                                let body = req.into_body().collect().await.unwrap().to_bytes();
                                let req_text = String::from_utf8_lossy(&body);
                                let json_resp = format!(
                                    r#"{{"message": "Hello from upstream mock server!", "received_body": {:?}, "timestamp": "{}"}}"#,
                                    req_text,
                                    time_now_str()
                                );
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(StatusCode::OK)
                                        .header("Content-Type", "application/json")
                                        .body(Full::new(Bytes::from(json_resp)))
                                        .unwrap(),
                                )
                            }
                            "/api/user" => {
                                let json = r#"{"id": 42, "username": "alex", "role": "admin", "verified": true}"#;
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(StatusCode::OK)
                                        .header("Content-Type", "application/json")
                                        .body(Full::new(Bytes::from(json)))
                                        .unwrap(),
                                )
                            }
                            _ => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .body(Full::new(Bytes::from(
                                        "Upstream mock server generic 200 OK",
                                    )))
                                    .unwrap(),
                            ),
                        }
                    });

                    let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection(io, service)
                        .await;
                }
            });
        }
    });
}
