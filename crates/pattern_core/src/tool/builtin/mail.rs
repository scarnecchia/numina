//! Mail tool using lettre and mailgun

use async_trait::async_trait;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;
use std::env;

use crate::{CoreError, Result, context::AgentHandle, tool::AiTool};

/// Input for email operations
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MailInput {
    /// Email subject line
    pub subject: String,

    /// Email body content
    pub content: String,

    /// Optional recipient override (uses default if not provided)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,
}

/// Output from email operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MailOutput {
    /// Whether the email was sent successfully
    pub success: bool,

    /// Result message
    pub message: String,
}

/// Mail tool for sending emails via SMTP
#[derive(Debug, Clone)]
pub struct MailTool {
    #[allow(dead_code)]
    pub(crate) handle: AgentHandle,
}

impl MailTool {
    /// Create a new mail tool
    pub fn new(handle: AgentHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl AiTool for MailTool {
    type Input = MailInput;
    type Output = MailOutput;

    fn name(&self) -> &str {
        "email"
    }

    fn description(&self) -> &str {
        "Send email for longer reports or important communications. \
         Uses SMTP via Mailgun to send emails. Automatically uses \
         configured recipient unless overridden."
    }

    async fn execute(
        &self,
        params: Self::Input,
        _meta: &crate::tool::ExecutionMeta,
    ) -> Result<Self::Output> {
        send_email(params)
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some(
            "Use this tool for sending reports or content that exceeds \
             Discord's message length limits (>2000 characters), or when \
             email is specifically requested.",
        )
    }
}

fn send_email(input: MailInput) -> Result<MailOutput> {
    // Get config from env
    let smtp_username = env::var("MAILGUN_SMTP_USERNAME")
        .map_err(|e| CoreError::tool_execution_error("email", format!("MAILGUN_SMTP_USERNAME not set: {}", e)))?;
    let smtp_password = env::var("MAILGUN_SMTP_PASSWORD")
        .map_err(|e| CoreError::tool_execution_error("email", format!("MAILGUN_SMTP_PASSWORD not set: {}", e)))?;
    let from_email = env::var("MAILGUN_FROM_EMAIL")
        .map_err(|e| CoreError::tool_execution_error("email", format!("MAILGUN_FROM_EMAIL not set: {}", e)))?;
    let default_recipient = env::var("RECIPIENT_EMAIL")
        .map_err(|e| CoreError::tool_execution_error("email", format!("RECIPIENT_EMAIL not set: {}", e)))?;

    // Use provided recipient or default
    let to_email = input.recipient.clone().unwrap_or(default_recipient);

    // Build email
    let email = Message::builder()
        .from(
            from_email
                .parse()
                .map_err(|e| CoreError::tool_execution_error("email", format!("Invalid from email from MAILGUN_FROM_EMAIL env var: {}", e)))?,
        )
        .to(to_email
            .parse()
            .map_err(|e| CoreError::tool_exec_msg(
                "email",
                serde_json::json!({ "recipient": to_email }),
                format!("Invalid to email: {}", e)
            ))?)
        .subject(input.subject.clone())
        .body(input.content.clone())
        .map_err(|e| CoreError::tool_exec_msg(
            "email",
            serde_json::to_value(&input).unwrap_or(serde_json::Value::Null),
            format!("Failed to build email: {}", e)
        ))?;

    // Setup SMTP
    let creds = Credentials::new(smtp_username, smtp_password);
    let mailer = SmtpTransport::relay("smtp.mailgun.org")
        .map_err(|e| CoreError::tool_execution_error("email", format!("SMTP relay error: {}", e)))?
        .credentials(creds)
        .build();

    // Send
    match mailer.send(&email) {
        Ok(_) => Ok(MailOutput {
            success: true,
            message: format!("Email sent to {}", to_email),
        }),
        Err(e) => Err(CoreError::tool_exec_msg(
            "email",
            serde_json::to_value(&input).unwrap_or(serde_json::Value::Null),
            format!("Failed to send email: {}", e)
        )),
    }
}
