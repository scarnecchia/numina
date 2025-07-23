//! MCP tools for sleeptime monitoring

use crate::{
    agent::{constellation::MultiAgentSystem, human::UserId},
    db::Database,
    sleeptime::SleeptimeMonitor,
};
use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, ErrorData},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

/// Request to trigger a manual sleeptime check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerSleeptimeCheckRequest {
    /// User ID to trigger the check for
    pub user_id: String,
    /// Optional reason for the manual check
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Request to update sleeptime monitoring state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSleeptimeStateRequest {
    /// User ID to update state for
    pub user_id: String,
    /// Type of update (break, water, movement, task)
    pub update_type: String,
    /// Optional details for the update
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Sleeptime monitoring tools
pub struct SleeptimeTools {
    db: Arc<Database>,
    multi_agent_system: Arc<MultiAgentSystem>,
    monitors: Arc<tokio::sync::RwLock<Vec<Arc<SleeptimeMonitor>>>>,
}

impl SleeptimeTools {
    pub fn new(
        db: Arc<Database>,
        multi_agent_system: Arc<MultiAgentSystem>,
        monitors: Arc<tokio::sync::RwLock<Vec<Arc<SleeptimeMonitor>>>>,
    ) -> Self {
        Self {
            db,
            multi_agent_system,
            monitors,
        }
    }

    /// Get the sleeptime monitor for a user
    async fn get_monitor(&self, user_id: UserId) -> Option<Arc<SleeptimeMonitor>> {
        let monitors = self.monitors.read().await;
        monitors
            .iter()
            .find(|m| m.user_id() == user_id)
            .map(|m| Arc::clone(m))
    }

    /// Trigger a manual sleeptime check
    pub async fn trigger_sleeptime_check(
        &self,
        params: Parameters<TriggerSleeptimeCheckRequest>,
    ) -> Result<CallToolResult, rmcp::ServiceError> {
        let params = params.0;
        // Parse user ID
        let user_id = params.user_id.parse::<i64>().map_err(|e| {
            rmcp::ServiceError::McpError(ErrorData::invalid_params(
                format!("Invalid user ID: {}", e),
                None,
            ))
        })?;

        info!(
            "Manual sleeptime check requested for user {} - reason: {:?}",
            user_id, params.reason
        );

        // Get the monitor for this user
        let monitor = self.get_monitor(UserId(user_id)).await.ok_or_else(|| {
            rmcp::ServiceError::McpError(ErrorData::resource_not_found(
                format!("No sleeptime monitor found for user {}", user_id),
                None,
            ))
        })?;

        // Trigger the manual check
        monitor.trigger_manual_check().await.map_err(|e| {
            error!("Failed to trigger manual check: {:?}", e);
            rmcp::ServiceError::McpError(ErrorData::internal_error(
                format!("Failed to trigger check: {}", e),
                None,
            ))
        })?;

        let response = format!(
            "Manual sleeptime check triggered for user {}{}",
            user_id,
            params
                .reason
                .as_ref()
                .map(|r| format!(" - Reason: {}", r))
                .unwrap_or_default()
        );

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            response,
        )]))
    }

    /// Update sleeptime monitoring state
    pub async fn update_sleeptime_state(
        &self,
        params: Parameters<UpdateSleeptimeStateRequest>,
    ) -> Result<CallToolResult, rmcp::ServiceError> {
        let params = params.0;
        // Parse user ID
        let user_id = params.user_id.parse::<i64>().map_err(|e| {
            rmcp::ServiceError::McpError(ErrorData::invalid_params(
                format!("Invalid user ID: {}", e),
                None,
            ))
        })?;

        info!(
            "Updating sleeptime state for user {} - type: {}",
            user_id, params.update_type
        );

        // Get the monitor for this user
        let monitor = self.get_monitor(UserId(user_id)).await.ok_or_else(|| {
            rmcp::ServiceError::McpError(ErrorData::resource_not_found(
                format!("No sleeptime monitor found for user {}", user_id),
                None,
            ))
        })?;

        // Update based on type
        let response = match params.update_type.as_str() {
            "break" => {
                monitor.record_break().await.map_err(|e| {
                    error!("Failed to record break: {:?}", e);
                    rmcp::ServiceError::McpError(ErrorData::internal_error(
                        format!("Failed to record break: {}", e),
                        None,
                    ))
                })?;
                "Break recorded - timers reset"
            }
            "water" => {
                monitor.record_water().await.map_err(|e| {
                    error!("Failed to record water: {:?}", e);
                    rmcp::ServiceError::McpError(ErrorData::internal_error(
                        format!("Failed to record water: {}", e),
                        None,
                    ))
                })?;
                "Hydration recorded"
            }
            "movement" => {
                monitor.record_movement().await.map_err(|e| {
                    error!("Failed to record movement: {:?}", e);
                    rmcp::ServiceError::McpError(ErrorData::internal_error(
                        format!("Failed to record movement: {}", e),
                        None,
                    ))
                })?;
                "Movement/stretch recorded"
            }
            "task" => {
                if let Some(task) = params.details {
                    monitor
                        .update_current_task(task.clone())
                        .await
                        .map_err(|e| {
                            error!("Failed to update task: {:?}", e);
                            rmcp::ServiceError::McpError(ErrorData::internal_error(
                                format!("Failed to update task: {}", e),
                                None,
                            ))
                        })?;
                    &format!("Current task updated to: {}", task)
                } else {
                    return Err(rmcp::ServiceError::McpError(ErrorData::invalid_params(
                        "Task update requires 'details' field with task description".to_string(),
                        None,
                    )));
                }
            }
            _ => {
                return Err(rmcp::ServiceError::McpError(ErrorData::invalid_params(
                    format!(
                        "Unknown update type '{}'. Valid types: break, water, movement, task",
                        params.update_type
                    ),
                    None,
                )));
            }
        };

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            response,
        )]))
    }
}
