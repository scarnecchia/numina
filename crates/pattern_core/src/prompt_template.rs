use std::collections::HashMap;

use minijinja::Environment;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A prompt template using Jinja2 syntax
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub template: String,
    pub description: Option<String>,
}

impl PromptTemplate {
    pub fn new(name: impl Into<String>, template: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let template = template.into();

        // Validate template compiles by trying to render it
        let mut env = Environment::new();
        env.add_template("test", &template).map_err(|e| {
            crate::CoreError::InvalidToolParameters {
                tool_name: "prompt_template".to_string(),
                expected_schema: serde_json::json!({"template": "valid jinja2 template"}),
                provided_params: serde_json::json!({"template": &template}),
                validation_errors: vec![format!("Template compile error: {}", e)],
            }
        })?;

        Ok(Self {
            name,
            template,
            description: None,
        })
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn render(&self, context: &HashMap<String, serde_json::Value>) -> Result<String> {
        // Create a fresh environment for each render
        let mut env = Environment::new();
        env.add_template(&self.name, &self.template).map_err(|e| {
            crate::CoreError::tool_exec_error(
                "prompt_template",
                serde_json::json!({"name": &self.name}),
                e,
            )
        })?;

        // Convert context to minijinja Value - from_serialize returns the value directly
        let jinja_context = minijinja::value::Value::from_serialize(context);

        // Get template and render
        let tmpl = env.get_template(&self.name).map_err(|e| {
            crate::CoreError::tool_exec_error(
                "prompt_template",
                serde_json::json!({"name": &self.name}),
                e,
            )
        })?;

        let rendered = tmpl.render(jinja_context).map_err(|e| {
            crate::CoreError::tool_exec_error(
                "prompt_template",
                serde_json::json!({"context": context}),
                e,
            )
        })?;

        Ok(rendered)
    }

    /// Extract variable names from the template
    pub fn required_fields(&self) -> Vec<String> {
        extract_template_vars(&self.template)
    }
}

/// Event that can prompt an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptableEvent {
    pub source: String, // "data_source:files", "schedule:daily", "user:dm", etc
    pub template_name: String,
    pub context: HashMap<String, serde_json::Value>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Registry for reusable templates
#[derive(Debug, Default)]
pub struct TemplateRegistry {
    templates: HashMap<String, PromptTemplate>,
}

impl TemplateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, template: PromptTemplate) {
        self.templates.insert(template.name.clone(), template);
    }

    pub fn get(&self, name: &str) -> Option<&PromptTemplate> {
        self.templates.get(name)
    }

    pub fn render(
        &self,
        name: &str,
        context: &HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        self.get(name)
            .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                tool_name: "prompt_template".to_string(),
                cause: format!("Template '{}' not found", name),
                parameters: serde_json::json!({"template": name}),
            })?
            .render(context)
    }

    /// Register common default templates
    pub fn with_defaults(mut self) -> Result<Self> {
        // File change template
        self.register(
            PromptTemplate::new(
                "file_changed",
                "File {{ path }} was modified at {{ timestamp }}:\n{{ preview }}",
            )?
            .with_description("Notify when a file changes"),
        );

        // Stream item template
        self.register(
            PromptTemplate::new("stream_item", "New item from {{ source }}: {{ content }}")?
                .with_description("Generic stream item notification"),
        );

        // Bluesky post template
        self.register(
            PromptTemplate::new(
                "bluesky_post",
                "New post from @{{ handle }} on Bluesky:\n{{ text }}\n\n[{{ uri }}]",
            )?
            .with_description("Bluesky post notification"),
        );

        // Bluesky reply template
        self.register(
            PromptTemplate::new(
                "bluesky_reply",
                "@{{ handle }} replied to {{ reply_to }}:\n{{ text }}\n\n[{{ uri }}]",
            )?
            .with_description("Bluesky reply notification"),
        );

        // Bluesky mention template
        self.register(
            PromptTemplate::new(
                "bluesky_mention",
                "You were mentioned by @{{ handle }}:\n{{ text }}\n\n[{{ uri }}]",
            )?
            .with_description("Bluesky mention notification"),
        );

        // Scheduled task template
        self.register(
            PromptTemplate::new(
                "scheduled_task",
                "Scheduled task '{{ name }}' triggered at {{ time }}",
            )?
            .with_description("Scheduled task notification"),
        );

        // Data ingestion template
        self.register(
            PromptTemplate::new(
                "data_ingestion",
                "New data from {{ source_id }}: {{ item_count }} items received",
            )?
            .with_description("Generic data ingestion notification"),
        );

        Ok(self)
    }
}

/// Extract variable names from a template string
fn extract_template_vars(template: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut chars = template.chars();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            if let Some(next) = chars.next() {
                if next == '{' {
                    // Found opening {{
                    let mut var_name = String::new();
                    let mut found_close = false;

                    while let Some(ch) = chars.next() {
                        if ch == '}' {
                            if let Some(next) = chars.next() {
                                if next == '}' {
                                    found_close = true;
                                    break;
                                }
                            }
                        } else if ch != ' ' || !var_name.is_empty() {
                            var_name.push(ch);
                        }
                    }

                    if found_close && !var_name.is_empty() {
                        // Trim and extract just the variable name (before any filters)
                        let var_name = var_name.trim().split('|').next().unwrap_or("").trim();
                        if !var_name.is_empty() && !vars.contains(&var_name.to_string()) {
                            vars.push(var_name.to_string());
                        }
                    }
                }
            }
        }
    }

    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vars() {
        let template = "Hello {{ name }}, you have {{ count }} messages from {{ sender|upper }}";
        let vars = extract_template_vars(template);
        assert_eq!(vars, vec!["name", "count", "sender"]);
    }

    #[test]
    fn test_template_render() {
        let template = PromptTemplate::new("test", "Hello {{ name }}!").unwrap();
        let mut context = HashMap::new();
        context.insert("name".to_string(), serde_json::json!("World"));

        let result = template.render(&context).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_registry() {
        let registry = TemplateRegistry::new().with_defaults().unwrap();

        let mut context = HashMap::new();
        context.insert("path".to_string(), serde_json::json!("/tmp/test.txt"));
        context.insert(
            "timestamp".to_string(),
            serde_json::json!("2024-01-01 12:00"),
        );
        context.insert(
            "preview".to_string(),
            serde_json::json!("First few lines..."),
        );

        let result = registry.render("file_changed", &context).unwrap();
        assert!(result.contains("/tmp/test.txt"));
        assert!(result.contains("2024-01-01 12:00"));
    }
}
