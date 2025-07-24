use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::config::{self, PatternConfig};
use std::path::PathBuf;

use crate::output::Output;

/// Show current configuration
pub async fn show(config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.section("Current Configuration");
    println!();

    // Display the current config in TOML format
    let toml_str = toml::to_string_pretty(config).into_diagnostic()?;
    println!("{}", toml_str);

    Ok(())
}

/// Save current configuration to file
pub async fn save(config: &PatternConfig, path: &PathBuf) -> Result<()> {
    let output = Output::new();

    output.info(
        "💾",
        &format!("Saving configuration to: {}", path.display()),
    );

    // Save the current config
    config::save_config(config, path).await?;

    output.success("Configuration saved successfully!");
    println!();
    println!("To use this configuration, run:");
    println!(
        "  {} --config {}",
        "pattern-cli".bright_green(),
        path.display()
    );

    Ok(())
}
