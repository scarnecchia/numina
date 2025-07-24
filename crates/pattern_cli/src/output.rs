use owo_colors::OwoColorize;
use termimad::MadSkin;

/// Standard output formatting for the CLI
pub struct Output {
    skin: MadSkin,
}

impl Output {
    pub fn new() -> Self {
        let mut skin = MadSkin::default();
        // Keep it simple for copy-paste friendliness
        skin.set_headers_fg(termimad::ansi(6)); // cyan
        skin.bold.set_fg(termimad::ansi(15)); // bright white

        // Make inline code stand out with color but no background
        skin.inline_code.set_fg(termimad::ansi(11)); // bright yellow
        skin.inline_code
            .set_bg(termimad::crossterm::style::Color::Black);

        // Fix code blocks to not have background
        skin.code_block
            .set_bg(termimad::crossterm::style::Color::Black);
        skin.code_block.set_fg(termimad::ansi(15)); // bright white text

        Self { skin }
    }

    /// Print an agent message with markdown formatting
    pub fn agent_message(&self, agent_name: &str, content: &str) {
        // Clear visual separation without box drawing chars
        println!();
        println!("{} {}", agent_name.bright_cyan().bold(), "says:".dimmed());
        println!();

        // Don't indent - let termimad handle the formatting
        // This avoids the issue where indented lists become code blocks
        self.skin.print_text(content);
        println!();
    }

    /// Print a system/status message (indented)
    pub fn status(&self, message: &str) {
        println!("  {}", message.dimmed());
    }

    /// Print an info message (indented)
    pub fn info(&self, label: &str, value: &str) {
        println!("  {} {}", label.bright_blue(), value);
    }

    /// Print a success message (indented)
    pub fn success(&self, message: &str) {
        println!("  {} {}", "✓".bright_green(), message);
    }

    /// Print an error message (indented)
    pub fn error(&self, message: &str) {
        println!("  {} {}", "✗".bright_red(), message);
    }

    /// Print a warning message (indented)
    pub fn warning(&self, message: &str) {
        println!("  {} {}", "⚠".yellow(), message);
    }

    /// Print a section header
    pub fn section(&self, title: &str) {
        println!();
        println!("{}", title.bright_cyan().bold());
        println!("{}", "─".repeat(40).dimmed());
    }

    /// Print a list item (already indented)
    pub fn list_item(&self, item: &str) {
        println!("  • {}", item);
    }

    /// Print a tool call
    pub fn tool_call(&self, tool_name: &str, args: &str) {
        println!(
            "{} Using tool: {}",
            "🔧".bright_blue(),
            tool_name.bright_yellow()
        );
        if !args.is_empty() {
            println!("   Args: {}", args.dimmed());
        }
    }

    /// Print a tool result
    pub fn tool_result(&self, result: &str) {
        println!("{} Tool result: {}", "→".bright_green(), result.dimmed());
    }
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}

/// Format agent state for display
pub fn format_agent_state(state: &pattern_core::agent::AgentState) -> String {
    use pattern_core::agent::AgentState;

    match state {
        AgentState::Ready => "Ready".bright_green().to_string(),
        AgentState::Processing => "Processing".bright_yellow().to_string(),
        AgentState::Cooldown { until } => format!("Cooldown until {}", until.format("%H:%M:%S"))
            .yellow()
            .to_string(),
        AgentState::Suspended => "Suspended".bright_red().to_string(),
        AgentState::Error => "Error".red().bold().to_string(),
    }
}

/// Format a timestamp as relative time
pub fn format_relative_time(time: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(time);

    if duration.num_seconds() < 60 {
        format!("{} seconds ago", duration.num_seconds())
            .dimmed()
            .to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
            .dimmed()
            .to_string()
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
            .dimmed()
            .to_string()
    } else if duration.num_days() < 30 {
        format!("{} days ago", duration.num_days())
            .dimmed()
            .to_string()
    } else {
        time.format("%Y-%m-%d").to_string().dimmed().to_string()
    }
}
