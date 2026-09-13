// tests/websocket_integration.rs — End-to-end WebSocket proxying and frame interception tests

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use hexbuffer_proxy::{Direction, HttpContext, ProxyBuilder, WebSocketHandler, WebSocketMessage};

struct TestWsHandler {
    client_to_server_frames: Arc<AtomicUsize>,
    server_to_client_frames: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl WebSocketHandler for TestWsHandler {
    async fn on_frame(
        &self,
        _ctx: &mut HttpContext,
        msg: WebSocketMessage,
        direction: Direction,
    ) -> Option<WebSocketMessage> {
        match direction {
            Direction::ClientToServer => {
                self.client_to_server_frames.fetch_add(1, Ordering::SeqCst);
                // Frame drop testing
                if let WebSocketMessage::Text(ref txt) = msg {
                    if txt.as_str() == "DROP_ME" {
                        return None;
                    }
                    if txt.as_str() == "MUTATE_ME" {
                        return Some(WebSocketMessage::Text("MUTATED_BY_PROXY".into()));
                    }
                }
                Some(msg)
            }
            Direction::ServerToClient => {
                self.server_to_client_frames.fetch_add(1, Ordering::SeqCst);
                Some(msg)
            }
        }
    }

    async fn on_close(&self, _ctx: &mut HttpContext) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn test_websocket_frame_inspection_mutation_and_dropping() {
    // 1. Spawn mock WebSocket upstream server (echo server)
    let (upstream_addr, upstream_handle) = common::spawn_mock_ws_server(|msg| async move {
        match msg {
            Message::Text(txt) => Some(Message::Text(format!("ECHO: {txt}").into())),
            Message::Binary(bin) => Some(Message::Binary(bin)),
            other => Some(other),
        }
    })
    .await;

    // 2. Configure proxy with WebSocket handler
    let c2s_count = Arc::new(AtomicUsize::new(0));
    let s2c_count = Arc::new(AtomicUsize::new(0));
    let closed_flag = Arc::new(AtomicBool::new(false));

    let ws_handler = TestWsHandler {
        client_to_server_frames: Arc::clone(&c2s_count),
        server_to_client_frames: Arc::clone(&s2c_count),
        closed: Arc::clone(&closed_flag),
    };

    let ca = common::create_test_ca("ws_test");
    let (proxy_addr, proxy_handle) =
        common::spawn_test_proxy(ProxyBuilder::new().with_ca(ca).with_ws_handler(ws_handler)).await;

    // 3. Connect client to proxy with HTTP WebSocket upgrade pointing to upstream Host
    let stream = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let ws_url = format!("ws://{}/ws", upstream_addr);
    let (mut client_ws, _) = tokio_tungstenite::client_async(ws_url, stream)
        .await
        .expect("WebSocket handshake through proxy failed");

    // ── Test 1: Normal frame passthrough ──
    client_ws.send(Message::Text("hello".into())).await.unwrap();
    let reply = client_ws.next().await.unwrap().unwrap();
    assert_eq!(reply, Message::Text("ECHO: hello".into()));

    // ── Test 2: Frame mutation ──
    client_ws
        .send(Message::Text("MUTATE_ME".into()))
        .await
        .unwrap();
    let reply_mutated = client_ws.next().await.unwrap().unwrap();
    assert_eq!(
        reply_mutated,
        Message::Text("ECHO: MUTATED_BY_PROXY".into())
    );

    // ── Test 3: Binary frame passthrough ──
    let test_bytes = vec![10u8, 20, 30, 40];
    client_ws
        .send(Message::Binary(test_bytes.clone().into()))
        .await
        .unwrap();
    let reply_bin = client_ws.next().await.unwrap().unwrap();
    assert_eq!(reply_bin, Message::Binary(test_bytes.into()));

    // ── Test 4: Frame drop ──
    client_ws
        .send(Message::Text("DROP_ME".into()))
        .await
        .unwrap();
    // Send a marker frame right after to verify DROP_ME was dropped and marker arrives next
    client_ws
        .send(Message::Text("marker".into()))
        .await
        .unwrap();
    let reply_marker = client_ws.next().await.unwrap().unwrap();
    assert_eq!(reply_marker, Message::Text("ECHO: marker".into()));

    // ── Test 5: Clean close ──
    client_ws.close(None).await.unwrap();

    // Verify frame counts
    // Client sent: hello (1), MUTATE_ME (2), binary (3), DROP_ME (4), marker (5) -> 5 frames inspected
    assert_eq!(c2s_count.load(Ordering::SeqCst), 5);
    // Server echoed: ECHO: hello (1), ECHO: MUTATED (2), binary (3), ECHO: marker (4) -> 4 frames
    assert_eq!(s2c_count.load(Ordering::SeqCst), 4);

    proxy_handle.abort();
    upstream_handle.abort();
}
