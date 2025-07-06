//! Core MCP tools for agent interactions

use crate::agent::{constellation::MultiAgentSystem, ModelCapability, UserId};
use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, Content},
    Error as McpError,
};
use serde_json::json;
use std::{future::Future, str::FromStr, sync::Arc};
use tracing::{debug, error, info};

use super::{
    ChatWithAgentRequest, GetAgentMemoryRequest, RecordEnergyStateRequest, ScheduleEventRequest,
    SendGroupMessageRequest, UpdateAgentMemoryRequest, UpdateAgentModelRequest,
};

/// Core MCP tools handler
#[derive(Clone)]
pub struct CoreTools {
    pub multi_agent_system: Arc<MultiAgentSystem>,
}

// Core MCP tool implementations
impl CoreTools {
    /// Parse user_id from string (handles Discord IDs and other large numbers)
    fn parse_user_id(user_id_str: &str) -> Result<i64, McpError> {
        user_id_str.parse::<i64>().map_err(|e| {
            McpError::invalid_params(
                "Invalid user_id format",
                Some(json!({
                    "user_id": user_id_str,
                    "error": e.to_string(),
                    "hint": "user_id must be a valid integer"
                })),
            )
        })
    }
    pub async fn chat_with_agent(
        &self,
        params: Parameters<ChatWithAgentRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        let user_id = Self::parse_user_id(&params.user_id)?;
        info!(user_id = user_id, agent_id = ?params.agent_id, "Sending message to agent");
        debug!(message = %params.message, "Message content");

        match self
            .chat_with_agent_internal(UserId(user_id), &params.message, params.agent_id.as_deref())
            .await
        {
            Ok(response) => {
                info!("Agent responded successfully");
                Ok(CallToolResult::success(vec![Content::text(response)]))
            }
            Err(e) => {
                error!("Agent error: {}", e);
                Err(McpError::internal_error(
                    "Error communicating with agent",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "agent_communication_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn get_agent_memory(
        &self,
        params: Parameters<GetAgentMemoryRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        let user_id = Self::parse_user_id(&params.user_id)?;
        info!(user_id = user_id, agent_id = ?params.agent_id, "Getting agent memory");

        match self
            .get_agent_memory_internal(UserId(user_id), params.agent_id.as_deref())
            .await
        {
            Ok(memory) => {
                info!("Retrieved agent memory successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&memory).unwrap_or_else(|_| "Error".to_string()),
                )]))
            }
            Err(e) => {
                error!("Error getting agent memory: {}", e);
                Err(McpError::internal_error(
                    "Error retrieving agent memory",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "memory_retrieval_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn update_agent_memory(
        &self,
        params: Parameters<UpdateAgentMemoryRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        let user_id = Self::parse_user_id(&params.user_id)?;
        info!(user_id = user_id, agent_id = ?params.agent_id, "Updating agent memory");

        let updates: serde_json::Value = match serde_json::from_str(&params.memory_json) {
            Ok(json) => json,
            Err(e) => {
                error!("Invalid JSON in memory update: {}", e);
                return Err(McpError::invalid_params(
                    "Invalid JSON in memory_json parameter",
                    Some(json!({
                        "parameter": "memory_json",
                        "error_type": "json_parse_error",
                        "details": e.to_string(),
                        "line": e.line(),
                        "column": e.column()
                    })),
                ));
            }
        };

        match self
            .update_agent_memory_internal(UserId(user_id), params.agent_id.as_deref(), &updates)
            .await
        {
            Ok(()) => {
                info!("Agent memory updated successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    "Memory updated successfully",
                )]))
            }
            Err(e) => {
                error!("Error updating agent memory: {}", e);
                Err(McpError::internal_error(
                    "Error updating agent memory",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "memory_update_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn update_agent_model(
        &self,
        params: Parameters<UpdateAgentModelRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        let user_id = Self::parse_user_id(&params.user_id)?;
        info!(
            user_id = user_id,
            agent_id = %params.agent_id,
            capability = ?params.capability,
            "Updating agent model capability"
        );

        // Parse agent ID
        let agent_id = match params.agent_id.parse::<crate::agent::AgentId>() {
            Ok(id) => id,
            Err(e) => {
                error!("Invalid agent ID: {}", e);
                return Err(McpError::invalid_params(
                    "Invalid agent_id parameter",
                    Some(json!({
                        "parameter": "agent_id",
                        "error_type": "invalid_agent_id",
                        "details": e.to_string(),
                        "allowed_values": ["pattern", "entropy", "flux", "archive", "momentum", "anchor"]
                    })),
                ));
            }
        };

        // Parse capability from string
        let capability = ModelCapability::from_str(&params.capability).map_err(|e| {
            McpError::invalid_params(
                "Invalid capability level",
                Some(json!({
                    "provided": params.capability,
                    "error": e.to_string(),
                    "valid_levels": ["routine", "interactive", "investigative", "critical"]
                })),
            )
        })?;

        match self
            .multi_agent_system
            .update_agent_model_capability(UserId(user_id), &agent_id, capability)
            .await
        {
            Ok(()) => {
                info!("Agent model capability updated successfully");
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Successfully updated {} to {} capability level",
                    params.agent_id, params.capability
                ))]))
            }
            Err(e) => {
                error!("Error updating agent model capability: {}", e);
                Err(McpError::internal_error(
                    "Error updating agent model capability",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "capability": format!("{:?}", params.capability),
                        "error_type": "model_update_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn schedule_event(
        &self,
        params: Parameters<ScheduleEventRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;

        // Log all incoming parameters
        info!(
            "schedule_event called with params: user_id='{}', title='{}', start_time='{}', end_time='{}', description={:?}, location={:?}",
            params.user_id, params.title, params.start_time, params.end_time, params.description, params.location
        );

        let user_id = match Self::parse_user_id(&params.user_id) {
            Ok(id) => {
                info!(
                    "Successfully parsed user_id: '{}' -> {}",
                    params.user_id, id
                );
                id
            }
            Err(e) => {
                error!("Failed to parse user_id '{}': {:?}", params.user_id, e);
                return Err(e);
            }
        };

        info!(user_id = user_id, title = params.title, "Scheduling event");

        // Helper function to parse relative time expressions
        fn parse_time_expression(time_expr: &str) -> Option<chrono::DateTime<chrono::Utc>> {
            let time_expr_lower = time_expr.to_lowercase();
            let now = chrono::Utc::now();

            // Try to parse as RFC3339 first
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(time_expr) {
                return Some(dt.with_timezone(&chrono::Utc));
            }

            // Handle relative time expressions
            if time_expr_lower.contains("in ") {
                // Extract number and unit from patterns like "in 10 minutes", "in 5 min", etc.
                let parts: Vec<&str> = time_expr_lower.split_whitespace().collect();
                if parts.len() >= 3 && parts[0] == "in" {
                    if let Ok(amount) = parts[1].parse::<i64>() {
                        let unit = parts[2..].join(" ");
                        match unit.as_str() {
                            "minute" | "minutes" | "min" | "mins" => {
                                return Some(now + chrono::Duration::minutes(amount));
                            }
                            "hour" | "hours" | "hr" | "hrs" => {
                                return Some(now + chrono::Duration::hours(amount));
                            }
                            "day" | "days" => {
                                return Some(now + chrono::Duration::days(amount));
                            }
                            _ => {}
                        }
                    }
                }
            }

            None
        }

        // Parse the start and end times
        let start_time = match parse_time_expression(&params.start_time) {
            Some(dt) => {
                info!(
                    "Parsed start_time '{}' as {}",
                    params.start_time,
                    dt.to_rfc3339()
                );
                dt
            }
            None => {
                error!("Unable to parse start_time: '{}'", params.start_time);
                return Err(McpError::invalid_params(
                    "Invalid start_time format. Use RFC3339 format (e.g., 2024-01-15T14:30:00Z) or relative time (e.g., 'in 10 minutes')",
                    Some(json!({
                        "parameter": "start_time",
                        "value": params.start_time,
                        "error_type": "datetime_parse_error",
                        "examples": ["2024-01-15T14:30:00Z", "in 10 minutes", "in 1 hour"]
                    })),
                ));
            }
        };

        let end_time = match parse_time_expression(&params.end_time) {
            Some(dt) => {
                info!(
                    "Parsed end_time '{}' as {}",
                    params.end_time,
                    dt.to_rfc3339()
                );
                dt
            }
            None => {
                error!("Unable to parse end_time: '{}'", params.end_time);
                return Err(McpError::invalid_params(
                    "Invalid end_time format. Use RFC3339 format (e.g., 2024-01-15T15:30:00Z) or relative time (e.g., 'in 10 minutes')",
                    Some(json!({
                        "parameter": "end_time",
                        "value": params.end_time,
                        "error_type": "datetime_parse_error",
                        "examples": ["2024-01-15T15:30:00Z", "in 10 minutes", "in 1 hour"]
                    })),
                ));
            }
        };

        // Validate that end time is after start time
        if end_time <= start_time {
            return Err(McpError::invalid_params(
                "End time must be after start time",
                Some(json!({
                    "start_time": params.start_time,
                    "end_time": params.end_time,
                    "error_type": "invalid_time_range"
                })),
            ));
        }

        // Get database handle from multi_agent_system
        let db = self.multi_agent_system.db();

        // Create the event in the database
        match db
            .create_event(
                user_id,
                params.title.clone(),
                params.description.clone(),
                start_time,
                end_time,
                params.location.clone(),
            )
            .await
        {
            Ok(event_id) => {
                info!("Event created successfully with ID {}", event_id);

                // If agents are initialized, update Pattern's context
                if let Ok(true) = self
                    .multi_agent_system
                    .is_user_initialized(UserId(user_id))
                    .await
                {
                    let event_context = format!(
                        "New event scheduled: {} from {} to {}",
                        params.title, params.start_time, params.end_time
                    );

                    // Update active context memory
                    let _ = self
                        .multi_agent_system
                        .update_shared_memory(UserId(user_id), "active_context", &event_context)
                        .await;
                }

                // Calculate duration for ADHD time estimation
                let duration = end_time - start_time;
                let duration_minutes = duration.num_minutes();

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Event '{}' scheduled successfully!\nID: {}\nStart: {}\nEnd: {}\nDuration: {} minutes{}",
                    params.title,
                    event_id,
                    params.start_time,
                    params.end_time,
                    duration_minutes,
                    if let Some(loc) = params.location {
                        format!("\nLocation: {}", loc)
                    } else {
                        String::new()
                    }
                ))]))
            }
            Err(e) => {
                error!("Failed to create event: {}", e);
                Err(McpError::internal_error(
                    "Failed to create event",
                    Some(json!({
                        "user_id": params.user_id,
                        "title": params.title,
                        "error_type": "database_error",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn check_activity_state(&self) -> std::result::Result<CallToolResult, McpError> {
        info!("Checking activity state");

        // For now, we'll check the latest energy state from the database
        // In the future, this could integrate with system activity monitors

        // Get the current user context from environment or default
        // TODO: This should be passed as a parameter or determined from context
        let default_user_id = 1; // Default user for now

        let db = self.multi_agent_system.db();

        match db.get_latest_energy_state(default_user_id).await {
            Ok(Some(energy_state)) => {
                // Parse the created_at timestamp
                let created_at = chrono::DateTime::parse_from_rfc3339(&energy_state.created_at)
                    .unwrap_or_else(|_| chrono::Utc::now().into());

                let minutes_since_update = chrono::Utc::now()
                    .signed_duration_since(created_at.with_timezone(&chrono::Utc))
                    .num_minutes();

                // Determine interruptibility based on attention state and energy level
                let interruptible = match energy_state.attention_state.as_str() {
                    "hyperfocus" => false,                       // Never interrupt hyperfocus
                    "focused" => energy_state.energy_level <= 3, // Only if low energy
                    "scattered" => true, // Always interruptible when scattered
                    _ => true,           // Default to interruptible
                };

                // Determine if user needs a break
                let needs_break = energy_state
                    .last_break_minutes
                    .map(|mins| mins > 90) // Need break after 90 minutes
                    .unwrap_or(false);

                let activity_data = json!({
                    "interruptible": interruptible,
                    "current_focus": energy_state.attention_state,
                    "energy_level": energy_state.energy_level,
                    "mood": energy_state.mood,
                    "last_break_minutes": energy_state.last_break_minutes,
                    "minutes_since_update": minutes_since_update,
                    "needs_break": needs_break,
                    "notes": energy_state.notes
                });

                info!("Activity state retrieved: {:?}", activity_data);

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&activity_data)
                        .unwrap_or_else(|_| "Error".to_string()),
                )]))
            }
            Ok(None) => {
                // No energy state recorded yet
                let default_state = json!({
                    "interruptible": true,
                    "current_focus": "unknown",
                    "energy_level": 5,
                    "mood": null,
                    "last_break_minutes": null,
                    "minutes_since_update": null,
                    "needs_break": false,
                    "notes": "No activity state recorded yet"
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&default_state)
                        .unwrap_or_else(|_| "Error".to_string()),
                )]))
            }
            Err(e) => {
                error!("Failed to get activity state: {}", e);
                Err(McpError::internal_error(
                    "Failed to retrieve activity state",
                    Some(json!({
                        "error_type": "database_error",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(description = "Send a message to a group of agents")]
    pub async fn send_group_message(
        &self,
        params: Parameters<SendGroupMessageRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        let user_id = Self::parse_user_id(&params.user_id)?;
        info!(
            user_id = user_id,
            group_name = params.group_name,
            "Sending message to group"
        );
        debug!(message = %params.message, "Message content");

        match self
            .multi_agent_system
            .send_message_to_group(UserId(user_id), &params.group_name, &params.message)
            .await
        {
            Ok(response) => {
                info!("Group responded successfully");
                // Extract assistant messages from the response
                let messages = response
                    .messages
                    .iter()
                    .filter_map(|msg| match msg {
                        letta::types::LettaMessageUnion::AssistantMessage(m) => {
                            Some(m.content.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                let response_text = if messages.is_empty() {
                    "No response from group".to_string()
                } else {
                    messages.join("\n\n")
                };

                Ok(CallToolResult::success(vec![Content::text(response_text)]))
            }
            Err(e) => {
                error!("Group error: {}", e);
                Err(McpError::internal_error(
                    "Error communicating with group",
                    Some(json!({
                        "group_name": params.group_name,
                        "user_id": params.user_id,
                        "error_type": "group_communication_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn record_energy_state(
        &self,
        params: Parameters<RecordEnergyStateRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let user_id = Self::parse_user_id(&params.user_id)?;
        info!(
            user_id = user_id,
            energy_level = params.energy_level,
            attention_state = params.attention_state,
            "Recording energy state"
        );

        // Validate energy level
        if params.energy_level < 1 || params.energy_level > 10 {
            return Err(McpError::invalid_params(
                "Energy level must be between 1 and 10",
                Some(json!({
                    "parameter": "energy_level",
                    "value": params.energy_level,
                    "error_type": "out_of_range"
                })),
            ));
        }

        // Validate attention state
        let valid_states = [
            "focused",
            "scattered",
            "hyperfocus",
            "distracted",
            "flowing",
            "stuck",
        ];
        if !valid_states.contains(&params.attention_state.as_str()) {
            return Err(McpError::invalid_params(
                "Invalid attention state",
                Some(json!({
                    "parameter": "attention_state",
                    "value": params.attention_state,
                    "valid_values": valid_states,
                    "error_type": "invalid_value"
                })),
            ));
        }

        let db = self.multi_agent_system.db();

        match db
            .record_energy_state(
                user_id,
                params.energy_level,
                params.attention_state.clone(),
                params.mood.clone(),
                params.last_break_minutes,
                params.notes.clone(),
            )
            .await
        {
            Ok(state_id) => {
                info!("Energy state recorded successfully with ID {}", state_id);

                // If agents are initialized, update Momentum agent's context
                if let Ok(true) = self
                    .multi_agent_system
                    .is_user_initialized(UserId(user_id))
                    .await
                {
                    let energy_context = format!(
                        "Energy update: Level {}/10, State: {}, Mood: {}",
                        params.energy_level,
                        params.attention_state,
                        params.mood.as_deref().unwrap_or("unspecified")
                    );

                    // Update active context memory
                    let _ = self
                        .multi_agent_system
                        .update_shared_memory(UserId(user_id), "active_context", &energy_context)
                        .await;
                }

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Energy state recorded successfully!\nID: {}\nEnergy: {}/10\nAttention: {}\nMood: {}{}",
                    state_id,
                    params.energy_level,
                    params.attention_state,
                    params.mood.as_deref().unwrap_or("not specified"),
                    if let Some(notes) = params.notes {
                        format!("\nNotes: {}", notes)
                    } else {
                        String::new()
                    }
                ))]))
            }
            Err(e) => {
                error!("Failed to record energy state: {}", e);
                Err(McpError::internal_error(
                    "Failed to record energy state",
                    Some(json!({
                        "user_id": params.user_id,
                        "error_type": "database_error",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    // Internal helper methods
    pub(crate) async fn chat_with_agent_internal(
        &self,
        user_id: UserId,
        message: &str,
        agent_id: Option<&str>,
    ) -> miette::Result<String> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Update active context
        let context = format!("task: chat | message: {} | agent: {:?}", message, agent_id);
        self.multi_agent_system
            .update_shared_memory(user_id, "active_context", &context)
            .await?;

        // Route to specific agent and get response
        let response = self
            .multi_agent_system
            .send_message_to_agent(user_id, agent_id, message)
            .await?;

        Ok(response)
    }

    pub(crate) async fn get_agent_memory_internal(
        &self,
        user_id: UserId,
        agent_id: Option<&str>,
    ) -> miette::Result<serde_json::Value> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Get memory state
        let block_name = agent_id.unwrap_or("current_state");
        let memory_content = self
            .multi_agent_system
            .get_shared_memory(user_id, block_name)
            .await?;

        Ok(serde_json::json!({
            "block_name": block_name,
            "content": memory_content
        }))
    }

    pub(crate) async fn update_agent_memory_internal(
        &self,
        user_id: UserId,
        _agent_id: Option<&str>,
        updates: &serde_json::Value,
    ) -> miette::Result<()> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Extract memory block updates
        if let Some(updates_obj) = updates.as_object() {
            for (block_name, value) in updates_obj {
                if let Some(value_str) = value.as_str() {
                    self.multi_agent_system
                        .update_shared_memory(user_id, block_name, value_str)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
