use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};

use hexbuffer_proxy::{CertificationAuthority, Proxy};

use crate::models::{DASHBOARD_PORT, PROXY_PORT, StatusResponse, TrafficRecord, UPSTREAM_PORT};
use crate::triggers::{
    trigger_https_mitm_test, trigger_mock_test, trigger_plain_http_test, trigger_ws_test,
};

pub const DASHBOARD_HTML: &str = include_str!("index.html");

pub struct AppState {
    pub history: Arc<Mutex<VecDeque<TrafficRecord>>>,
    pub tx: broadcast::Sender<TrafficRecord>,
    pub proxy: Proxy,
    pub ca: Arc<CertificationAuthority>,
}

impl AppState {
    pub async fn get_all(&self) -> Vec<TrafficRecord> {
        let history = self.history.lock().await;
        history.iter().cloned().collect()
    }

    pub async fn clear(&self) {
        let mut history = self.history.lock().await;
        history.clear();
    }
}

pub async fn start_dashboard_server(port: u16, state: Arc<AppState>) {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    println!("Dashboard UI & API listening on http://127.0.0.1:{port}");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(c) => c,
            Err(_) => break,
        };

        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let state = Arc::clone(&state);
                async move { handle_dashboard_request(req, state).await }
            });

            let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection(io, service)
                .await;
        });
    }
}

pub async fn handle_dashboard_request(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let path = req.uri().path();
    let method = req.method();

    match (method, path) {
        (&Method::GET, "/") => {
            let html = DASHBOARD_HTML;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/html; charset=utf-8")
                .body(Full::new(Bytes::from(html)).boxed())
                .unwrap())
        }

        (&Method::GET, "/api/history") => {
            let records = state.get_all().await;
            let json = serde_json::to_string(&records).unwrap();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(json)).boxed())
                .unwrap())
        }

        (&Method::GET, "/api/status") => {
            let records = state.get_all().await;
            let status = StatusResponse {
                enabled: state.proxy.is_enabled(),
                proxy_port: PROXY_PORT,
                dashboard_port: DASHBOARD_PORT,
                upstream_port: UPSTREAM_PORT,
                total_records: records.len(),
            };
            let json = serde_json::to_string(&status).unwrap();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(json)).boxed())
                .unwrap())
        }

        (&Method::POST, "/api/clear") => {
            state.clear().await;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("{\"ok\":true}")).boxed())
                .unwrap())
        }

        (&Method::POST, "/api/toggle") => {
            let current = state.proxy.is_enabled();
            state.proxy.set_enabled(!current);
            let json = format!(r#"{{"enabled": {}}}"#, !current);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(json)).boxed())
                .unwrap())
        }

        (&Method::GET, "/events") => {
            let rx = state.tx.subscribe();
            let stream = futures_util::stream::unfold((rx, true), |(mut rx, first)| async move {
                if first {
                    let frame = Frame::data(Bytes::from(": connected\n\n"));
                    return Some((Ok::<_, Infallible>(frame), (rx, false)));
                }
                match rx.recv().await {
                    Ok(record) => {
                        let json = serde_json::to_string(&record).unwrap_or_default();
                        let frame = Frame::data(Bytes::from(format!("data: {json}\n\n")));
                        Some((Ok(frame), (rx, false)))
                    }
                    Err(_) => None,
                }
            });
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/event-stream")
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .body(StreamBody::new(stream).boxed())
                .unwrap())
        }

        // ── Test Triggers ───────────────────────────────────────────
        (&Method::POST, "/api/trigger/http") => {
            trigger_plain_http_test().await;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("{\"triggered\":\"http\"}")).boxed())
                .unwrap())
        }

        (&Method::POST, "/api/trigger/mock") => {
            trigger_mock_test().await;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("{\"triggered\":\"mock\"}")).boxed())
                .unwrap())
        }

        (&Method::POST, "/api/trigger/https") => {
            let ca = Arc::clone(&state.ca);
            trigger_https_mitm_test(ca).await;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("{\"triggered\":\"https\"}")).boxed())
                .unwrap())
        }

        (&Method::POST, "/api/trigger/ws") => {
            trigger_ws_test().await;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("{\"triggered\":\"ws\"}")).boxed())
                .unwrap())
        }

        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("404 Not Found")).boxed())
            .unwrap()),
    }
}
