use serde::{Deserialize, Serialize};

pub const PROXY_PORT: u16 = 8080;
pub const DASHBOARD_PORT: u16 = 8081;
pub const UPSTREAM_PORT: u16 = 8082;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsFrameEntry {
    pub id: u64,
    pub time: String,
    pub direction: String,
    pub opcode: String,
    pub payload: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrafficRecord {
    pub id: u64,
    pub timestamp: String,
    pub protocol: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub duration_ms: u64,
    pub client_addr: String,
    pub req_headers: Vec<HeaderEntry>,
    pub res_headers: Vec<HeaderEntry>,
    pub req_body: String,
    pub res_body: String,
    pub short_circuited: bool,
    pub ws_frames: Vec<WsFrameEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub enabled: bool,
    pub proxy_port: u16,
    pub dashboard_port: u16,
    pub upstream_port: u16,
    pub total_records: usize,
}
