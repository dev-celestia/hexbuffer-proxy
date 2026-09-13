use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use tokio::sync::{Mutex, broadcast};

use hexbuffer_proxy::{
    Body, Direction, HttpContext, HttpHandler, RequestOrResponse,
    WebSocketHandler, WebSocketMessage,
};

use crate::models::{HeaderEntry, TrafficRecord, WsFrameEntry};

pub fn time_now_str() -> String {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() % 86400;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, mins, s)
}

pub async fn record_traffic(
    history: &Arc<Mutex<VecDeque<TrafficRecord>>>,
    tx: &broadcast::Sender<TrafficRecord>,
    record: TrafficRecord,
) {
    let mut lock = history.lock().await;
    if lock.len() >= 100 {
        lock.pop_front();
    }
    lock.push_back(record.clone());
    let _ = tx.send(record);
}

pub type RequestMetadataMap = HashMap<u64, (Instant, Vec<HeaderEntry>, String)>;

pub struct InspectorHttpHandler {
    history: Arc<Mutex<VecDeque<TrafficRecord>>>,
    tx: broadcast::Sender<TrafficRecord>,
    req_start: Mutex<RequestMetadataMap>,
}

impl InspectorHttpHandler {
    pub fn new(
        history: Arc<Mutex<VecDeque<TrafficRecord>>>,
        tx: broadcast::Sender<TrafficRecord>,
    ) -> Self {
        Self {
            history,
            tx,
            req_start: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl HttpHandler for InspectorHttpHandler {
    async fn handle_request(
        &self,
        ctx: &mut HttpContext,
        mut req: Request<Body>,
    ) -> hexbuffer_proxy::Result<RequestOrResponse> {
        // Ignore dashboard internal traffic to prevent self-interception feedback loops
        if ctx.host.contains(":8081") || req.uri().port_u16() == Some(8081) {
            return Ok(RequestOrResponse::Request(req));
        }

        let start = Instant::now();

        // Inject header to demonstrate proxy request modification
        req.headers_mut()
            .insert("X-Hexbuffer-Intercepted", "true".parse().unwrap());

        let req_headers: Vec<HeaderEntry> = req
            .headers()
            .iter()
            .map(|(k, v)| HeaderEntry {
                name: k.as_str().to_string(),
                value: v.to_str().unwrap_or("<binary>").to_string(),
            })
            .collect();

        let req_body_str = match req.body() {
            Body::Full(b) => String::from_utf8_lossy(b).to_string(),
            _ => "<streaming>".to_string(),
        };

        // Demo Synthetic Short-Circuit endpoint
        if req.uri().path() == "/mock/synthetic-short-circuit" {
            let mock_json = r#"{"mocked": true, "message": "Synthetic response generated directly by hexbuffer-proxy without contacting upstream!", "status": "intercepted_and_short_circuited"}"#;
            let res = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header("X-Hexbuffer-Mock", "active")
                .body(Body::Full(Bytes::from(mock_json)))
                .unwrap();

            let record = TrafficRecord {
                id: ctx.id,
                timestamp: time_now_str(),
                protocol: if ctx.is_https { "HTTPS" } else { "HTTP" }.to_string(),
                method: req.method().to_string(),
                url: req.uri().to_string(),
                status: 200,
                status_text: "OK (Synthetic Mock)".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                client_addr: ctx.client_addr.to_string(),
                req_headers,
                res_headers: vec![
                    HeaderEntry {
                        name: "Content-Type".into(),
                        value: "application/json".into(),
                    },
                    HeaderEntry {
                        name: "X-Hexbuffer-Mock".into(),
                        value: "active".into(),
                    },
                ],
                req_body: req_body_str,
                res_body: mock_json.to_string(),
                short_circuited: true,
                ws_frames: Vec::new(),
            };

            record_traffic(&self.history, &self.tx, record).await;
            return Ok(RequestOrResponse::Response(res));
        }

        self.req_start
            .lock()
            .await
            .insert(ctx.id, (start, req_headers, req_body_str));

        Ok(RequestOrResponse::Request(req))
    }

    async fn handle_response(
        &self,
        ctx: &mut HttpContext,
        mut res: Response<Body>,
    ) -> hexbuffer_proxy::Result<Response<Body>> {
        if ctx.host.contains(":8081") {
            return Ok(res);
        }

        let (start, req_headers, req_body) = self
            .req_start
            .lock()
            .await
            .remove(&ctx.id)
            .unwrap_or_else(|| (Instant::now(), Vec::new(), String::new()));

        let duration_ms = start.elapsed().as_millis() as u64;

        // Injected response header
        res.headers_mut()
            .insert("X-Hexbuffer-Response-Header", "processed".parse().unwrap());

        let res_headers: Vec<HeaderEntry> = res
            .headers()
            .iter()
            .map(|(k, v)| HeaderEntry {
                name: k.as_str().to_string(),
                value: v.to_str().unwrap_or("<binary>").to_string(),
            })
            .collect();

        let status = res.status();
        let (parts, body) = res.into_parts();
        let body_bytes = body.into_bytes().await?;
        let res_body_str = String::from_utf8_lossy(&body_bytes).to_string();

        let record = TrafficRecord {
            id: ctx.id,
            timestamp: time_now_str(),
            protocol: if ctx.is_https { "HTTPS" } else { "HTTP" }.to_string(),
            method: "FORWARD".to_string(),
            url: format!(
                "http{}://{}{}",
                if ctx.is_https { "s" } else { "" },
                ctx.host,
                parts
                    .headers
                    .get("x-original-uri")
                    .and_then(|u| u.to_str().ok())
                    .unwrap_or("/")
            ),
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("Unknown").to_string(),
            duration_ms,
            client_addr: ctx.client_addr.to_string(),
            req_headers,
            res_headers,
            req_body,
            res_body: res_body_str,
            short_circuited: false,
            ws_frames: Vec::new(),
        };

        record_traffic(&self.history, &self.tx, record).await;

        Ok(Response::from_parts(parts, Body::Full(body_bytes)))
    }
}

pub struct InspectorWsHandler {
    history: Arc<Mutex<VecDeque<TrafficRecord>>>,
    tx: broadcast::Sender<TrafficRecord>,
    counter: AtomicU64,
}

impl InspectorWsHandler {
    pub fn new(
        history: Arc<Mutex<VecDeque<TrafficRecord>>>,
        tx: broadcast::Sender<TrafficRecord>,
    ) -> Self {
        Self {
            history,
            tx,
            counter: AtomicU64::new(1),
        }
    }
}

#[async_trait]
impl WebSocketHandler for InspectorWsHandler {
    async fn on_frame(
        &self,
        ctx: &mut HttpContext,
        msg: WebSocketMessage,
        direction: Direction,
    ) -> Option<WebSocketMessage> {
        let frame_id = self.counter.fetch_add(1, Ordering::SeqCst);
        let dir_str = match direction {
            Direction::ClientToServer => "Client → Server",
            Direction::ServerToClient => "Server → Client",
        };

        let (opcode, payload) = match &msg {
            WebSocketMessage::Text(t) => ("Text", t.to_string()),
            WebSocketMessage::Binary(b) => (
                "Binary",
                format!("{} bytes: {:?}", b.len(), &b[..b.len().min(32)]),
            ),
            _ => ("Control", "<control frame>".to_string()),
        };

        let record = TrafficRecord {
            id: ctx.id + frame_id,
            timestamp: time_now_str(),
            protocol: "WS".to_string(),
            method: "FRAME".to_string(),
            url: format!("ws://{}/ws", ctx.host),
            status: 101,
            status_text: "Switching Protocols".to_string(),
            duration_ms: 1,
            client_addr: ctx.client_addr.to_string(),
            req_headers: vec![],
            res_headers: vec![],
            req_body: format!("[{dir_str}] {payload}"),
            res_body: String::new(),
            short_circuited: false,
            ws_frames: vec![WsFrameEntry {
                id: frame_id,
                time: time_now_str(),
                direction: dir_str.to_string(),
                opcode: opcode.to_string(),
                payload,
            }],
        };

        record_traffic(&self.history, &self.tx, record).await;
        Some(msg)
    }

    async fn on_close(&self, _ctx: &mut HttpContext) {}
}
