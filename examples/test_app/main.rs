use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio_rustls::rustls::crypto::aws_lc_rs::default_provider;

use hexbuffer_proxy::{CertificationAuthority, ProxyBuilder};

mod dashboard;
mod handlers;
mod models;
mod triggers;
mod upstream;

use dashboard::{AppState, start_dashboard_server};
use handlers::{InspectorHttpHandler, InspectorWsHandler};
use models::{DASHBOARD_PORT, PROXY_PORT, UPSTREAM_PORT};
use upstream::start_mock_upstream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("═══════════════════════════════════════════════════════════════");
    println!("  HEXBUFFER-PROXY — Interactive Web Test Application          ");
    println!("═══════════════════════════════════════════════════════════════");

    // 1. Rustls crypto provider
    let _ = default_provider().install_default();

    // 2. Start local upstream mock server (port 8082)
    start_mock_upstream(UPSTREAM_PORT).await;

    // 3. Create shared history and broadcast channel
    let (tx, _) = broadcast::channel(200);
    let history = Arc::new(Mutex::new(VecDeque::with_capacity(100)));

    // 4. Custom handlers
    let http_handler = InspectorHttpHandler::new(history.clone(), tx.clone());
    let ws_handler = InspectorWsHandler::new(history.clone(), tx.clone());

    // 5. Build proxy instance (port 8080)
    let proxy = ProxyBuilder::new()
        .with_addr(
            format!("127.0.0.1:{PROXY_PORT}")
                .parse::<SocketAddr>()
                .unwrap(),
        )
        .with_ca(CertificationAuthority::new())
        .with_http_handler(http_handler)
        .with_ws_handler(ws_handler)
        .build()?;

    let ca = proxy.ca();
    let state = Arc::new(AppState {
        history,
        tx,
        proxy: proxy.clone(),
        ca,
    });

    // 6. Start Dashboard Web server (port 8081)
    let state_for_dashboard = Arc::clone(&state);
    tokio::spawn(async move {
        start_dashboard_server(DASHBOARD_PORT, state_for_dashboard).await;
    });

    // 7. Print welcoming banner
    println!();
    println!("🟢 MITM Proxy listening on:   127.0.0.1:{}", PROXY_PORT);
    println!(
        "🌐 Test Dashboard UI at:      http://127.0.0.1:{}",
        DASHBOARD_PORT
    );
    println!("🚀 Mock Upstream on:          127.0.0.1:{}", UPSTREAM_PORT);
    println!();
    println!(
        "👉 Open your browser to http://127.0.0.1:{} to inspect and test live traffic!",
        DASHBOARD_PORT
    );
    println!("Press Ctrl+C to stop.");
    println!();

    // 8. Start proxy accept loop
    proxy.start().await?;

    Ok(())
}
