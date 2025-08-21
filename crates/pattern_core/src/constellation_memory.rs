//! Constellation-specific memory management for shared context

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::memory::{MemoryBlock, MemoryPermission, MemoryType};
use crate::{AgentId, MemoryId, UserId};

/// Activity event in a constellation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstellationEvent {
    /// When this event occurred
    pub timestamp: DateTime<Utc>,
    /// Which agent generated this event
    pub agent_id: AgentId,
    /// Agent's name for readability
    pub agent_name: String,
    /// Type of event
    pub event_type: ConstellationEventType,
    /// Brief description of the event
    pub description: String,
    /// Optional additional context
    pub metadata: Option<serde_json::Value>,
}

/// Types of events tracked in constellation activity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstellationEventType {
    /// Agent processed a message
    MessageProcessed {
        /// Brief summary or excerpt
        summary: String,
    },
    /// Agent performed a significant action via tool
    ToolExecuted {
        tool_name: String,
        /// Brief description of what was done
        action: String,
    },
    /// Memory was created or significantly updated
    MemoryUpdated {
        memory_label: String,
        change_type: MemoryChangeType,
    },
    /// Context sync occurred
    ContextSync {
        /// Which agent was synced
        synced_agent_id: AgentId,
    },
    /// Custom event type for domain-specific tracking
    Custom { category: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryChangeType {
    Created,
    Updated,
    Archived,
    Deleted,
}

/// Manages constellation activity tracking
pub struct ConstellationActivityTracker {
    /// Stable memory ID for the activity block
    memory_id: MemoryId,
    /// Maximum number of events to keep in the activity log
    max_events: usize,
    /// Events in chronological order, wrapped for thread-safe access
    events: Arc<RwLock<Vec<ConstellationEvent>>>,
}

impl ConstellationActivityTracker {
    pub fn new(max_events: usize) -> Self {
        Self {
            memory_id: MemoryId::generate(),
            max_events,
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create with a specific memory ID (for persistence)
    pub fn with_memory_id(memory_id: MemoryId, max_events: usize) -> Self {
        Self {
            memory_id,
            max_events,
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get the stable memory ID for this tracker
    pub fn memory_id(&self) -> &MemoryId {
        &self.memory_id
    }

    /// Add a new event to the activity log
    pub async fn add_event(&self, event: ConstellationEvent) {
        tracing::debug!(
            "ConstellationActivityTracker::add_event called for agent: {}",
            event.agent_name
        );
        let mut events = self.events.write().await;
        events.push(event);
        tracing::debug!("Event added, total events now: {}", events.len());

        // Trim to max size, keeping most recent
        if events.len() > self.max_events {
            let trim_count = events.len() - self.max_events;
            events.drain(0..trim_count);
            tracing::info!(
                "Trimmed {} old events, keeping {}",
                trim_count,
                self.max_events
            );
        }
    }

    /// Format the activity log as a memory block value
    pub async fn format_as_memory_content(&self) -> String {
        let mut content = String::from("# Constellation Activity Log\n\n");

        let events = self.events.read().await;

        if events.is_empty() {
            content.push_str("No recent activity recorded.\n");
            return content;
        }

        // Clone events for sorting
        let mut events_by_time = events.clone();
        events_by_time.sort_by_key(|e| e.timestamp);

        for event in events_by_time.iter().rev().take(50) {
            content.push_str(&format!(
                "[{}] **{}**: {}\n",
                event.timestamp.format("%Y-%m-%d %H:%M:%S"),
                event.agent_name,
                event.description
            ));

            // Add event-specific details
            match &event.event_type {
                ConstellationEventType::MessageProcessed { summary } => {
                    if !summary.is_empty() {
                        content.push_str(&format!("  > {}\n", summary));
                    }
                }
                ConstellationEventType::ToolExecuted { tool_name, action } => {
                    content.push_str(&format!("  > Tool: {} - {}\n", tool_name, action));
                }
                ConstellationEventType::MemoryUpdated {
                    memory_label,
                    change_type,
                } => {
                    content.push_str(&format!(
                        "  > Memory '{}' {:?}\n",
                        memory_label, change_type
                    ));
                }
                ConstellationEventType::ContextSync { synced_agent_id } => {
                    content.push_str(&format!("  > Synced with agent: {}\n", synced_agent_id));
                }
                ConstellationEventType::Custom { category } => {
                    content.push_str(&format!("  > Category: {}\n", category));
                }
            }

            content.push('\n');
        }

        content
    }

    /// Get events since a specific timestamp
    pub async fn events_since(&self, since: DateTime<Utc>) -> Vec<ConstellationEvent> {
        let events = self.events.read().await;
        events
            .iter()
            .filter(|e| e.timestamp > since)
            .cloned()
            .collect()
    }

    /// Count events since a specific timestamp
    pub async fn event_count_since(&self, since: DateTime<Utc>) -> usize {
        let events = self.events.read().await;
        events.iter().filter(|e| e.timestamp > since).count()
    }

    /// Create or update the memory block for this tracker
    pub async fn to_memory_block(&self, owner_id: UserId) -> MemoryBlock {
        create_constellation_activity_block(
            self.memory_id.clone(),
            owner_id,
            self.format_as_memory_content().await,
        )
    }
}

/// Create a constellation activity memory block with a stable ID
pub fn create_constellation_activity_block(
    memory_id: MemoryId,
    owner_id: UserId,
    content: String,
) -> MemoryBlock {
    MemoryBlock::owned_with_id(memory_id, owner_id, "constellation_activity", content)
        .with_description(
            "Shared, automatically-updating activity log for all agents in the constellation",
        )
        .with_memory_type(MemoryType::Core)
        .with_permission(MemoryPermission::ReadOnly)
        .with_pinned(true) // Don't swap this out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_activity_tracker() {
        let tracker = ConstellationActivityTracker::new(100);

        let event = ConstellationEvent {
            timestamp: Utc::now(),
            agent_id: AgentId::generate(),
            agent_name: "Pattern".to_string(),
            event_type: ConstellationEventType::MessageProcessed {
                summary: "Discussed constellation context sharing".to_string(),
            },
            description: "Processed user message about context sharing".to_string(),
            metadata: None,
        };

        tracker.add_event(event).await;
        assert_eq!(tracker.events.read().await.len(), 1);

        let content = tracker.format_as_memory_content().await;
        assert!(content.contains("Pattern"));
        assert!(content.contains("context sharing"));
    }

    #[tokio::test]
    async fn test_event_trimming() {
        let tracker = ConstellationActivityTracker::new(5);

        // Add more events than the limit
        for i in 0..10 {
            let event = ConstellationEvent {
                timestamp: Utc::now(),
                agent_id: AgentId::generate(),
                agent_name: format!("Agent{}", i),
                event_type: ConstellationEventType::Custom {
                    category: "test".to_string(),
                },
                description: format!("Event {}", i),
                metadata: None,
            };
            tracker.add_event(event).await;
        }

        // Should only keep the last 5
        let events = tracker.events.read().await;
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].agent_name, "Agent5");
        assert_eq!(events[4].agent_name, "Agent9");
    }
}
