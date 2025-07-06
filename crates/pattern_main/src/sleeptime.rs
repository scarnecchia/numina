//! Two-tier sleeptime monitoring for ADHD support
//!
//! This module implements a cost-optimized background monitoring system:
//! - Tier 1: Lightweight rules-based checks every 20 minutes
//! - Tier 2: Pattern agent intervention when concerning patterns detected

use crate::{
    agent::{constellation::MultiAgentSystem, human::UserId, StandardMemoryBlock},
    db::Database,
    error::Result,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Configuration for sleeptime monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleeptimeConfig {
    /// Interval for lightweight checks (seconds)
    pub tier1_interval_secs: u64,
    /// Maximum hyperfocus duration before intervention (minutes)
    pub max_hyperfocus_minutes: i32,
    /// Maximum time without movement before reminder (minutes)
    pub max_sedentary_minutes: i32,
    /// Maximum time without water before reminder (minutes)
    pub max_water_gap_minutes: i32,
    /// Minimum energy level before suggesting break
    pub min_energy_level: i32,
    /// Whether to use expensive models for tier 2
    pub use_expensive_models: bool,
}

impl Default for SleeptimeConfig {
    fn default() -> Self {
        Self {
            tier1_interval_secs: 1200, // 20 minutes
            max_hyperfocus_minutes: 90,
            max_sedentary_minutes: 120,
            max_water_gap_minutes: 120,
            min_energy_level: 4,
            use_expensive_models: true,
        }
    }
}

/// State tracked by the monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorState {
    /// Last time user took a break
    pub last_break_time: Option<DateTime<Utc>>,
    /// Last time user moved/stretched
    pub last_movement_time: Option<DateTime<Utc>>,
    /// Last time user had water
    pub last_water_time: Option<DateTime<Utc>>,
    /// Current task start time
    pub current_task_start: Option<DateTime<Utc>>,
    /// Current task description
    pub current_task: Option<String>,
    /// Number of tier 1 checks since last tier 2
    pub checks_since_intervention: u32,
    /// Last Pattern intervention time
    pub last_pattern_wake: Option<DateTime<Utc>>,
}

impl Default for MonitorState {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            last_break_time: Some(now),
            last_movement_time: Some(now),
            last_water_time: Some(now),
            current_task_start: None,
            current_task: None,
            checks_since_intervention: 0,
            last_pattern_wake: None,
        }
    }
}

/// Reasons to wake Pattern for intervention
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InterventionReason {
    Hyperfocus { duration_minutes: i32 },
    NoMovement { duration_minutes: i32 },
    NoWater { duration_minutes: i32 },
    LowEnergy { level: i32 },
    TaskSwitch { from: String, to: String },
    UserRequest,
    ScheduledCheck,
}

/// Two-tier sleeptime monitor
pub struct SleeptimeMonitor {
    config: SleeptimeConfig,
    db: Arc<Database>,
    multi_agent_system: Arc<MultiAgentSystem>,
    state: Arc<RwLock<MonitorState>>,
    user_id: UserId,
}

impl SleeptimeMonitor {
    pub fn new(
        config: SleeptimeConfig,
        db: Arc<Database>,
        multi_agent_system: Arc<MultiAgentSystem>,
        user_id: UserId,
    ) -> Self {
        Self {
            config,
            db,
            multi_agent_system,
            state: Arc::new(RwLock::new(MonitorState::default())),
            user_id,
        }
    }

    /// Get the user ID this monitor is tracking
    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    /// Start the monitoring loop
    pub async fn start(self: Arc<Self>) -> Result<()> {
        let monitor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
                monitor.config.tier1_interval_secs,
            ));

            loop {
                interval.tick().await;

                if let Err(e) = monitor.run_tier1_check().await {
                    error!("Tier 1 check failed: {:?}", e);
                }
            }
        });

        Ok(())
    }

    /// Run lightweight tier 1 check
    async fn run_tier1_check(&self) -> Result<()> {
        info!("Running tier 1 sleeptime check");

        // Get latest energy state
        let energy_state = self.db.get_latest_energy_state(self.user_id.0).await?;

        // Check various conditions
        let mut intervention_reasons = Vec::new();
        let now = Utc::now();

        // Update state
        let mut state = self.state.write().await;
        state.checks_since_intervention += 1;

        // Check hyperfocus
        if let Some(task_start) = state.current_task_start {
            let duration = now.signed_duration_since(task_start);
            let minutes = duration.num_minutes() as i32;

            if minutes > self.config.max_hyperfocus_minutes {
                intervention_reasons.push(InterventionReason::Hyperfocus {
                    duration_minutes: minutes,
                });
            }
        }

        // Check movement
        if let Some(last_movement) = state.last_movement_time {
            let duration = now.signed_duration_since(last_movement);
            let minutes = duration.num_minutes() as i32;

            if minutes > self.config.max_sedentary_minutes {
                intervention_reasons.push(InterventionReason::NoMovement {
                    duration_minutes: minutes,
                });
            }
        }

        // Check water
        if let Some(last_water) = state.last_water_time {
            let duration = now.signed_duration_since(last_water);
            let minutes = duration.num_minutes() as i32;

            if minutes > self.config.max_water_gap_minutes {
                intervention_reasons.push(InterventionReason::NoWater {
                    duration_minutes: minutes,
                });
            }
        }

        // Check energy level
        if let Some(energy) = energy_state {
            if energy.energy_level < self.config.min_energy_level {
                intervention_reasons.push(InterventionReason::LowEnergy {
                    level: energy.energy_level,
                });

                // Update state with break info if available
                if let Some(break_mins) = energy.last_break_minutes {
                    state.last_break_time = Some(now - Duration::minutes(break_mins as i64));
                }
            }

            // Detect task switch
            if energy.attention_state == "scattered" || energy.attention_state == "switching" {
                if let Some(current_task) = &state.current_task {
                    if let Some(notes) = &energy.notes {
                        if !notes.contains(current_task) {
                            intervention_reasons.push(InterventionReason::TaskSwitch {
                                from: current_task.clone(),
                                to: notes.clone(),
                            });
                        }
                    }
                }
            }
        }

        // Periodic check (every 10 tier 1 checks = ~3.3 hours)
        if state.checks_since_intervention >= 10 {
            intervention_reasons.push(InterventionReason::ScheduledCheck);
        }

        // Update monitoring state
        let monitoring_summary = format!(
            "Tier 1 check: {} concerns found. Hyperfocus: {}min, Sedentary: {}min, Water gap: {}min",
            intervention_reasons.len(),
            state.current_task_start
                .map(|t| now.signed_duration_since(t).num_minutes())
                .unwrap_or(0),
            state.last_movement_time
                .map(|t| now.signed_duration_since(t).num_minutes())
                .unwrap_or(0),
            state.last_water_time
                .map(|t| now.signed_duration_since(t).num_minutes())
                .unwrap_or(0),
        );

        debug!("{}", monitoring_summary);

        // Update shared memory with monitoring state
        if let Err(e) = self
            .multi_agent_system
            .update_shared_memory(
                self.user_id,
                StandardMemoryBlock::CurrentState.id().as_str(),
                &monitoring_summary,
            )
            .await
        {
            warn!("Failed to update monitoring state: {:?}", e);
        }

        drop(state); // Release write lock before potential tier 2

        // Trigger tier 2 if needed
        if !intervention_reasons.is_empty() {
            self.trigger_tier2_intervention(intervention_reasons)
                .await?;
        }

        Ok(())
    }

    /// Trigger Pattern intervention for concerning patterns
    async fn trigger_tier2_intervention(&self, reasons: Vec<InterventionReason>) -> Result<()> {
        info!(
            "Triggering tier 2 intervention for {} reasons",
            reasons.len()
        );

        // Reset counter
        self.state.write().await.checks_since_intervention = 0;

        // Build intervention context
        let context = self.build_intervention_context(&reasons).await?;

        // Wake Pattern with context
        let message = format!(
            "SLEEPTIME CHECK: I need to assess the current situation.\n\n{}",
            context
        );

        // Send to Pattern via the main group for coordinated response
        match self
            .multi_agent_system
            .send_message_to_group(self.user_id, "main", &message)
            .await
        {
            Ok(response) => {
                info!("Pattern intervention complete");

                // Update last wake time
                self.state.write().await.last_pattern_wake = Some(Utc::now());

                // Convert response to string and parse state updates
                let response_text = format!("{:?}", response);
                self.parse_intervention_response(&response_text).await?;
            }
            Err(e) => {
                error!("Failed to wake Pattern: {:?}", e);
            }
        }

        Ok(())
    }

    /// Build detailed context for Pattern intervention
    async fn build_intervention_context(&self, reasons: &[InterventionReason]) -> Result<String> {
        let mut context = String::from("Intervention triggered by:\n");

        for reason in reasons {
            context.push_str(&match reason {
                InterventionReason::Hyperfocus { duration_minutes } => {
                    format!(
                        "- Hyperfocus detected: {} minutes on current task\n",
                        duration_minutes
                    )
                }
                InterventionReason::NoMovement { duration_minutes } => {
                    format!("- No movement for {} minutes\n", duration_minutes)
                }
                InterventionReason::NoWater { duration_minutes } => {
                    format!("- No water for {} minutes\n", duration_minutes)
                }
                InterventionReason::LowEnergy { level } => {
                    format!("- Low energy level: {}/10\n", level)
                }
                InterventionReason::TaskSwitch { from, to } => {
                    format!("- Task switch detected: {} → {}\n", from, to)
                }
                InterventionReason::UserRequest => "- User requested check-in\n".to_string(),
                InterventionReason::ScheduledCheck => "- Scheduled periodic check\n".to_string(),
            });
        }

        // Add current state summary
        let state = self.state.read().await;
        context.push_str(&format!("\nCurrent state:\n"));

        if let Some(task) = &state.current_task {
            context.push_str(&format!("- Working on: {}\n", task));
        }

        if let Some(energy) = self.db.get_latest_energy_state(self.user_id.0).await? {
            context.push_str(&format!(
                "- Energy: {}/10, Attention: {}, Mood: {}\n",
                energy.energy_level,
                energy.attention_state,
                energy.mood.as_deref().unwrap_or("neutral")
            ));
        }

        // Add any upcoming events
        let events = self.db.get_upcoming_events(self.user_id.0, 60).await?;
        if !events.is_empty() {
            context.push_str("\nUpcoming events:\n");
            for event in events.iter().take(3) {
                context.push_str(&format!(
                    "- {} at {}\n",
                    event.description.as_deref().unwrap_or("Unknown"),
                    event.start_time
                ));
            }
        }

        Ok(context)
    }

    /// Parse Pattern's response to update state
    async fn parse_intervention_response(&self, response: &str) -> Result<()> {
        let mut state = self.state.write().await;

        // Simple parsing for now - look for keywords
        let lower = response.to_lowercase();

        if lower.contains("break") || lower.contains("rest") {
            state.last_break_time = Some(Utc::now());
            info!("Detected break suggestion in response");
        }

        if lower.contains("water") || lower.contains("hydrat") {
            state.last_water_time = Some(Utc::now());
            info!("Detected hydration reminder in response");
        }

        if lower.contains("stretch") || lower.contains("move") || lower.contains("walk") {
            state.last_movement_time = Some(Utc::now());
            info!("Detected movement suggestion in response");
        }

        Ok(())
    }

    /// Manual check trigger (e.g., from user request)
    pub async fn trigger_manual_check(&self) -> Result<()> {
        info!("Manual sleeptime check requested");

        self.trigger_tier2_intervention(vec![InterventionReason::UserRequest])
            .await
    }

    /// Update task information
    pub async fn update_current_task(&self, task: String) -> Result<()> {
        let mut state = self.state.write().await;

        // Detect task switch
        if let Some(current) = &state.current_task {
            if current != &task {
                let current_task = current.clone(); // Clone before dropping state
                drop(state); // Release lock before triggering

                self.trigger_tier2_intervention(vec![InterventionReason::TaskSwitch {
                    from: current_task,
                    to: task.clone(),
                }])
                .await?;

                state = self.state.write().await;
            }
        }

        state.current_task = Some(task);
        state.current_task_start = Some(Utc::now());

        Ok(())
    }

    /// Record that user took a break
    pub async fn record_break(&self) -> Result<()> {
        let mut state = self.state.write().await;
        let now = Utc::now();

        state.last_break_time = Some(now);
        state.last_movement_time = Some(now); // Breaks usually involve movement
        state.current_task_start = Some(now); // Reset task timer

        Ok(())
    }

    /// Record physical activity
    pub async fn record_movement(&self) -> Result<()> {
        self.state.write().await.last_movement_time = Some(Utc::now());
        Ok(())
    }

    /// Record hydration
    pub async fn record_water(&self) -> Result<()> {
        self.state.write().await.last_water_time = Some(Utc::now());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intervention_reasons() {
        let reason = InterventionReason::Hyperfocus {
            duration_minutes: 120,
        };
        assert_eq!(
            serde_json::to_string(&reason).unwrap(),
            r#"{"Hyperfocus":{"duration_minutes":120}}"#
        );
    }

    #[test]
    fn test_monitor_state_default() {
        let state = MonitorState::default();
        assert_eq!(state.checks_since_intervention, 0);
        assert!(state.last_break_time.is_some());
    }
}
