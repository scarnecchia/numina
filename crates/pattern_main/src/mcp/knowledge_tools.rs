use crate::agent::{AgentType, UserId};
use crate::error::{AgentError, PatternError, Result};
use letta::LettaClient;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info, warn};

/// Knowledge tools handler
#[derive(Clone)]
pub struct KnowledgeTools;

impl KnowledgeTools {
    /// Writes insights to an agent's knowledge file for passive sharing
    pub async fn write_agent_knowledge(
        agent: String,
        category: String,
        insight: String,
        context: Option<String>,
    ) -> Result<String> {
        let agent_type = AgentType::from_str(&agent).unwrap();
        // Create knowledge directory if it doesn't exist
        let knowledge_dir = PathBuf::from("knowledge");
        if !knowledge_dir.exists() {
            fs::create_dir_all(&knowledge_dir)
                .await
                .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?;
        }

        // Get the knowledge file name from the agent
        let file_name = agent_type.knowledge_file();

        let file_path = knowledge_dir.join(&file_name);

        // Read existing content or create header if new file
        let mut content = if file_path.exists() {
            fs::read_to_string(&file_path)
                .await
                .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?
        } else {
            format!(
                "# {} Knowledge Base\n\nInsights gathered by the {} agent.\n\n",
                agent_type.name(),
                agent_type.name()
            )
        };

        // Add timestamp
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

        // Format the new entry
        let entry = format!(
            "\n## [{}] {}\n\n**Insight:** {}\n",
            timestamp,
            category.replace('_', " ").to_uppercase(),
            insight
        );

        // Add context if provided
        let entry = if let Some(ctx) = context {
            format!("{}\n**Context:** {}\n", entry, ctx)
        } else {
            entry
        };

        // Add separator
        let entry = format!("{}\n---\n", entry);

        // Append to content
        content.push_str(&entry);

        // Write back to file
        fs::write(&file_path, content)
            .await
            .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?;

        info!(
            "Agent {} wrote {} insight to knowledge base",
            agent_type.name(),
            category
        );

        Ok(format!(
            "Successfully recorded {} insight for {} agent in {}",
            category,
            agent_type.name(),
            file_name
        ))
    }

    /// Reads insights from a specific agent's knowledge file
    pub async fn read_agent_knowledge(
        agent: String,
        category_filter: Option<String>,
        limit: Option<i32>,
    ) -> Result<String> {
        // Parse the agent string into AgentType
        let agent_type = AgentType::from_str(&agent).unwrap();
        let knowledge_dir = PathBuf::from("knowledge");

        // Get the knowledge file name from the agent
        let file_name = agent_type.knowledge_file();

        let file_path = knowledge_dir.join(&file_name);

        if !file_path.exists() {
            return Ok(format!(
                "No knowledge file found for {} agent",
                agent_type.name()
            ));
        }

        let content = fs::read_to_string(&file_path)
            .await
            .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?;

        // Parse insights (simple parsing - can be improved)
        let insights: Vec<&str> = content.split("\n---\n").collect();

        let limit = limit.unwrap_or(10) as usize;
        let mut filtered_insights = Vec::new();

        for insight in insights.iter().rev() {
            if insight.trim().is_empty() {
                continue;
            }

            // Apply category filter if specified
            if let Some(ref filter) = category_filter {
                if !insight.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            filtered_insights.push(*insight);

            if filtered_insights.len() >= limit {
                break;
            }
        }

        if filtered_insights.is_empty() {
            Ok(format!(
                "No insights found for {} agent{}",
                agent_type.name(),
                if category_filter.is_some() {
                    " with specified filter"
                } else {
                    ""
                }
            ))
        } else {
            Ok(filtered_insights.join("\n---\n"))
        }
    }

    /// Lists all available knowledge files and their sizes
    pub async fn list_knowledge_files() -> Result<String> {
        let knowledge_dir = PathBuf::from("knowledge");

        if !knowledge_dir.exists() {
            return Ok("No knowledge directory found. Knowledge files will be created as agents record insights.".to_string());
        }

        let mut files = Vec::new();
        let mut entries = fs::read_dir(&knowledge_dir)
            .await
            .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let metadata = entry.metadata().await.map_err(|e| {
                    PatternError::Agent(AgentError::Other(letta::LettaError::Io(e)))
                })?;

                let size = metadata.len();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                files.push(format!("- {} ({} bytes)", name, size));
            }
        }

        if files.is_empty() {
            Ok("No knowledge files found yet.".to_string())
        } else {
            Ok(format!("Available knowledge files:\n{}", files.join("\n")))
        }
    }

    /// Synchronizes knowledge files with Letta sources for semantic search
    pub async fn sync_knowledge_to_letta(
        letta_client: &Arc<LettaClient>,
        user_id: UserId,
    ) -> Result<String> {
        let knowledge_dir = PathBuf::from("knowledge");

        if !knowledge_dir.exists() {
            return Ok("No knowledge directory found. Create knowledge files first.".to_string());
        }

        let mut synced_files = Vec::new();
        let user_hash = format!("{:x}", user_id.0 % 1000000);

        // Get existing sources for this user
        let existing_sources = match letta_client.sources().list().await {
            Ok(sources) => sources,
            Err(e) => {
                warn!("Failed to list sources: {:?}", e);
                vec![]
            }
        };

        // Read all markdown files
        let mut entries = fs::read_dir(&knowledge_dir)
            .await
            .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PatternError::Agent(AgentError::Other(letta::LettaError::Io(e))))?
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                // Create unique source name for this user
                let source_name =
                    format!("knowledge_{}_{}", file_name.replace(".md", ""), user_hash);

                // Check if source already exists
                let source_exists = existing_sources.iter().any(|s| s.name == source_name);

                if source_exists {
                    debug!("Source {} already exists, skipping", source_name);
                    continue;
                }

                // Read file content
                let content = fs::read_to_string(&path).await.map_err(|e| {
                    PatternError::Agent(AgentError::Other(letta::LettaError::Io(e)))
                })?;

                // Create source in Letta
                let create_request = letta::types::CreateSourceRequest::builder()
                    .name(source_name.clone())
                    .description(format!(
                        "Knowledge base for {} agent",
                        file_name.replace(".md", "")
                    ))
                    .build();

                match letta_client.sources().create(create_request).await {
                    Ok(source) => {
                        info!("Created source {} with ID {:?}", source_name, source.id);

                        // Upload file directly as bytes
                        let upload_filename = format!("{}.md", source_name);

                        match letta_client
                            .sources()
                            .upload_file(
                                source.id.as_ref().unwrap(),
                                upload_filename,
                                content.into_bytes().into(), // Convert String -> Vec<u8> -> Bytes
                                Some("text/markdown".to_string()),
                            )
                            .await
                        {
                            Ok(_) => {
                                info!("Uploaded content to source {}", source_name);
                                synced_files.push(file_name.to_string());
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to upload content to source {}: {:?}",
                                    source_name, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to create source {}: {:?}", source_name, e);
                    }
                }
            }
        }

        if synced_files.is_empty() {
            Ok("No new knowledge files to sync.".to_string())
        } else {
            Ok(format!(
                "Synced {} knowledge files to Letta sources: {}",
                synced_files.len(),
                synced_files.join(", ")
            ))
        }
    }
}

/// Request types for knowledge tools
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WriteAgentKnowledgeRequest {
    /// The agent writing the knowledge (e.g., "pattern", "entropy", "flux", "archive", "momentum", "anchor")
    pub agent: String,
    /// Category of insight (e.g., "task_patterns", "time_estimates", "energy_patterns", "memory_triggers")
    pub category: String,
    /// The actual insight to record
    pub insight: String,
    /// Optional context about when/how this insight was learned
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ReadAgentKnowledgeRequest {
    /// The agent whose knowledge to read (e.g., "pattern", "entropy", "flux", "archive", "momentum", "anchor")
    pub agent: String,
    /// Optional category filter (e.g., "task_patterns")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_filter: Option<String>,
    /// Maximum number of insights to return (default: 10)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SyncKnowledgeRequest {
    /// User ID to sync knowledge for
    pub user_id: i64,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_and_read_knowledge() {
        // Create temp directory for test
        let temp_dir = TempDir::new().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Write knowledge
        let result = KnowledgeTools::write_agent_knowledge(
            AgentType::Entropy.to_string(),
            "task_complexity".to_string(),
            "Documentation tasks take 3x longer than coding".to_string(),
            Some("Observed across multiple sessions".to_string()),
        )
        .await;

        assert!(result.is_ok());

        // Read it back
        let content =
            KnowledgeTools::read_agent_knowledge(AgentType::Entropy.to_string(), None, None)
                .await
                .unwrap();

        assert!(content.contains("Documentation tasks take 3x longer"));
        assert!(content.contains("TASK COMPLEXITY")); // Category has underscore replaced with space
        assert!(content.contains("Observed across multiple sessions"));
    }

    #[tokio::test]
    async fn test_knowledge_file_list() {
        // Create temp directory for test
        let temp_dir = TempDir::new().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Initially no files
        let result = KnowledgeTools::list_knowledge_files().await.unwrap();
        assert!(result.contains("No knowledge directory found"));

        // Write some knowledge
        KnowledgeTools::write_agent_knowledge(
            AgentType::Pattern.to_string(),
            "coordination".to_string(),
            "Multi-agent systems require careful coordination".to_string(),
            None,
        )
        .await
        .unwrap();

        // Now should list the file
        let result = KnowledgeTools::list_knowledge_files().await;
        assert!(result.is_ok(), "Failed to list files: {:?}", result);
        let file_list = result.unwrap();
        assert!(
            file_list.contains("coordination_patterns.md"),
            "File list: {}",
            file_list
        );
    }
}
