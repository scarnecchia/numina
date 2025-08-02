use chrono::Utc;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    config::{
        AgentConfig, GroupConfig, GroupMemberConfig, GroupMemberRoleConfig, GroupPatternConfig,
        MemoryBlockConfig, ModelConfig, PatternConfig, UserConfig,
    },
    coordination::{
        groups::{AgentGroup, GroupMembership},
        types::{CoordinationPattern, GroupMemberRole, GroupState},
    },
    db::{DatabaseConfig, client::DB, ops, ops::get_group_by_name},
    id::{AgentId, GroupId, RelationId},
};
use std::{collections::HashMap, path::Path};

use crate::{agent_ops, output::Output};

/// List all groups for the current user
pub async fn list(config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.section("Agent Groups");

    // Get groups for the user
    let groups = ops::list_groups_for_user(&DB, &config.user.id).await?;

    if groups.is_empty() {
        output.info("No groups found", "");
        output.info(
            "Hint:",
            "Create a group with: pattern-cli group create <name> --description <desc>",
        );
    } else {
        for group in groups {
            output.info("Group:", &group.name);
            output.kv("  ID", &group.id.to_string());
            output.kv("  Description", &group.description);
            output.kv("  Pattern", &format_pattern(&group.coordination_pattern));
            output.kv("  Members", &format!("{} agents", group.members.len()));
            output.kv("  Active", if group.is_active { "yes" } else { "no" });
            println!();
        }
    }

    Ok(())
}

/// Create a new group
pub async fn create(
    name: &str,
    description: &str,
    pattern: &str,
    config: &PatternConfig,
) -> Result<()> {
    let output = Output::new();

    output.section(&format!("Creating group '{}'", name));

    // Parse the coordination pattern
    let coordination_pattern = match pattern {
        "round_robin" => CoordinationPattern::RoundRobin {
            current_index: 0,
            skip_unavailable: true,
        },
        "supervisor" => {
            output.error("Supervisor pattern requires a leader to be specified");
            output.info("Hint:", "Use --pattern supervisor --leader <agent_name>");
            return Ok(());
        }
        "dynamic" => CoordinationPattern::Dynamic {
            selector_name: "random".to_string(),
            selector_config: Default::default(),
        },
        "pipeline" => {
            output.error("Pipeline pattern requires stages to be specified");
            output.info(
                "Hint:",
                "Use --pattern pipeline --stages <stage1,stage2,...>",
            );
            return Ok(());
        }
        _ => {
            output.error(&format!("Unknown pattern: {}", pattern));
            output.info(
                "Hint:",
                "Available patterns: round_robin, supervisor, dynamic, pipeline",
            );
            return Ok(());
        }
    };

    // Create the group
    let group = AgentGroup {
        id: GroupId::generate(),
        name: name.to_string(),
        description: description.to_string(),
        coordination_pattern,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        is_active: true,
        state: GroupState::RoundRobin {
            current_index: 0,
            last_rotation: Utc::now(),
        },
        members: vec![],
    };

    let created = ops::create_group_for_user(&DB, &config.user.id, &group).await?;

    output.success(&format!("Created group '{}'", created.name));
    output.kv("ID", &created.id.to_string());
    output.kv("Pattern", &format_pattern(&created.coordination_pattern));

    output.info(
        "Next:",
        &format!(
            "Add members with: pattern-cli group add-member {} <agent_name>",
            name
        ),
    );

    Ok(())
}

/// Add an agent to a group
pub async fn add_member(
    group_name: &str,
    agent_name: &str,
    role: &str,
    capabilities: Option<&str>,
    config: &PatternConfig,
) -> Result<()> {
    let output = Output::new();

    output.section(&format!(
        "Adding '{}' to group '{}'",
        agent_name, group_name
    ));

    // Find the group
    let group = ops::get_group_by_name(&DB, &config.user.id, group_name).await?;
    let group = match group {
        Some(g) => g,
        None => {
            output.error(&format!("Group '{}' not found", group_name));
            return Ok(());
        }
    };

    // Find the agent by name
    let query = "SELECT id FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    let agent_ids: Vec<surrealdb::RecordId> = response.take("id").into_diagnostic()?;

    let agent_id = match agent_ids.first() {
        Some(id_value) => AgentId::from_record(id_value.clone()),
        None => {
            output.error(&format!("Agent '{}' not found", agent_name));
            output.info(
                "Hint:",
                "Create the agent first with: pattern-cli agent create <name>",
            );
            return Ok(());
        }
    };

    // Parse role
    let member_role = match role {
        "regular" => GroupMemberRole::Regular,
        "supervisor" => GroupMemberRole::Supervisor,
        role if role.starts_with("specialist:") => {
            let domain = role.strip_prefix("specialist:").unwrap();
            GroupMemberRole::Specialist {
                domain: domain.to_string(),
            }
        }
        _ => {
            output.error(&format!("Unknown role: {}", role));
            output.info(
                "Hint:",
                "Available roles: regular, supervisor, specialist:<domain>",
            );
            return Ok(());
        }
    };

    // Parse capabilities
    let caps = capabilities
        .map(|c| c.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    // Create membership
    let membership = GroupMembership {
        id: RelationId::nil(),
        in_id: agent_id,
        out_id: group.id,
        joined_at: Utc::now(),
        role: member_role,
        is_active: true,
        capabilities: caps,
    };

    // Add to group
    ops::add_agent_to_group(&DB, &membership).await?;

    output.success(&format!(
        "Added '{}' to group '{}' as {}",
        agent_name, group_name, role
    ));

    Ok(())
}

/// Show group status and members
pub async fn status(name: &str, config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.section(&format!("Group: {}", name));

    // Find the group
    let group = ops::get_group_by_name(&DB, &config.user.id, name).await?;
    let group = match group {
        Some(g) => g,
        None => {
            output.error(&format!("Group '{}' not found", name));
            return Ok(());
        }
    };

    // Basic info
    output.kv("ID", &group.id.to_string());
    output.kv("Description", &group.description);
    output.kv("Pattern", &format_pattern(&group.coordination_pattern));
    output.kv("Active", if group.is_active { "yes" } else { "no" });
    output.kv(
        "Created",
        &group.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
    );

    // Members
    if group.members.is_empty() {
        output.info("No members", "");
    } else {
        output.section("Members");
        for (agent, membership) in &group.members {
            output.info("  Agent:", &agent.name);
            output.kv("  Role", &format_role(&membership.role));
            if !membership.capabilities.is_empty() {
                output.kv("  Capabilities", &membership.capabilities.join(", "));
            }
            output.kv("  Active", if membership.is_active { "yes" } else { "no" });
        }
    }

    // State info
    output.section("Current State");
    match &group.state {
        GroupState::RoundRobin {
            current_index,
            last_rotation,
        } => {
            output.kv("Type", "Round Robin");
            output.kv("Current Index", &current_index.to_string());
            output.kv(
                "Last Rotation",
                &last_rotation.format("%Y-%m-%d %H:%M:%S").to_string(),
            );
        }
        _ => {
            output.kv("Type", &format!("{:?}", group.state));
        }
    }

    Ok(())
}

// Helper functions

fn format_pattern(pattern: &CoordinationPattern) -> String {
    match pattern {
        CoordinationPattern::Supervisor { leader_id, .. } => {
            format!("Supervisor (leader: {})", leader_id)
        }
        CoordinationPattern::RoundRobin {
            skip_unavailable, ..
        } => {
            format!("Round Robin (skip inactive: {})", skip_unavailable)
        }
        CoordinationPattern::Voting { quorum, .. } => format!("Voting (quorum: {})", quorum),
        CoordinationPattern::Pipeline {
            stages,
            parallel_stages,
        } => {
            format!(
                "Pipeline ({} stages, parallel: {})",
                stages.len(),
                parallel_stages
            )
        }
        CoordinationPattern::Dynamic { selector_name, .. } => {
            format!("Dynamic (selector: {})", selector_name)
        }
        CoordinationPattern::Sleeptime { check_interval, .. } => {
            format!("Sleeptime (check every: {:?})", check_interval)
        }
    }
}

fn format_role(role: &GroupMemberRole) -> &str {
    match role {
        GroupMemberRole::Regular => "Regular",
        GroupMemberRole::Supervisor => "Supervisor",
        GroupMemberRole::Specialist { .. } => "Specialist",
    }
}

/// Initialize groups from configuration
pub async fn initialize_from_config(
    config: &PatternConfig,
    heartbeat_sender: pattern_core::context::heartbeat::HeartbeatSender,
) -> Result<()> {
    let output = Output::new();

    if config.groups.is_empty() {
        return Ok(());
    }

    output.section("Initializing Groups from Configuration");

    for group_config in &config.groups {
        output.status(&format!("Processing group: {}", group_config.name));

        // Check if group already exists
        let existing = ops::get_group_by_name(&DB, &config.user.id, &group_config.name).await?;

        let created_group = if let Some(existing_group) = existing {
            output.info("Group already exists", &group_config.name);
            output.status("Syncing group members from configuration...");
            existing_group
        } else {
            // Convert pattern from config to coordination pattern
            let coordination_pattern = convert_pattern_config(&group_config.pattern)?;

            // Create the group
            let group = AgentGroup {
                id: group_config.id.clone().unwrap_or_else(GroupId::generate),
                name: group_config.name.clone(),
                description: group_config.description.clone(),
                coordination_pattern,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                is_active: true,
                state: GroupState::RoundRobin {
                    current_index: 0,
                    last_rotation: Utc::now(),
                },
                members: vec![],
            };

            // Create group in database
            let created = ops::create_group_for_user(&DB, &config.user.id, &group).await?;
            output.success(&format!("Created group: {}", created.name));
            created
        };

        // Get existing member names to avoid duplicates
        let existing_member_names: std::collections::HashSet<String> = created_group
            .members
            .iter()
            .map(|(agent, _)| agent.name.clone())
            .collect();

        // Initialize members
        for member_config in &group_config.members {
            // Skip if member already exists
            if existing_member_names.contains(&member_config.name) {
                output.info(
                    &format!("  Member already exists: {}", member_config.name),
                    "",
                );
                continue;
            }

            output.status(&format!("  Adding member: {}", member_config.name));

            // Load or create agent from member config
            let agent = agent_ops::load_or_create_agent_from_member(
                member_config,
                &config.user.id,
                None, // model_name
                true, // enable_tools
                heartbeat_sender.clone(),
                Some(config),
            )
            .await?;

            // Convert role
            let role = convert_role_config(&member_config.role);

            // Create membership
            let membership = GroupMembership {
                id: RelationId::nil(),
                in_id: agent.id().clone(),
                out_id: created_group.id.clone(),
                joined_at: Utc::now(),
                role,
                is_active: true,
                capabilities: member_config.capabilities.clone(),
            };

            // Add to group
            ops::add_agent_to_group(&DB, &membership).await?;
            output.success(&format!(
                "  Added member: {} ({})",
                member_config.name,
                agent.id()
            ));
        }
    }

    output.success("Group initialization complete");
    Ok(())
}

fn convert_pattern_config(pattern: &GroupPatternConfig) -> Result<CoordinationPattern> {
    use pattern_core::coordination::types::{
        DelegationRules, DelegationStrategy, FallbackBehavior, PipelineStage, StageFailureAction,
    };

    Ok(match pattern {
        GroupPatternConfig::Supervisor { leader: _ } => {
            // For now, using the leader name as ID - in production should look up actual agent ID
            CoordinationPattern::Supervisor {
                leader_id: AgentId::generate(), // TODO: Look up actual agent ID by name
                delegation_rules: DelegationRules {
                    max_delegations_per_agent: None,
                    delegation_strategy: DelegationStrategy::RoundRobin,
                    fallback_behavior: FallbackBehavior::HandleSelf,
                },
            }
        }
        GroupPatternConfig::RoundRobin { skip_unavailable } => CoordinationPattern::RoundRobin {
            current_index: 0,
            skip_unavailable: *skip_unavailable,
        },
        GroupPatternConfig::Pipeline { stages } => {
            // Convert stage names to PipelineStage structs
            let pipeline_stages = stages
                .iter()
                .map(|stage_name| {
                    PipelineStage {
                        name: stage_name.clone(),
                        agent_ids: vec![AgentId::generate()], // TODO: Look up actual IDs by stage name
                        timeout: std::time::Duration::from_secs(300), // 5 minute default
                        on_failure: StageFailureAction::Skip,
                    }
                })
                .collect();

            CoordinationPattern::Pipeline {
                stages: pipeline_stages,
                parallel_stages: false,
            }
        }
        GroupPatternConfig::Dynamic {
            selector,
            selector_config,
        } => CoordinationPattern::Dynamic {
            selector_name: selector.clone(),
            selector_config: selector_config.clone(),
        },
        GroupPatternConfig::Sleeptime {
            interval_seconds,
            intervention_agent: _,
        } => {
            CoordinationPattern::Sleeptime {
                check_interval: std::time::Duration::from_secs(*interval_seconds),
                triggers: vec![],                           // Empty triggers for now
                intervention_agent_id: AgentId::generate(), // TODO: Look up actual ID
            }
        }
    })
}

fn convert_role_config(role: &GroupMemberRoleConfig) -> GroupMemberRole {
    match role {
        GroupMemberRoleConfig::Regular => GroupMemberRole::Regular,
        GroupMemberRoleConfig::Supervisor => GroupMemberRole::Supervisor,
        GroupMemberRoleConfig::Specialist { domain } => GroupMemberRole::Specialist {
            domain: domain.clone(),
        },
    }
}

/// Export group configuration (members and pattern only)
pub async fn export(name: &str, output_path: Option<&Path>, config: &PatternConfig) -> Result<()> {
    let output = Output::new();
    let user_id = config.user.id.clone();

    // Get the group with members already loaded
    let group = match get_group_by_name(&DB, &user_id, name).await? {
        Some(g) => g,
        None => {
            output.error(&format!("No group found with name '{}'", name));
            return Ok(());
        }
    };

    output.info("Exporting group:", &group.name.bright_cyan().to_string());

    // Members are already loaded in the group from get_group_by_name
    let members = group.members.clone();

    // Create the group config structure
    let mut group_config = GroupConfig {
        id: None, // Skip ID for export to avoid serialization issues
        name: group.name.clone(),
        description: group.description.clone(),
        pattern: convert_pattern_to_config(&group.coordination_pattern),
        members: vec![],
    };

    // Convert each member to config format
    for (member_agent, membership) in members {
        // Export each agent's configuration
        let agent_config = AgentConfig {
            id: Some(member_agent.id.clone()),
            name: member_agent.name.clone(),
            system_prompt: if member_agent.base_instructions.is_empty() {
                None
            } else {
                Some(member_agent.base_instructions.clone())
            },
            persona: None, // Will be extracted from memory blocks
            instructions: None,
            bluesky_handle: None,
            memory: HashMap::new(), // Will be populated from memory blocks
            tool_rules: Vec::new(),
            tools: Vec::new(),
            model: None,
            context: None,
        };

        // Get memory blocks for this agent
        let memories = ops::get_agent_memories(&DB, &member_agent.id).await?;

        // Convert memory blocks to config format
        let mut memory_configs = HashMap::new();
        let mut persona_content = None;

        for (memory_block, permission) in &memories {
            // Check if this is the persona block
            if memory_block.label == "persona" {
                persona_content = Some(memory_block.value.clone());
                continue;
            }

            let memory_config = MemoryBlockConfig {
                content: Some(memory_block.value.clone()),
                content_path: None,
                permission: permission.clone(),
                memory_type: memory_block.memory_type.clone(),
                description: memory_block.description.clone(),
                id: None,
                shared: false,
            };

            memory_configs.insert(memory_block.label.to_string(), memory_config);
        }

        // Create the final agent config with persona and memory
        let mut final_agent_config = agent_config;
        final_agent_config.persona = persona_content;
        final_agent_config.memory = memory_configs;

        // Create member config
        let member_config = GroupMemberConfig {
            name: member_agent.name.clone(),
            agent_id: Some(member_agent.id.clone()),
            config_path: None,
            agent_config: Some(final_agent_config),
            role: convert_role_to_config(&membership.role),
            capabilities: membership.capabilities.clone(),
        };

        group_config.members.push(member_config);
    }

    // Create a minimal PatternConfig with just the group
    let export_config = PatternConfig {
        user: UserConfig::default(),
        agent: AgentConfig {
            name: String::new(),
            id: None,
            system_prompt: None,
            persona: None,
            instructions: None,
            bluesky_handle: None,
            memory: HashMap::new(),
            tool_rules: Vec::new(),
            tools: Vec::new(),
            model: None,
            context: None,
        },
        model: ModelConfig::default(),
        database: DatabaseConfig::default(),
        bluesky: None,
        groups: vec![group_config.clone()],
    };

    // Debug: try serializing step by step
    output.status("Serializing group configuration...");

    // Serialize just the groups array
    let toml_str = match toml::to_string_pretty(&export_config.groups) {
        Ok(s) => s,
        Err(e) => {
            output.error(&format!("Serialization error: {}", e));
            // Try serializing just the group config without the full export config
            match toml::to_string_pretty(&group_config) {
                Ok(s) => format!("[[groups]]\n{}", s),
                Err(e2) => {
                    output.error(&format!("Group config serialization also failed: {}", e2));
                    return Err(miette::miette!(
                        "Failed to serialize group configuration: {}",
                        e2
                    ));
                }
            }
        }
    };

    // Determine output path
    let output_file = if let Some(path) = output_path {
        path.to_path_buf()
    } else {
        std::path::PathBuf::from(format!("{}_group.toml", group.name))
    };

    // Write to file
    tokio::fs::write(&output_file, toml_str)
        .await
        .into_diagnostic()?;

    output.success(&format!(
        "Exported group configuration to: {}",
        output_file.display().to_string().bright_green()
    ));
    output.status("Note: All member agents were exported with their full configurations");
    output.status("Message history and statistics are not included");

    Ok(())
}

fn convert_pattern_to_config(pattern: &CoordinationPattern) -> GroupPatternConfig {
    match pattern {
        CoordinationPattern::RoundRobin {
            skip_unavailable, ..
        } => GroupPatternConfig::RoundRobin {
            skip_unavailable: *skip_unavailable,
        },
        CoordinationPattern::Supervisor { .. } => {
            // For export, we can't determine the leader name from ID
            // This would need to be resolved from the group members
            GroupPatternConfig::Supervisor {
                leader: String::new(), // Default empty string
            }
        }
        CoordinationPattern::Pipeline { .. } => {
            // Similar issue - stages are IDs, not names
            GroupPatternConfig::Pipeline { stages: vec![] }
        }
        CoordinationPattern::Dynamic {
            selector_name,
            selector_config,
        } => GroupPatternConfig::Dynamic {
            selector: selector_name.clone(),
            selector_config: selector_config.clone(),
        },
        CoordinationPattern::Voting { .. } => {
            // GroupPatternConfig doesn't have a Voting variant, use Dynamic as fallback
            GroupPatternConfig::Dynamic {
                selector: "voting".to_string(),
                selector_config: Default::default(),
            }
        }
        CoordinationPattern::Sleeptime { check_interval, .. } => GroupPatternConfig::Sleeptime {
            interval_seconds: check_interval.as_secs(),
            intervention_agent: String::new(), // Can't resolve agent name from ID without lookup
        },
    }
}

fn convert_role_to_config(role: &GroupMemberRole) -> GroupMemberRoleConfig {
    match role {
        GroupMemberRole::Regular => GroupMemberRoleConfig::Regular,
        GroupMemberRole::Supervisor => GroupMemberRoleConfig::Supervisor,
        GroupMemberRole::Specialist { domain } => GroupMemberRoleConfig::Specialist {
            domain: domain.clone(),
        },
    }
}
