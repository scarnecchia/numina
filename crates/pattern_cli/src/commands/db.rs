use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    config::PatternConfig,
    db::{DatabaseConfig, client::DB},
};

use crate::output::Output;

/// Show database statistics
pub async fn stats(config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.section("Database Statistics");
    println!();

    // Count entities - for now, just print responses
    let agent_response = DB
        .query("SELECT count() FROM agent")
        .await
        .into_diagnostic()?;

    output.status(&format!("Agent count response: {:?}", agent_response));

    let agent_count = 0; // TODO: Parse properly
    let message_count = 0; // TODO: Parse properly
    let memory_count = 0; // TODO: Parse properly
    let tool_call_count = 0; // TODO: Parse properly

    // Entity counts
    output.section("Entity Counts");
    output.kv(
        "Agents",
        &agent_count.to_string().bright_white().to_string(),
    );
    output.kv(
        "Messages",
        &message_count.to_string().bright_white().to_string(),
    );
    output.kv(
        "Memory blocks",
        &memory_count.to_string().bright_white().to_string(),
    );
    output.kv(
        "Tool calls",
        &tool_call_count.to_string().bright_white().to_string(),
    );
    println!();

    // Most active agents
    let active_agents_query = r#"
        SELECT name, total_messages, total_tool_calls, last_active
        FROM agent
        ORDER BY total_messages DESC
        LIMIT 5
    "#;

    let mut response = DB.query(active_agents_query).await.into_diagnostic()?;

    let active_agents: Vec<serde_json::Value> = response.take(0).into_diagnostic()?;

    if !active_agents.is_empty() {
        output.section("Most Active Agents");
        for agent in active_agents {
            if let (Some(name), Some(messages), Some(tools)) = (
                agent.get("name").and_then(|v| v.as_str()),
                agent.get("total_messages").and_then(|v| v.as_u64()),
                agent.get("total_tool_calls").and_then(|v| v.as_u64()),
            ) {
                output.list_item(&format!(
                    "{} - {} messages, {} tool calls",
                    name.bright_yellow(),
                    messages.to_string().bright_white(),
                    tools.to_string().bright_white()
                ));
            }
        }
        println!();
    }

    // Database info
    output.section("Database Info");
    output.kv("Type", &"SurrealDB (embedded)".bright_white().to_string());
    output.kv(
        "File",
        &match &config.database {
            DatabaseConfig::Embedded { path, .. } => path.bright_white().to_string(),
            //DatabaseConfig::Remote { url, .. } => url.bright_white(),
            #[allow(unreachable_patterns)]
            _ => "".bright_yellow().to_string(),
        },
    );

    // Get file size if possible for embedded databases
    #[allow(irrefutable_let_patterns)]
    if let DatabaseConfig::Embedded { path, .. } = &config.database {
        if let Ok(metadata) = std::fs::metadata(path) {
            let size = metadata.len();
            let size_str = if size < 1024 {
                format!("{} bytes", size)
            } else if size < 1024 * 1024 {
                format!("{:.2} KB", size as f64 / 1024.0)
            } else {
                format!("{:.2} MB", size as f64 / (1024.0 * 1024.0))
            };
            output.kv("Size", &size_str.bright_white().to_string());
        }
    }

    Ok(())
}

/// Run a raw SQL query
pub async fn query(sql: &str) -> Result<()> {
    let output = Output::new();

    output.info("Running query:", &sql.bright_cyan().to_string());
    println!();

    // Execute the query
    let response = DB.query(sql).await.into_diagnostic()?;

    output.status(&format!("Results: {:?}", response));

    Ok(())
}
