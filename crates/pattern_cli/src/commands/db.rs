use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    config::PatternConfig,
    db::{DatabaseConfig, client::DB},
};

use crate::output::Output;

/// Show database statistics
pub async fn stats(config: &PatternConfig, output: &Output) -> Result<()> {
    output.section("Database Statistics");
    output.print("");

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
    output.print("");

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
        output.print("");
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
pub async fn query(sql: &str, output: &Output) -> Result<()> {

    // Execute the query
    let mut response = DB.query(sql).await.into_diagnostic()?;

    // Process each statement result
    let num_statements = response.num_statements();
    
    if num_statements == 0 {
        output.status("No results");
        return Ok(());
    }
    
    for statement_idx in 0..num_statements {
        // Print separator like SurrealDB CLI
        output.print(&format!("\n{} Query {} {}", 
            "-".repeat(8).dimmed(), 
            (statement_idx + 1).to_string().bright_cyan(),
            "-".repeat(8).dimmed()
        ));
        output.print("");
        
        match response.take::<surrealdb::Value>(statement_idx) {
            Ok(value) => {
                // Convert to JSON
                let wrapped_json = serde_json::to_value(&value).into_diagnostic()?;
                let json_value = unwrap_surrealdb_value(wrapped_json);
                
                // Flatten nested arrays for cleaner output
                let final_value = match json_value {
                    serde_json::Value::Array(rows) => {
                        let mut flattened = Vec::new();
                        for item in rows {
                            match item {
                                serde_json::Value::Array(inner) => {
                                    flattened.extend(inner);
                                }
                                other => {
                                    flattened.push(other);
                                }
                            }
                        }
                        serde_json::Value::Array(flattened)
                    }
                    other => other,
                };
                
                // Pretty print with custom formatting
                let pretty = serde_json::to_string_pretty(&final_value).into_diagnostic()?;
                output.print(&pretty);
            }
            Err(_) => {
                output.status("Query produced no output");
            }
        }
    }
    
    output.print("");
    Ok(())
}

/// Recursively unwrap surrealdb's type descriptors from JSON
fn unwrap_surrealdb_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            // Check if this is a wrapped type (single key that's a type name)
            if map.len() == 1 {
                let key = map.keys().next().unwrap().clone();
                match key.as_str() {
                    // Simple unwrappers - just take the inner value
                    "Array" | "Object" | "Strand" | "Number" | "Bool" | "Datetime" 
                    | "Uuid" | "Bytes" | "Duration" => {
                        let inner = map.remove(&key).unwrap();
                        return unwrap_surrealdb_value(inner);
                    }
                    // Int needs special handling
                    "Int" | "Float" => {
                        if let Some(n) = map.remove(&key) {
                            return n;
                        }
                    }
                    // Thing (record ID) - convert to string representation
                    "Thing" => {
                        if let Some(thing_val) = map.remove(&key) {
                            // Thing has { tb: "table", id: "id" } structure
                            if let serde_json::Value::Object(mut thing_map) = thing_val {
                                if let (Some(tb), Some(id)) = (
                                    thing_map.remove("tb").and_then(|v| v.as_str().map(|s| s.to_string())),
                                    thing_map.remove("id")
                                ) {
                                    // Format as table:id
                                    let id_str = match id {
                                        serde_json::Value::String(s) => s,
                                        other => other.to_string(),
                                    };
                                    return serde_json::Value::String(format!("{}:{}", tb, id_str));
                                } else {
                                    // If we can't extract tb/id, return as object
                                    return serde_json::Value::Object(thing_map);
                                }
                            } else {
                                // Not an object, return as-is
                                return thing_val;
                            }
                        }
                    }
                    // None becomes null
                    "None" => {
                        return serde_json::Value::Null;
                    }
                    // Geometry types - just unwrap for now
                    "Geometry" | "Point" | "Line" | "Polygon" | "MultiPoint" 
                    | "MultiLine" | "MultiPolygon" | "Collection" => {
                        let inner = map.remove(&key).unwrap();
                        return unwrap_surrealdb_value(inner);
                    }
                    _ => {}
                }
            }
            
            // Not a wrapper, recursively process all values
            let mut result = serde_json::Map::new();
            for (k, v) in map {
                result.insert(k, unwrap_surrealdb_value(v));
            }
            serde_json::Value::Object(result)
        }
        serde_json::Value::Array(arr) => {
            // Recursively process array elements
            serde_json::Value::Array(arr.into_iter().map(unwrap_surrealdb_value).collect())
        }
        // Primitives pass through
        other => other,
    }
}
