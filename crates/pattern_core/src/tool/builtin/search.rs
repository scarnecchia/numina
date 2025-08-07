//! Unified search tool for querying across different domains

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Result, context::AgentHandle, message::ChatRole, tool::AiTool};

/// Search domains available
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum SearchDomain {
    ArchivalMemory,
    Conversations,
    ConstellationMessages,
    All,
}

/// Input for unified search
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchInput {
    /// Where to search
    pub domain: SearchDomain,

    /// Search query
    pub query: String,

    /// Maximum number of results (default: 10)
    #[schemars(default, with = "i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,

    /// For conversations: filter by role (user/assistant/tool)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// For time-based filtering: start time
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,

    /// For time-based filtering: end time
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,

    /// Request another turn after this tool executes
    #[serde(default)]
    pub request_heartbeat: bool,
}

/// Output from search operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchOutput {
    /// Whether the search was successful
    pub success: bool,

    /// Message about the search
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Search results
    pub results: serde_json::Value,
}

/// Unified search tool
#[derive(Debug, Clone)]
pub struct SearchTool {
    pub(crate) handle: AgentHandle,
}

#[async_trait]
impl AiTool for SearchTool {
    type Input = SearchInput;
    type Output = SearchOutput;

    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Unified search across different domains (archival_memory, conversations, constellation_messages, all). Returns relevant results based on query and filters. Use constellation_messages to search messages from all agents in your constellation. archival_memory domain searches your recall memory."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        let limit = params
            .limit
            .map(|l| l.max(1).min(100) as usize)
            .unwrap_or(10);

        match params.domain {
            SearchDomain::ArchivalMemory => self.search_archival(&params.query, limit).await,
            SearchDomain::Conversations => {
                let role = params
                    .role
                    .as_ref()
                    .and_then(|r| match r.to_lowercase().as_str() {
                        "user" => Some(ChatRole::User),
                        "assistant" => Some(ChatRole::Assistant),
                        "tool" => Some(ChatRole::Tool),
                        _ => None,
                    });

                let start_time = params
                    .start_time
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                let end_time = params
                    .end_time
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                self.search_conversations(&params.query, role, start_time, end_time, limit)
                    .await
            }
            SearchDomain::ConstellationMessages => {
                let role = params
                    .role
                    .as_ref()
                    .and_then(|r| match r.to_lowercase().as_str() {
                        "user" => Some(ChatRole::User),
                        "assistant" => Some(ChatRole::Assistant),
                        "tool" => Some(ChatRole::Tool),
                        _ => None,
                    });

                let start_time = params
                    .start_time
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                let end_time = params
                    .end_time
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                self.search_constellation_messages(&params.query, role, start_time, end_time, limit)
                    .await
            }
            SearchDomain::All => self.search_all(&params.query, limit).await,
        }
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some("the conversation will be continued when called")
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Search archival memory for user preferences".to_string(),
                parameters: SearchInput {
                    domain: SearchDomain::ArchivalMemory,
                    query: "favorite color".to_string(),
                    limit: Some(5),
                    role: None,
                    start_time: None,
                    end_time: None,
                    request_heartbeat: false,
                },
                expected_output: Some(SearchOutput {
                    success: true,
                    message: Some("Found 1 archival memory matching 'favorite color'".to_string()),
                    results: json!([{
                        "label": "user_preferences",
                        "content": "User's favorite color is blue",
                        "created_at": "2024-01-01T00:00:00Z",
                        "updated_at": "2024-01-01T00:00:00Z"
                    }]),
                }),
            },
            crate::tool::ToolExample {
                description: "Search conversation history for technical discussions".to_string(),
                parameters: SearchInput {
                    domain: SearchDomain::Conversations,
                    query: "database design".to_string(),
                    limit: Some(10),
                    role: Some("assistant".to_string()),
                    start_time: None,
                    end_time: None,
                    request_heartbeat: false,
                },
                expected_output: Some(SearchOutput {
                    success: true,
                    message: Some("Found 3 messages matching 'database design'".to_string()),
                    results: json!([{
                        "id": "msg_123",
                        "role": "assistant",
                        "content": "For the database design, I recommend using...",
                        "created_at": "2024-01-01T00:00:00Z"
                    }]),
                }),
            },
        ]
    }
}

impl SearchTool {
    async fn search_archival(&self, query: &str, limit: usize) -> Result<SearchOutput> {
        // Try to use database if available
        if self.handle.has_db_connection() {
            match self.handle.search_archival_memories(query, limit).await {
                Ok(blocks) => {
                    let results: Vec<_> = blocks
                        .into_iter()
                        .map(|block| {
                            json!({
                                "label": block.label,
                                "content": block.value,
                                "created_at": block.created_at,
                                "updated_at": block.updated_at
                            })
                        })
                        .collect();

                    Ok(SearchOutput {
                        success: true,
                        message: Some(format!(
                            "Found {} archival memories matching '{}'",
                            results.len(),
                            query
                        )),
                        results: json!(results),
                    })
                }
                Err(e) => {
                    tracing::warn!("Database search failed, falling back to in-memory: {}", e);
                    self.search_archival_in_memory(query, limit)
                }
            }
        } else {
            self.search_archival_in_memory(query, limit)
        }
    }

    fn search_archival_in_memory(&self, query: &str, limit: usize) -> Result<SearchOutput> {
        let query_lower = query.to_lowercase();

        let mut results: Vec<_> = self
            .handle
            .memory
            .get_all_blocks()
            .into_iter()
            .filter(|block| {
                block.memory_type == crate::memory::MemoryType::Archival
                    && block.value.to_lowercase().contains(&query_lower)
            })
            .take(limit)
            .map(|block| {
                json!({
                    "label": block.label,
                    "content": block.value,
                    "created_at": block.created_at,
                    "updated_at": block.updated_at
                })
            })
            .collect();

        // Sort by most recently updated first
        results.sort_by(|a, b| {
            let a_time = a.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
            let b_time = b.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
            b_time.cmp(a_time)
        });

        Ok(SearchOutput {
            success: true,
            message: Some(format!(
                "Found {} archival memories matching '{}'",
                results.len(),
                query
            )),
            results: json!(results),
        })
    }

    async fn search_conversations(
        &self,
        query: &str,
        role: Option<ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<SearchOutput> {
        // Use database search if available
        if self.handle.has_db_connection() {
            match self
                .handle
                .search_conversations(Some(query), role, start_time, end_time, limit)
                .await
            {
                Ok(messages) => {
                    let results: Vec<_> = messages
                        .into_iter()
                        .map(|msg| {
                            json!({
                                "id": msg.id,
                                "role": msg.role.to_string(),
                                "content": msg.text_content().unwrap_or_default(),
                                "created_at": msg.created_at
                            })
                        })
                        .collect();

                    Ok(SearchOutput {
                        success: true,
                        message: Some(format!(
                            "Found {} messages matching '{}'",
                            results.len(),
                            query
                        )),
                        results: json!(results),
                    })
                }
                Err(e) => Ok(SearchOutput {
                    success: false,
                    message: Some(format!("Conversation search failed: {}", e)),
                    results: json!([]),
                }),
            }
        } else {
            Ok(SearchOutput {
                success: false,
                message: Some("Conversation search requires database connection".to_string()),
                results: json!([]),
            })
        }
    }

    async fn search_constellation_messages(
        &self,
        query: &str,
        role: Option<ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<SearchOutput> {
        // Use database search if available
        if self.handle.has_db_connection() {
            match self
                .handle
                .search_constellation_messages(Some(query), role, start_time, end_time, limit)
                .await
            {
                Ok(messages) => {
                    let results: Vec<_> = messages
                        .into_iter()
                        .map(|(agent_name, msg)| {
                            json!({
                                "agent": agent_name,
                                "id": msg.id,
                                "role": msg.role.to_string(),
                                "content": msg.text_content().unwrap_or_default(),
                                "created_at": msg.created_at
                            })
                        })
                        .collect();

                    Ok(SearchOutput {
                        success: true,
                        message: Some(format!(
                            "Found {} constellation messages matching '{}'",
                            results.len(),
                            query
                        )),
                        results: json!(results),
                    })
                }
                Err(e) => Ok(SearchOutput {
                    success: false,
                    message: Some(format!("Constellation message search failed: {}", e)),
                    results: json!([]),
                }),
            }
        } else {
            Ok(SearchOutput {
                success: false,
                message: Some(
                    "Constellation message search requires database connection".to_string(),
                ),
                results: json!([]),
            })
        }
    }

    async fn search_all(&self, query: &str, limit: usize) -> Result<SearchOutput> {
        // Search both domains and combine results
        let archival_result = self.search_archival(query, limit).await?;
        let conv_result = self
            .search_conversations(query, None, None, None, limit)
            .await?;

        let all_results = json!({
            "archival_memory": archival_result.results,
            "conversations": conv_result.results
        });

        Ok(SearchOutput {
            success: true,
            message: Some(format!("Searched all domains for '{}'", query)),
            results: all_results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        UserId,
        memory::{Memory, MemoryType},
    };

    #[tokio::test]
    async fn test_search_archival_in_memory() {
        let memory = Memory::with_owner(&UserId::generate());

        // Create some archival memories
        memory
            .create_block("pref_color", "User's favorite color is blue")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("pref_color") {
            block.memory_type = MemoryType::Archival;
        }

        memory
            .create_block("pref_food", "User likes Italian food")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("pref_food") {
            block.memory_type = MemoryType::Archival;
        }

        let handle = AgentHandle::test_with_memory(memory);
        let tool = SearchTool { handle };

        // Test searching
        let result = tool
            .execute(SearchInput {
                domain: SearchDomain::ArchivalMemory,
                query: "color".to_string(),
                limit: None,
                role: None,
                start_time: None,
                end_time: None,
                request_heartbeat: false,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.message.unwrap().contains("Found 1"));

        // Verify the results structure
        let results = result.results.as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["label"], "pref_color");
    }
}
