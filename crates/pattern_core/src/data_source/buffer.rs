use std::collections::VecDeque;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::Surreal;

use super::traits::StreamEvent;

/// Buffer for stream history with optional persistence
#[derive(Debug)]
pub struct StreamBuffer<T, C> {
    items: VecDeque<StreamEvent<T, C>>,
    max_items: usize,
    max_age: Duration,
    db: Option<Surreal<surrealdb::engine::any::Any>>,
}

impl<T, C> StreamBuffer<T, C>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone,
    C: Serialize + for<'de> Deserialize<'de> + Clone,
{
    pub fn new(max_items: usize, max_age: Duration) -> Self {
        Self {
            items: VecDeque::with_capacity(max_items),
            max_items,
            max_age,
            db: None,
        }
    }

    pub fn with_persistence(mut self, db: Surreal<surrealdb::engine::any::Any>) -> Self {
        self.db = Some(db);
        self
    }

    /// Add an item to the buffer
    pub fn push(&mut self, event: StreamEvent<T, C>) {
        // Remove old items if at capacity
        while self.items.len() >= self.max_items {
            self.items.pop_front();
        }

        // Remove items older than max_age
        let cutoff = Utc::now() - chrono::Duration::from_std(self.max_age).unwrap();
        while let Some(front) = self.items.front() {
            if front.timestamp < cutoff {
                self.items.pop_front();
            } else {
                break;
            }
        }

        self.items.push_back(event);
    }

    /// Get items within a time range
    pub fn get_range(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Vec<&StreamEvent<T, C>> {
        self.items
            .iter()
            .filter(|event| {
                let after_start = start.map_or(true, |s| event.timestamp >= s);
                let before_end = end.map_or(true, |e| event.timestamp <= e);
                after_start && before_end
            })
            .collect()
    }

    /// Get items after a specific cursor
    pub fn get_after_cursor(&self, cursor: &C) -> Vec<&StreamEvent<T, C>>
    where
        C: PartialEq,
    {
        let mut found = false;
        self.items
            .iter()
            .filter(|event| {
                if found {
                    true
                } else if event.cursor == *cursor {
                    found = true;
                    false
                } else {
                    false
                }
            })
            .collect()
    }

    /// Get buffer statistics
    pub fn stats(&self) -> BufferStats {
        BufferStats {
            item_count: self.items.len(),
            oldest_item: self.items.front().map(|e| e.timestamp),
            newest_item: self.items.back().map(|e| e.timestamp),
            max_items: self.max_items,
            max_age: self.max_age,
        }
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferStats {
    pub item_count: usize,
    pub oldest_item: Option<DateTime<Utc>>,
    pub newest_item: Option<DateTime<Utc>>,
    pub max_items: usize,
    pub max_age: Duration,
}

/// Configuration for stream buffering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferConfig {
    pub max_items: usize,
    pub max_age: Duration,
    pub persist_to_db: bool,
    pub index_content: bool,
}
