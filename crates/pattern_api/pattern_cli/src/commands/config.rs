use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::config::{self, PatternConfig};
use std::path::PathBuf;

use crate::output::Output;

/// Show current configuration
pub async fn show(config: &PatternConfig, output: &Output) -> Result<()> {
    output.section("Current Configuration");
    output.print("");

    // Display the current config in TOML format
    let toml_str = toml::to_string_pretty(config).into_diagnostic()?;
    // Print the TOML directly without indentation since it's already formatted
    for line in toml_str.lines() {
        output.print(line);
    }

    Ok(())
}

/// Save current configuration to file
pub async fn save(config: &PatternConfig, path: &PathBuf, output: &Output) -> Result<()> {
    output.info(
        "💾",
        &format!("Saving configuration to: {}", path.display()),
    );

    // Save the current config
    config::save_config(config, path).await?;

    output.success("Configuration saved successfully!");
    output.print("");
    output.status("To use this configuration, run:");
    output.status(&format!(
        "{} --config {}",
        "pattern-cli".bright_green(),
        path.display()
    ));

    Ok(())
}
