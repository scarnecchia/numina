//! Export and import commands for agents, groups, and constellations

use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use std::path::PathBuf;
use tokio::fs::File;

use pattern_core::{
    UserId,
    agent::AgentRecord,
    config::PatternConfig,
    db::{client::DB, ops},
    export::{AgentExporter, AgentImporter, ExportOptions, ImportOptions},
};

use crate::output::Output;

/// Export an agent to a CAR file
pub async fn export_agent(
    name: &str,
    output: Option<PathBuf>,
    config: &PatternConfig,
) -> Result<()> {
    let output_handler = Output::new();

    // Get user from config
    let user_id = &config.user.id;

    // Find the agent
    let agent = get_agent_by_name(&DB, user_id, name)
        .await?
        .ok_or_else(|| miette::miette!("Agent '{}' not found", name))?;

    // Determine output filename
    let output_path =
        output.unwrap_or_else(|| PathBuf::from(format!("{}.car", name.replace(' ', "-"))));

    output_handler.info(
        "Exporting",
        &format!(
            "agent '{}' to {}",
            name.bright_cyan(),
            output_path.display()
        ),
    );

    // Create exporter
    let exporter = AgentExporter::new(DB.clone());

    let options = ExportOptions {
        include_messages: true,
        chunk_size: 1000,
        messages_since: None,
        ..Default::default()
    };

    let file = File::create(&output_path).await.into_diagnostic()?;

    let manifest = exporter
        .export_to_car(agent.id, file, options)
        .await
        .into_diagnostic()?;

    output_handler.success(&format!("Export complete!"));
    output_handler.kv("Manifest CID", &manifest.data_cid.to_string());
    output_handler.kv("Messages", &manifest.stats.message_count.to_string());
    output_handler.kv("Memories", &manifest.stats.memory_count.to_string());
    output_handler.kv("Total blocks", &manifest.stats.total_blocks.to_string());
    output_handler.kv(
        "Size",
        &format!("{} bytes", manifest.stats.uncompressed_size),
    );

    Ok(())
}

/// Export a group to a CAR file
pub async fn export_group(
    name: &str,
    output: Option<PathBuf>,
    config: &PatternConfig,
) -> Result<()> {
    let output_handler = Output::new();

    // Get user from config
    let user_id = &config.user.id;

    // Find the group
    let group = ops::get_group_by_name(&DB, user_id, name)
        .await
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("Group '{}' not found", name))?;

    // Determine output filename
    let output_path =
        output.unwrap_or_else(|| PathBuf::from(format!("{}.car", name.replace(' ', "-"))));

    output_handler.info(
        "Exporting",
        &format!(
            "group '{}' to {}",
            name.bright_cyan(),
            output_path.display()
        ),
    );

    // Create exporter
    let exporter = AgentExporter::new(DB.clone());

    let options = ExportOptions {
        include_messages: true,
        chunk_size: 1000,
        messages_since: None,
        ..Default::default()
    };

    let file = File::create(&output_path).await.into_diagnostic()?;

    let manifest = exporter
        .export_group_to_car(group.id, file, options)
        .await
        .into_diagnostic()?;

    output_handler.success(&format!("Export complete!"));
    output_handler.kv("Manifest CID", &manifest.data_cid.to_string());
    output_handler.kv("Members", &manifest.stats.message_count.to_string());

    Ok(())
}

/// Export a constellation to a CAR file
pub async fn export_constellation(output: Option<PathBuf>, config: &PatternConfig) -> Result<()> {
    let output_handler = Output::new();

    // Get user from config
    let user_id = &config.user.id;

    // Get user's constellation
    let constellation = ops::get_or_create_constellation(&DB, user_id)
        .await
        .into_diagnostic()?;

    // Determine output filename
    let output_path = output.unwrap_or_else(|| PathBuf::from("constellation.car"));

    output_handler.info(
        "Exporting",
        &format!("constellation to {}", output_path.display()),
    );

    // Create exporter
    let exporter = AgentExporter::new(DB.clone());

    let options = ExportOptions {
        include_messages: true,
        chunk_size: 1000,
        messages_since: None,
        ..Default::default()
    };

    let file = File::create(&output_path).await.into_diagnostic()?;

    let manifest = exporter
        .export_constellation_to_car(constellation.id, file, options)
        .await
        .into_diagnostic()?;

    output_handler.success(&format!("Export complete!"));
    output_handler.kv("Manifest CID", &manifest.data_cid.to_string());
    output_handler.kv("Agents", &manifest.stats.memory_count.to_string());
    output_handler.kv(
        "Groups",
        &(manifest.stats.total_blocks
            - manifest.stats.memory_count
            - manifest.stats.message_count
            - manifest.stats.chunk_count
            - 1)
        .to_string(),
    );

    Ok(())
}

/// Import from a CAR file
pub async fn import(
    file_path: PathBuf,
    rename_to: Option<String>,
    preserve_ids: bool,
    config: &PatternConfig,
) -> Result<()> {
    let output_handler = Output::new();

    output_handler.info("Importing", &format!("from {}", file_path.display()));

    // Get user from config
    let user_id = &config.user.id;

    let importer = AgentImporter::new(DB.clone());

    let options = ImportOptions {
        rename_to,
        merge_existing: false,
        preserve_ids,
        owner_id: user_id.clone(),
        preserve_timestamps: true,
        import_messages: true,
        import_memories: true,
    };

    let file = File::open(&file_path).await.into_diagnostic()?;

    // Detect the type of export
    let (export_type, buffer) = AgentImporter::<surrealdb::engine::any::Any>::detect_type(file)
        .await
        .into_diagnostic()?;

    // Create a cursor from the buffer for importing
    let cursor = std::io::Cursor::new(buffer);

    // Import based on detected type
    let result = match export_type {
        pattern_core::export::ExportType::Agent => importer
            .import_agent_from_car(cursor, options)
            .await
            .into_diagnostic()?,
        pattern_core::export::ExportType::Group => importer
            .import_group_from_car(cursor, options)
            .await
            .into_diagnostic()?,
        pattern_core::export::ExportType::Constellation => importer
            .import_constellation_from_car(cursor, options)
            .await
            .into_diagnostic()?,
    };

    output_handler.success(&format!("Import complete!"));
    output_handler.kv("Agents imported", &result.agents_imported.to_string());
    output_handler.kv("Messages imported", &result.messages_imported.to_string());
    output_handler.kv("Memories imported", &result.memories_imported.to_string());

    if result.groups_imported > 0 {
        output_handler.kv("Groups imported", &result.groups_imported.to_string());
    }

    if !result.agent_id_map.is_empty() {
        println!();
        output_handler.info("Agent ID mappings", "");
        for (old_id, new_id) in &result.agent_id_map {
            output_handler.kv(&format!("  {}", old_id), &new_id.to_string());
        }
    }

    Ok(())
}

// Helper function to get agent by name
pub async fn get_agent_by_name<C: surrealdb::Connection>(
    db: &surrealdb::Surreal<C>,
    user_id: &UserId,
    name: &str,
) -> Result<Option<AgentRecord>> {
    let query = r#"
        SELECT * FROM agent
        WHERE owner_id = $user_id
        AND name = $name
        LIMIT 1
    "#;

    let mut result = db
        .query(query)
        .bind(("user_id", surrealdb::RecordId::from(user_id.clone())))
        .bind(("name", name.to_string()))
        .await
        .into_diagnostic()?;

    use pattern_core::db::DbEntity;

    let db_agents: Vec<<AgentRecord as DbEntity>::DbModel> = result.take(0).into_diagnostic()?;

    if let Some(db_model) = db_agents.into_iter().next() {
        Ok(Some(
            AgentRecord::from_db_model(db_model).into_diagnostic()?,
        ))
    } else {
        Ok(None)
    }
}
