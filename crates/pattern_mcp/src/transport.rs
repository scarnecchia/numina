use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::Result;

/// A transport mechanism for MCP communication
#[async_trait]
pub trait Transport: Send + Sync + Debug {
    /// Get the type of this transport
    fn transport_type(&self) -> TransportType;

    /// Start the transport
    async fn start(&self) -> Result<()>;

    /// Stop the transport
    async fn stop(&self) -> Result<()>;

    /// Send a message through the transport
    async fn send(&self, message: TransportMessage) -> Result<()>;

    /// Receive a message from the transport
    async fn receive(&self) -> Result<TransportMessage>;

    /// Check if the transport is connected
    fn is_connected(&self) -> bool;
}

/// Types of transport supported
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    /// Standard input/output
    Stdio,
    /// HTTP with request/response
    Http,
    /// Server-sent events
    Sse,
    /// WebSocket
    WebSocket,
}

impl TransportType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Sse => "sse",
            Self::WebSocket => "websocket",
        }
    }
}

/// A message sent through the transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportMessage {
    pub id: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<TransportError>,
}

/// An error in transport communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

/// Standard MCP error codes
impl TransportError {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// A transport that does nothing (for testing)
#[derive(Debug)]
pub struct NullTransport;

#[async_trait]
impl Transport for NullTransport {
    fn transport_type(&self) -> TransportType {
        TransportType::Stdio
    }

    async fn start(&self) -> Result<()> {
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    async fn send(&self, _message: TransportMessage) -> Result<()> {
        Ok(())
    }

    async fn receive(&self) -> Result<TransportMessage> {
        futures::future::pending().await
    }

    fn is_connected(&self) -> bool {
        true
    }
}
