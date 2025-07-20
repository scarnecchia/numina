//! Type-safe ID generation and management
//!
//! This module provides a generic, type-safe ID system with consistent prefixes
//! and UUID-based uniqueness guarantees.

use compact_str::CompactString;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{self, Display};
use std::marker::PhantomData;
use std::str::FromStr;
use surrealdb::RecordId;
use uuid::Uuid;

use crate::db::strip_brackets;

/// A type-safe ID with a consistent prefix and UUID
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id<T> {
    /// The unique identifier
    uuid: Uuid,
    /// Phantom data to make each ID type unique
    _phantom: PhantomData<T>,
}

impl<T: IdType> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", T::PREFIX, self.uuid)
    }
}

/// Trait for types that can be used as ID markers
pub trait IdType: Send + Sync + 'static {
    /// The prefix for this ID type (e.g., "agt" for agents, "usr" for users)
    const PREFIX: &'static str;
}

/// Errors that can occur when working with IDs
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum IdError {
    #[error("Invalid ID format: expected prefix '{expected}', got '{actual}'")]
    #[diagnostic(help("Ensure the ID starts with the correct prefix followed by an underscore"))]
    InvalidPrefix { expected: String, actual: String },

    #[error("Invalid UUID: {0}")]
    #[diagnostic(help("The UUID portion of the ID must be a valid UUID v4 format"))]
    InvalidUuid(#[from] uuid::Error),

    #[error("Invalid ID format: {0}")]
    #[diagnostic(help(
        "IDs must be in the format 'prefix_uuid' where prefix matches the expected type"
    ))]
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
        let parts: Vec<&str> = s.splitn(2, '_').collect();
        if parts.len() != 2 {
            return Err(IdError::InvalidFormat(
                "ID must be in format 'prefix_uuid'".to_string(),
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

    pub fn from_record(record: RecordId) -> Self {
        Self::from_uuid(
            Uuid::from_str(strip_brackets(&record.key().to_string()))
                .expect("should be a valid uuid"),
        )
    }

    /// Get the prefix for this ID type
    pub fn prefix(&self) -> &'static str {
        T::PREFIX
    }

    /// Convert to a compact string representation
    pub fn to_compact_string(&self) -> CompactString {
        compact_str::format_compact!("{}_{}", T::PREFIX, self.uuid)
    }

    pub fn to_record_id(&self) -> String {
        self.uuid().to_string()
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
        write!(f, "{}_{}", T::PREFIX, self.uuid)
    }
}

impl<T: IdType> FromStr for Id<T> {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
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

impl<T: IdType> From<Id<T>> for RecordId {
    fn from(id: Id<T>) -> Self {
        // Use just the UUID part as the key
        RecordId::from_table_key(T::PREFIX, id.uuid.to_string())
    }
}

impl<T: IdType> From<&Id<T>> for RecordId {
    fn from(id: &Id<T>) -> Self {
        // Use just the UUID part as the key
        RecordId::from_table_key(T::PREFIX, id.uuid.to_string())
    }
}

impl<T: IdType> Serialize for Id<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}_{}", T::PREFIX, self.uuid()))
    }
}

impl<'de, T: IdType> Deserialize<'de> for Id<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let visitor: Id<T> = Id::nil();
        deserializer.deserialize_str(visitor)
    }
}

impl<'de, T: IdType> Visitor<'de> for Id<T> {
    type Value = Id<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "A string with the format 'prefix_UUID'")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Check if the string contains a separator
        let parts: Vec<&str> = s.splitn(2, '_').collect();
        if parts.len() != 2 {
            return Err(de::Error::custom(
                "ID must be in format 'prefix_uuid'".to_string(),
            ));
        }

        let [prefix, uuid_str] = [parts[0], parts[1]];

        // Verify prefix matches
        if prefix != T::PREFIX {
            return Err(de::Error::custom(format!(
                "ID prefix must match type ({}), but was {}",
                T::PREFIX,
                prefix
            )));
        }

        // Parse the UUID
        let uuid = Uuid::parse_str(uuid_str).map_err(|e| {
            de::Error::custom(format!(
                "Second component of id must be a valid UUIDv4, but got error{}",
                e
            ))
        })?;

        Ok(Self {
            uuid,
            _phantom: PhantomData,
        })
    }
}

// Implement common ID types
//
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelationIdType;

impl IdType for RelationIdType {
    const PREFIX: &'static str = "rel";
}

pub type RelationId = Id<RelationIdType>;

/// Marker type for Agent IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct AgentIdType;

impl IdType for AgentIdType {
    const PREFIX: &'static str = "agent";
}

/// Type alias for Agent IDs
pub type AgentId = Id<AgentIdType>;

/// Marker type for User IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct UserIdType;

impl IdType for UserIdType {
    const PREFIX: &'static str = "user";
}

/// Type alias for User IDs
pub type UserId = Id<UserIdType>;

/// Marker type for Conversation IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ConversationIdType;

impl IdType for ConversationIdType {
    const PREFIX: &'static str = "convo";
}

/// Type alias for Conversation IDs
pub type ConversationId = Id<ConversationIdType>;

/// Marker type for Task IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct TaskIdType;

impl IdType for TaskIdType {
    const PREFIX: &'static str = "task";
}

/// Type alias for Task IDs
pub type TaskId = Id<TaskIdType>;

/// Marker type for Tool Call IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ToolCallIdType;

impl IdType for ToolCallIdType {
    const PREFIX: &'static str = "toolcall";
}

/// Type alias for Tool Call IDs
pub type ToolCallId = Id<ToolCallIdType>;

/// Type alias for Message IDs (these are just strings for compat with API)
///
/// Unlike other IDs in the system, MessageId doesn't follow the `prefix_uuid`
/// format because it needs to be compatible with Anthropic/OpenAI APIs which
/// expect arbitrary string UUIDs.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
#[repr(transparent)]
pub struct MessageId(pub String);

impl IdType for MessageId {
    const PREFIX: &'static str = "msg";
}

impl MessageId {
    pub fn generate() -> Self {
        let mut buf = Uuid::encode_buffer();
        let uuid = uuid::Uuid::new_v4().simple().encode_lower(&mut buf);
        MessageId(format!("msg_{uuid}"))
    }
}

impl From<MessageId> for RecordId {
    fn from(value: MessageId) -> Self {
        RecordId::from_table_key("msg", value.0)
    }
}

/// Marker type for Memory Block IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct MemoryIdType;

impl IdType for MemoryIdType {
    const PREFIX: &'static str = "mem";
}

/// Type alias for Memory Block IDs
pub type MemoryId = Id<MemoryIdType>;

/// Marker type for Event IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct EventIdType;

impl IdType for EventIdType {
    const PREFIX: &'static str = "event";
}

/// Type alias for Event IDs
pub type EventId = Id<EventIdType>;

/// Marker type for Session IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct SessionIdType;

impl IdType for SessionIdType {
    const PREFIX: &'static str = "session";
}

/// Type alias for Session IDs
pub type SessionId = Id<SessionIdType>;

/// Marker type for Session IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ModelIdType;

impl IdType for ModelIdType {
    const PREFIX: &'static str = "model";
}

/// Type alias for Session IDs
pub type ModelId = Id<RequestIdType>;

/// Marker type for Session IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct RequestIdType;

impl IdType for RequestIdType {
    const PREFIX: &'static str = "request";
}

/// Type alias for Session IDs
pub type RequestId = Id<RequestIdType>;

/// Marker type for Group IDs
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct GroupIdType;

impl IdType for GroupIdType {
    const PREFIX: &'static str = "group";
}

/// Type alias for Group IDs
pub type GroupId = Id<GroupIdType>;

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
        assert!(id2.to_string().starts_with("agent_"));
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
        assert!(AgentId::parse("agent_").is_err());
        assert!(AgentId::parse("agent_not-a-uuid").is_err());

        // Should succeed with valid format
        let uuid = uuid::Uuid::new_v4();
        assert!(AgentId::parse(&format!("agent_{}", uuid)).is_ok());
    }

    #[test]
    fn test_id_serialization() {
        let id = AgentId::generate();

        // JSON serialization
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);

        // Should serialize as "prefix_uuid"
        assert!(json.contains("agent_"));
    }

    #[test]
    fn test_different_id_types() {
        let agent_id = AgentId::generate();
        let user_id = UserId::generate();
        let task_id = TaskId::generate();

        assert!(agent_id.to_string().starts_with("agent_"));
        assert!(user_id.to_string().starts_with("user_"));
        assert!(task_id.to_string().starts_with("task_"));
    }

    #[test]
    fn test_nil_id() {
        let nil_id = AgentId::nil();
        assert!(nil_id.is_nil());
        assert_eq!(
            nil_id.to_string(),
            "agent_00000000-0000-0000-0000-000000000000"
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

    #[test]
    fn test_debug_output() {
        let agent_id = AgentId::generate();
        let user_id = UserId::generate();

        // Debug output should be clean, just "prefix_uuid"
        let agent_debug = format!("{:?}", agent_id);
        let user_debug = format!("{:?}", user_id);

        assert!(agent_debug.starts_with("agent_"));
        assert!(user_debug.starts_with("user_"));

        // Should not contain PhantomData or other noise
        assert!(!agent_debug.contains("PhantomData"));
        assert!(!agent_debug.contains("_phantom"));
        assert!(!agent_debug.contains("uuid:"));

        // Debug should match Display
        assert_eq!(agent_debug, agent_id.to_string());
        assert_eq!(user_debug, user_id.to_string());
    }
}
