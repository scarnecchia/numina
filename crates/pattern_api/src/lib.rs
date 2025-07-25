//! Pattern API types and definitions
//!
//! This crate defines the request/response types for the Pattern API,
//! shared between server and client implementations.

pub mod error;
pub mod events;
pub mod requests;
pub mod responses;

pub use error::ApiError;

// Re-export common types from pattern-core
pub use pattern_core::agent::AgentState;
pub use pattern_core::id::{AgentId, GroupId, MessageId, UserId};
pub use pattern_core::message::{ChatRole, Message, MessageContent};

/// API version constant
pub const API_VERSION: &str = "v1";

/// Common metadata included in all responses
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResponseMetadata {
    /// API version
    pub version: String,
    /// Request ID for tracing
    pub request_id: uuid::Uuid,
    /// Timestamp of response
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Default for ResponseMetadata {
    fn default() -> Self {
        Self {
            version: API_VERSION.to_string(),
            request_id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
        }
    }
}

/// Standard API response wrapper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiResponse<T> {
    /// Response metadata
    pub meta: ResponseMetadata,
    /// Response data
    pub data: T,
}

impl<T> ApiResponse<T> {
    pub fn new(data: T) -> Self {
        Self {
            meta: ResponseMetadata::default(),
            data,
        }
    }

    pub fn with_request_id(mut self, request_id: uuid::Uuid) -> Self {
        self.meta.request_id = request_id;
        self
    }
}

/// Pagination parameters
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PaginationParams {
    /// Page number (1-indexed)
    #[serde(default = "default_page")]
    pub page: u32,
    /// Items per page
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_page() -> u32 {
    1
}
fn default_limit() -> u32 {
    20
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: default_page(),
            limit: default_limit(),
        }
    }
}

/// Paginated response wrapper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaginatedResponse<T> {
    /// Items in this page
    pub items: Vec<T>,
    /// Current page number
    pub page: u32,
    /// Items per page
    pub limit: u32,
    /// Total number of items
    pub total: u64,
    /// Total number of pages
    pub total_pages: u32,
}

impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, page: u32, limit: u32, total: u64) -> Self {
        let total_pages = ((total as f64) / (limit as f64)).ceil() as u32;
        Self {
            items,
            page,
            limit,
            total,
            total_pages,
        }
    }
}
