//! Unified search tool for querying across different domains

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::search_utils::{extract_snippet, process_search_results};
use crate::{
    Result,
    context::AgentHandle,
    message::ChatRole,
    tool::{AiTool, ExecutionMeta},
};

/// Search domains available
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
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

    /// Enable fuzzy search (Note: Currently a placeholder - fuzzy search not yet implemented)
    /// This will enable typo-tolerant search once SurrealDB fuzzy functions are integrated
    #[serde(default)]
    pub fuzzy: bool,
    // request_heartbeat handled via ExecutionMeta injection; field removed
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
        "Unified search across different domains (archival_memory, conversations, constellation_messages, all). Returns relevant results ranked by BM25 relevance score. Make regular use of this to ground yourself in past events.
        - Use constellation_messages to search messages from all agents in your constellation.
        - archival_memory domain searches your recall memory.
        - To broaden your search, use a larger limit
        - To narrow your search, you can:
            - use explicit start_time and end_time parameters with rfc3339 datetime parsing
            - filter based on role (user, assistant, tool)
            - use time expressions after your query string
                - e.g. 'search term > 5 days', 'search term < 3 hours',
                       'search term 5 days old', 'search term 1-2 weeks'
                - supported units: hour/hours, day/days, week/weeks, month/months
                - IMPORTANT: time expression must come after query string, distinguishable by regular expression
                - if the only thing in the query is a time expression, it becomes a simple time-based filter
                - if you need to search for something that might otherwise be parsed as a time expression, quote it with \"5 days old\"
                "
    }

    async fn execute(&self, params: Self::Input, _meta: &ExecutionMeta) -> Result<Self::Output> {
        let limit = params
            .limit
            .map(|l| l.max(1).min(100) as usize)
            .unwrap_or(20);

        match params.domain {
            SearchDomain::ArchivalMemory => {
                self.search_archival(&params.query, limit, params.fuzzy)
                    .await
            }
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

                self.search_conversations(
                    &params.query,
                    role,
                    start_time,
                    end_time,
                    limit,
                    params.fuzzy,
                )
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

                self.search_constellation_messages(
                    &params.query,
                    role,
                    start_time,
                    end_time,
                    limit,
                    params.fuzzy,
                )
                .await
            }
            SearchDomain::All => self.search_all(&params.query, limit, params.fuzzy).await,
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
                    fuzzy: false,
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
                    fuzzy: false,
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
    async fn search_archival(
        &self,
        query: &str,
        limit: usize,
        fuzzy: bool,
    ) -> Result<SearchOutput> {
        // Try to use database if available
        if self.handle.has_db_connection() {
            // Note: fuzzy parameter is a placeholder for future fuzzy search implementation
            // Currently just passes through to methods that will use it when available
            let fuzzy_level = if fuzzy { Some(1) } else { None };
            match self
                .handle
                .search_archival_memories_with_options(query, limit, fuzzy_level)
                .await
            {
                Ok(mut scored_blocks) => {
                    // Re-sort and limit after we have all scores
                    scored_blocks.sort_by(|a, b| {
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    scored_blocks.truncate(limit);

                    let results: Vec<_> = scored_blocks
                        .into_iter()
                        .enumerate()
                        .map(|(i, sb)| {
                            // Progressive truncation: show less content for lower-ranked results
                            let content = if i < 5 {
                                sb.block.value.clone()
                            } else if i < 10 {
                                extract_snippet(&sb.block.value, query, 1000)
                            } else {
                                extract_snippet(&sb.block.value, query, 400)
                            };

                            json!({
                                "label": sb.block.label,
                                "content": content,
                                "created_at": sb.block.created_at,
                                "updated_at": sb.block.updated_at,
                                "relevance_score": sb.score
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
        fuzzy: bool,
    ) -> Result<SearchOutput> {
        // Use database search if available
        if self.handle.has_db_connection() {
            // Note: fuzzy parameter is a placeholder for future fuzzy search implementation
            // Currently just passes through to methods that will use it when available
            let fuzzy_level = if fuzzy { Some(1) } else { None };
            match self
                .handle
                .search_conversations_with_options(
                    Some(query),
                    role,
                    start_time,
                    end_time,
                    limit,
                    fuzzy_level,
                )
                .await
            {
                Ok(scored_messages) => {
                    // Process results with score adjustments and re-sorting
                    let processed = process_search_results(scored_messages, query, limit);

                    let results: Vec<_> = processed
                        .into_iter()
                        .enumerate()
                        .map(|(i, sm)| {
                            // Progressive content display
                            let content = if i < 2 {
                                // Full content for top 2 results
                                sm.message.display_content()
                            } else if i < 5 {
                                // Snippet for next 3 results
                                extract_snippet(&sm.message.display_content(), query, 400)
                            } else {
                                // Shorter snippet for remaining results
                                extract_snippet(&sm.message.display_content(), query, 200)
                            };

                            json!({
                                "id": sm.message.id,
                                "role": sm.message.role.to_string(),
                                "content": content,
                                "created_at": sm.message.created_at,
                                "relevance_score": sm.score
                            })
                        })
                        .collect();

                    Ok(SearchOutput {
                        success: true,
                        message: Some(format!(
                            "Found {} messages matching '{}' (ranked by relevance)",
                            results.len(),
                            query
                        )),
                        results: json!(results),
                    })
                }
                Err(e) => Ok(SearchOutput {
                    success: false,
                    message: Some(format!("Conversation search failed: {:?}", e)),
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
        fuzzy: bool,
    ) -> Result<SearchOutput> {
        // Use database search if available
        if self.handle.has_db_connection() {
            // Note: fuzzy parameter is a placeholder for future fuzzy search implementation
            // Currently just passes through to methods that will use it when available
            let fuzzy_level = if fuzzy { Some(1) } else { None };
            match self
                .handle
                .search_constellation_messages_with_options(
                    Some(query),
                    role,
                    start_time,
                    end_time,
                    limit,
                    fuzzy_level,
                )
                .await
            {
                Ok(scored_messages) => {
                    // Process results with score adjustments and truncation
                    use super::search_utils::{extract_snippet, process_constellation_results};
                    let processed_messages =
                        process_constellation_results(scored_messages, query, limit);

                    let results: Vec<_> = processed_messages
                        .into_iter()
                        .enumerate()
                        .map(|(i, scm)| {
                            // Progressive content display
                            let content = if i < 2 {
                                scm.message.display_content()
                            } else if i < 5 {
                                extract_snippet(&scm.message.display_content(), query, 400)
                            } else {
                                extract_snippet(&scm.message.display_content(), query, 200)
                            };

                            json!({
                                "agent": scm.agent_name,
                                "id": scm.message.id,
                                "role": scm.message.role.to_string(),
                                "content": content,
                                "created_at": scm.message.created_at,
                                "relevance_score": scm.score
                            })
                        })
                        .collect();

                    Ok(SearchOutput {
                        success: true,
                        message: Some(format!(
                            "Found {} constellation messages matching '{}' (ranked by relevance)",
                            results.len(),
                            query
                        )),
                        results: json!(results),
                    })
                }
                Err(e) => Ok(SearchOutput {
                    success: false,
                    message: Some(format!("Constellation message search failed: {:?}", e)),
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

    async fn search_all(&self, query: &str, limit: usize, fuzzy: bool) -> Result<SearchOutput> {
        // Search both domains and combine results
        let archival_result = self.search_archival(query, limit, fuzzy).await?;
        let conv_result = self
            .search_conversations(query, None, None, None, limit, fuzzy)
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
            .execute(
                SearchInput {
                    domain: SearchDomain::ArchivalMemory,
                    query: "color".to_string(),
                    limit: None,
                    role: None,
                    start_time: None,
                    end_time: None,
                    fuzzy: false,
                },
                &crate::tool::ExecutionMeta::default(),
            )
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
