//! Type-safe ID generation and management
//!
//! This module provides a generic, type-safe ID system with consistent prefixes
//! and UUID-based uniqueness guarantees.

use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{self, Display};
use std::marker::PhantomData;
use std::str::FromStr;
use uuid::Uuid;

/// A type-safe ID with a consistent prefix and UUID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id<T> {
    /// The unique identifier
    uuid: Uuid,
    /// Phantom data to make each ID type unique
    _phantom: PhantomData<T>,
}

/// Trait for types that can be used as ID markers
pub trait IdType: Send + Sync + 'static {
    /// The prefix for this ID type (e.g., "agt" for agents, "usr" for users)
    const PREFIX: &'static str;
}

/// Errors that can occur when working with IDs
#[derive(Debug, thiserror::Error)]
pub enum IdError {
    #[error("Invalid ID format: expected prefix '{expected}', got '{actual}'")]
    InvalidPrefix { expected: String, actual: String },

    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    #[error("Invalid ID format: {0}")]
    InvalidFormat(String),
}

impl<T: IdType> Id<T> {
    /// Create a new ID with a generated UUID
    pub fn generate() -> Self {
        Self {
            uuid: Uuid::new_v4(),
            _phantom: PhantomData,
        }
    }

    /// Create an ID from a specific UUID (useful for tests or migrations)
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self {
            uuid,
            _phantom: PhantomData,
        }
    }

    /// Parse an ID from a string
    pub fn parse(s: &str) -> Result<Self, IdError> {
        // Check if the string contains a separator
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(IdError::InvalidFormat(
                "ID must be in format 'prefix-uuid'".to_string(),
            ));
        }

        let [prefix, uuid_str] = [parts[0], parts[1]];

        // Verify prefix matches
        if prefix != T::PREFIX {
            return Err(IdError::InvalidPrefix {
                expected: T::PREFIX.to_string(),
                actual: prefix.to_string(),
            });
        }

        // Parse the UUID
        let uuid = Uuid::parse_str(uuid_str)?;

        Ok(Self {
            uuid,
            _phantom: PhantomData,
        })
    }

    /// Get the UUID part
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get the prefix for this ID type
    pub fn prefix(&self) -> &'static str {
        T::PREFIX
    }

    /// Convert to a string representation
    pub fn to_string(&self) -> String {
        format!("{}-{}", T::PREFIX, self.uuid)
    }

    /// Convert to a compact string representation
    pub fn to_compact_string(&self) -> CompactString {
        CompactString::from(format!("{}-{}", T::PREFIX, self.uuid))
    }

    /// Create a nil/empty ID (all zeros)
    pub fn nil() -> Self {
        Self {
            uuid: Uuid::nil(),
            _phantom: PhantomData,
        }
    }

    /// Check if this is a nil/empty ID
    pub fn is_nil(&self) -> bool {
        self.uuid.is_nil()
    }
}

impl<T: IdType> Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", T::PREFIX, self.uuid)
    }
}

impl<T: IdType> FromStr for Id<T> {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl<T: IdType> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de, T: IdType> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl<T: IdType> From<Id<T>> for String {
    fn from(id: Id<T>) -> Self {
        id.to_string()
    }
}

impl<T: IdType> AsRef<Uuid> for Id<T> {
    fn as_ref(&self) -> &Uuid {
        &self.uuid
    }
}

// Implement common ID types

/// Marker type for Agent IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct AgentIdType;

impl IdType for AgentIdType {
    const PREFIX: &'static str = "agent";
}

/// Type alias for Agent IDs
pub type AgentId = Id<AgentIdType>;

/// Marker type for User IDs
#[derive(Debug)]
pub struct UserIdType;

impl IdType for UserIdType {
    const PREFIX: &'static str = "user";
}

/// Type alias for User IDs
pub type UserId = Id<UserIdType>;

/// Marker type for Conversation IDs
#[derive(Debug)]
pub struct ConversationIdType;

impl IdType for ConversationIdType {
    const PREFIX: &'static str = "convo";
}

/// Type alias for Conversation IDs
pub type ConversationId = Id<ConversationIdType>;

/// Marker type for Task IDs
#[derive(Debug)]
pub struct TaskIdType;

impl IdType for TaskIdType {
    const PREFIX: &'static str = "task";
}

/// Type alias for Task IDs
pub type TaskId = Id<TaskIdType>;

/// Marker type for Tool Call IDs
#[derive(Debug)]
pub struct ToolCallIdType;

impl IdType for ToolCallIdType {
    const PREFIX: &'static str = "toolcall";
}

/// Type alias for Tool Call IDs
pub type ToolCallId = Id<ToolCallIdType>;

/// Marker type for Message IDs
#[derive(Debug)]
pub struct MessageIdType;

impl IdType for MessageIdType {
    const PREFIX: &'static str = "msg";
}

/// Type alias for Message IDs
pub type MessageId = Id<MessageIdType>;

/// Marker type for Memory Block IDs
#[derive(Debug)]
pub struct MemoryIdType;

impl IdType for MemoryIdType {
    const PREFIX: &'static str = "mem";
}

/// Type alias for Memory Block IDs
pub type MemoryId = Id<MemoryIdType>;

/// Marker type for Session IDs
#[derive(Debug)]
pub struct SessionIdType;

impl IdType for SessionIdType {
    const PREFIX: &'static str = "session";
}

/// Type alias for Session IDs
pub type SessionId = Id<SessionIdType>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_generation() {
        let id1 = AgentId::generate();
        let id2 = AgentId::generate();

        // IDs should be unique
        assert_ne!(id1, id2);

        // IDs should have correct prefix
        assert_eq!(id1.prefix(), "agent");
        assert!(id1.to_string().starts_with("agent-"));
    }

    #[test]
    fn test_id_parsing() {
        let id = AgentId::generate();
        let id_str = id.to_string();

        // Should be able to parse back
        let parsed = AgentId::parse(&id_str).unwrap();
        assert_eq!(id, parsed);

        // Should fail with wrong prefix
        assert!(UserId::parse(&id_str).is_err());

        // Should fail with invalid format
        assert!(AgentId::parse("invalid").is_err());
        assert!(AgentId::parse("agent_wrong_separator").is_err());
        assert!(AgentId::parse("agent-not-a-uuid").is_err());
    }

    #[test]
    fn test_id_serialization() {
        let id = AgentId::generate();

        // JSON serialization
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);

        // Should serialize as "prefix-uuid"
        assert!(json.contains("agent-"));
    }

    #[test]
    fn test_different_id_types() {
        let agent_id = AgentId::generate();
        let user_id = UserId::generate();
        let task_id = TaskId::generate();

        assert!(agent_id.to_string().starts_with("agent-"));
        assert!(user_id.to_string().starts_with("user-"));
        assert!(task_id.to_string().starts_with("task-"));
    }

    #[test]
    fn test_nil_id() {
        let nil_id = AgentId::nil();
        assert!(nil_id.is_nil());
        assert_eq!(
            nil_id.to_string(),
            "agent-00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn test_from_uuid() {
        let uuid = Uuid::new_v4();
        let id = AgentId::from_uuid(uuid);
        assert_eq!(id.uuid(), uuid);
    }

    #[test]
    fn test_compact_string() {
        let id = AgentId::generate();
        let compact = id.to_compact_string();
        let string = id.to_string();
        assert_eq!(compact.as_str(), string.as_str());
    }
}
