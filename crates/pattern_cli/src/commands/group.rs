use chrono::Utc;
use miette::{IntoDiagnostic, Result};
use pattern_core::{
    config::PatternConfig,
    coordination::{
        groups::{AgentGroup, GroupMembership},
        types::{CoordinationPattern, GroupMemberRole, GroupState},
    },
    db::{client::DB, ops},
    id::{AgentId, GroupId, RelationId},
};

use crate::output::Output;

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
