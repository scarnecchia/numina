use crate::{context::AgentHandle, error::Result, id::MemoryId, memory::MemoryBlock, tool::AiTool};
use async_trait::async_trait;
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{fs::OpenOptions, io::Write, path::PathBuf, process};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SystemIntegrityInput {
    /// Detailed reason for emergency halt
    pub reason: String,

    /// Severity level: critical, catastrophic, unrecoverable
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_severity() -> String {
    "critical".to_string()
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SystemIntegrityOutput {
    pub status: String,
    pub halt_id: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SystemIntegrityTool {
    halt_log_path: PathBuf,
    handle: AgentHandle,
}

impl SystemIntegrityTool {
    pub fn new(handle: AgentHandle) -> Self {
        let halt_log_path = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pattern")
            .join("halts.log");

        // Ensure directory exists
        if let Some(parent) = halt_log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        Self {
            halt_log_path,
            handle,
        }
    }
}

#[async_trait]
impl AiTool for SystemIntegrityTool {
    type Input = SystemIntegrityInput;
    type Output = SystemIntegrityOutput;

    fn name(&self) -> &str {
        "emergency_halt"
    }

    fn description(&self) -> &str {
        "EMERGENCY ONLY: Immediately terminate the process. Use only when system integrity is at risk or unrecoverable errors occur."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        let reason = params.reason;
        let severity = params.severity;

        let timestamp = Utc::now();
        let halt_id = format!("halt_{}", timestamp.timestamp());

        // Create memory block for the halt event
        let memory_content = json!({
            "event_type": "emergency_halt",
            "halt_id": &halt_id,
            "timestamp": timestamp.to_rfc3339(),
            "reason": &reason,
            "severity": &severity,
            "agent_id": self.handle.agent_id.to_string(),
            "agent_name": &self.handle.name,
        });

        let halt_label = format!("EMERGENCY_HALT_{}", halt_id);
        let memory_block = MemoryBlock::owned_with_id(
            MemoryId::generate(),
            self.handle.memory.owner_id.clone(),
            &halt_label,
            serde_json::to_string_pretty(&memory_content).unwrap(),
        );

        // Store in agent's memory
        match self.handle.memory.upsert_block(&halt_label, memory_block) {
            Ok(_) => tracing::info!("Halt event stored in memory"),
            Err(e) => tracing::error!("Failed to store halt event in memory: {}", e),
        }

        // Write to log file
        let log_entry = format!(
            "[{}] HALT {} - Agent: {} ({}) - Severity: {} - Reason: {}\n",
            timestamp.to_rfc3339(),
            halt_id,
            self.handle.name,
            self.handle.agent_id,
            severity,
            reason
        );

        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.halt_log_path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(log_entry.as_bytes()) {
                    tracing::error!("Failed to write halt log: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to open halt log file: {}", e);
            }
        }

        // Log the final message
        tracing::error!("EMERGENCY HALT INITIATED: {}", reason);

        // Prepare response
        let response = SystemIntegrityOutput {
            status: "halt_initiated".to_string(),
            halt_id: halt_id.clone(),
            message: format!(
                "Emergency halt initiated. Process will terminate. Reason: {}",
                reason
            ),
        };

        // Spawn task to terminate after response is sent
        tokio::spawn(async {
            // Give a moment for response to be sent and logs to flush
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // Terminate the process
            process::exit(1);
        });

        // Return response immediately so agent can send it
        Ok(response)
    }
}
