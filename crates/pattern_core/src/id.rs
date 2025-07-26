//! Type-safe ID generation and management
//!
//! This module provides a generic, type-safe ID system with consistent prefixes
//! and UUID-based uniqueness guarantees.

use compact_str::CompactString;
use schemars::JsonSchema;
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

impl<T: IdType> JsonSchema for Id<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Owned(format!("{}Id", T::PREFIX))
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Generate same schema as String since we serialize to string
        String::json_schema(generator)
    }
}

/// Macro to define new ID types with minimal boilerplate
#[macro_export]
macro_rules! define_id_type {
    ($type_name:ident, $prefix:expr) => {
        /// Marker type for the ID
        #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
        pub struct $type_name;

        impl $crate::id::IdType for $type_name {
            const PREFIX: &'static str = $prefix;
        }
    };
}

// Implement common ID types
//
/// Type alias for Message IDs (these are just strings for compat with API)
///
/// Unlike other IDs in the system, MessageId doesn't follow the `prefix_uuid`
/// format because it needs to be compatible with Anthropic/OpenAI APIs which
/// expect arbitrary string UUIDs.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
#[repr(transparent)]
pub struct RelationId(pub String);

// RelationId is special and doesn't use the macro
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct RelationIdType;

impl IdType for RelationIdType {
    const PREFIX: &'static str = "rel";
}

impl RelationId {
    pub fn generate() -> Self {
        let mut buf = Uuid::encode_buffer();
        let uuid = uuid::Uuid::new_v4().simple().encode_lower(&mut buf);
        RelationId(format!("rel_{uuid}"))
    }
}

impl From<RelationId> for RecordId {
    fn from(value: RelationId) -> Self {
        RecordId::from_table_key("rel", value.0)
    }
}

// Define common ID types using the macro
define_id_type!(AgentIdType, "agent");
define_id_type!(UserIdType, "user");
define_id_type!(ConversationIdType, "convo");
define_id_type!(TaskIdType, "task");
define_id_type!(ToolCallIdType, "toolcall");

/// Type alias for Agent IDs
pub type AgentId = Id<AgentIdType>;

/// Type alias for User IDs
pub type UserId = Id<UserIdType>;

impl Default for UserId {
    fn default() -> Self {
        UserId::generate()
    }
}

/// Type alias for Conversation IDs
pub type ConversationId = Id<ConversationIdType>;

/// Type alias for Task IDs
pub type TaskId = Id<TaskIdType>;

/// Type alias for Tool Call IDs
pub type ToolCallId = Id<ToolCallIdType>;

// MessageIdType uses the macro
define_id_type!(MessageIdType, "msg");

/// Unlike other IDs in the system, MessageId doesn't follow the `prefix_uuid`
/// format because it needs to be compatible with Anthropic/OpenAI APIs which
/// expect arbitrary string UUIDs.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, JsonSchema)]
#[repr(transparent)]
pub struct MessageId(pub String);

// MessageId cannot implement Copy because String doesn't implement Copy
// This is intentional as MessageId needs to own its string data

impl MessageId {
    pub fn generate() -> Self {
        let uuid = uuid::Uuid::new_v4();
        MessageId(format!("msg_{}", uuid))
    }

    pub fn to_record_id(&self) -> String {
        // Return the full string as the record key
        // MessageId can be arbitrary strings for API compatibility
        self.0.clone()
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        MessageId(format!("msg_{}", uuid))
    }

    pub fn from_record(record_id: RecordId) -> Self {
        MessageId(record_id.key().to_string())
    }

    pub fn nil() -> Self {
        MessageId("msg_nil".to_string())
    }
}

impl From<MessageId> for RecordId {
    fn from(value: MessageId) -> Self {
        // Use the full string as the key - MessageId can be arbitrary
        RecordId::from_table_key("msg", value.0)
    }
}

impl From<&MessageId> for RecordId {
    fn from(value: &MessageId) -> Self {
        // Use the full string as the key - MessageId can be arbitrary
        RecordId::from_table_key("msg", &value.0)
    }
}

// More ID types using the macro
define_id_type!(MemoryIdType, "mem");
define_id_type!(EventIdType, "event");
define_id_type!(SessionIdType, "session");

/// Type alias for Memory Block IDs
pub type MemoryId = Id<MemoryIdType>;

/// Type alias for Event IDs
pub type EventId = Id<EventIdType>;

/// Type alias for Session IDs
pub type SessionId = Id<SessionIdType>;

// Define new ID types using the macro
define_id_type!(ModelIdType, "model");
define_id_type!(RequestIdType, "request");
define_id_type!(GroupIdType, "group");
define_id_type!(ConstellationIdType, "const");
define_id_type!(OAuthTokenIdType, "oauth");

/// Type alias for Model IDs
pub type ModelId = Id<ModelIdType>;

/// Type alias for Request IDs
pub type RequestId = Id<RequestIdType>;

/// Type alias for Group IDs
pub type GroupId = Id<GroupIdType>;

/// Type alias for Constellation IDs
pub type ConstellationId = Id<ConstellationIdType>;

/// Type alias for OAuth Token IDs
pub type OAuthTokenId = Id<OAuthTokenIdType>;

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
