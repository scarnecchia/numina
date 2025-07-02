use rmcp::{model::ServerInfo, schemars, serde_json, tool, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct PatternServer {
    inner: Arc<PatternServerInner>,
}

#[derive(Debug)]
struct PatternServerInner {
    letta_client: Option<Arc<letta::LettaClient>>,
    db_pool: Option<Arc<sqlx::SqlitePool>>,
}

impl PatternServer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(PatternServerInner {
                letta_client: None,
                db_pool: None,
            }),
        }
    }

    pub fn with_letta_client(mut self, client: letta::LettaClient) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("No other references during construction")
            .letta_client = Some(Arc::new(client));
        self
    }

    pub fn with_db_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("No other references during construction")
            .db_pool = Some(Arc::new(pool));
        self
    }
}

impl ServerHandler for PatternServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: "pattern".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some("Pattern MCP server for Letta agent management with calendar scheduling and activity monitoring".to_string()),
            ..Default::default()
        }
    }
}

#[tool(tool_box)]
impl PatternServer {
    #[tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(&self, #[tool(aggr)] req: ScheduleEventRequest) -> String {
        info!("Scheduling event: {}", req.title);
        debug!(?req, "Full event request");

        // TODO: Implement actual scheduling logic
        let response = format!(
            "Event '{}' scheduled for {} minutes",
            req.title, req.duration_minutes
        );

        info!("Event scheduled successfully");
        response
    }

    #[tool(description = "Send message to user via Discord")]
    async fn send_message(
        &self,
        #[tool(param)] channel_id: u64,
        #[tool(param)] message: String,
    ) -> String {
        info!(channel_id, "Sending message to Discord channel");
        debug!(%message, "Message content");

        // TODO: Implement Discord integration
        let response = format!("Message sent to channel {}: {}", channel_id, message);

        info!("Message sent successfully");
        response
    }

    #[tool(description = "Check activity state for interruption timing")]
    fn check_activity_state(&self) -> String {
        debug!("Checking activity state");

        // TODO: Implement platform-specific activity monitoring
        let state = ActivityState::default();
        let json = serde_json::to_string(&state).expect("ActivityState should serialize");

        debug!(?state, "Current activity state");
        json
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleEventRequest {
    pub title: String,
    pub duration_minutes: u32,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityState {
    pub interruptibility: InterruptibilityScore,
    pub current_app: Option<String>,
    pub idle_minutes: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum InterruptibilityScore {
    Low,
    Medium,
    High,
}

impl Default for ActivityState {
    fn default() -> Self {
        Self {
            interruptibility: InterruptibilityScore::Medium,
            current_app: None,
            idle_minutes: 0.0,
        }
    }
}
