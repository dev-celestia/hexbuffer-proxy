// hexbuffer-proxy — HTTPS MITM proxy library
//
// Core modules
//! # hexbuffer-proxy
//!
//! A local MITM (man-in-the-middle) HTTP/HTTPS proxy
//! built on Tokio + Hyper + rustls.
//!
//! ## Architecture
//!
//! - [`ProxyBuilder`] assembles the proxy with custom handlers.
//! - [`HttpHandler`] is the core trait — implement it to inspect or
//!   mutate traffic.
//! - HTTPS is intercepted via on-the-fly TLS certificate forging
//!   (powered by [`CertificationAuthority`]).
//! - Decrypted inner traffic is served by Hyper's HTTP/1.1 server
//!   (keep-alive, body framing, pipelining).
//! - Plain HTTP requests are handled by the same handler pipeline.
//! - WebSocket connections are detected, relayed, and optionally
//!   intercepted frame-by-frame via [`WebSocketHandler`].

pub mod builder;
pub mod ca;
pub mod error;
pub mod handler;

// Optional application-level body decoder
#[cfg(feature = "decoder")]
pub mod decoder;

// Internal modules
mod http_proxy;
mod https_proxy;
mod proxy;
mod upstream;
mod ws_proxy;

// Re-export public API at crate root
pub use builder::{Proxy, ProxyBuilder};
pub use ca::CertificationAuthority;
pub use error::{ProxyError, Result};
pub use handler::{
    Body, Direction, HttpContext, HttpHandler, NoopHandler, NoopWebSocketHandler,
    RequestOrResponse, WebSocketHandler, WebSocketMessage, full_body,
};
