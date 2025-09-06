//! Database-backed agent implementation

use async_trait::async_trait;
use compact_str::CompactString;
use futures::{Stream, StreamExt};
use std::sync::Arc;
use std::time::Duration;

use surrealdb::{RecordId, Surreal};
use tokio::sync::RwLock;

use crate::context::AgentHandle;
use crate::db::DbEntity;
use crate::id::RelationId;
use crate::memory::MemoryType;
use crate::message::{
    BatchType, ContentBlock, ContentPart, ImageSource, Request, Response, ToolCall, ToolResponse,
};
use crate::model::ResponseOptions;
use crate::tool::builtin::BuiltinTools;

use crate::QueuedMessage;
use crate::{
    CoreError, MemoryBlock, ModelProvider, Result, UserId,
    agent::{
        Agent, AgentMemoryRelation, AgentState, AgentType, RecoverableErrorKind,
        tool_rules::{ToolRule, ToolRuleEngine},
    },
    context::{
        AgentContext, CompressionStrategy, ContextConfig,
        heartbeat::{HeartbeatSender, check_heartbeat_request},
    },
    db::{DatabaseError, ops, schema},
    embeddings::EmbeddingProvider,
    id::AgentId,
    memory::Memory,
    message::{Message, MessageContent},
    tool::{DynamicTool, ToolRegistry},
};
use chrono::Utc;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashSet};

use crate::agent::{ResponseEvent, get_next_message_position_sync};

/// Wrapper for model providers to work with Arc<RwLock<M>>
#[derive(Debug, Clone)]
struct ModelProviderWrapper<M: ModelProvider> {
    model: Arc<RwLock<M>>,
}

#[async_trait]
impl<M: ModelProvider> ModelProvider for ModelProviderWrapper<M> {
    fn name(&self) -> &str {
        "model_wrapper"
    }

    async fn complete(
        &self,
        options: &crate::model::ResponseOptions,
        request: crate::message::Request,
    ) -> crate::Result<crate::message::Response> {
        let model = self.model.read().await;
        model.complete(options, request).await
    }

    async fn list_models(&self) -> crate::Result<Vec<crate::model::ModelInfo>> {
        let model = self.model.read().await;
        model.list_models().await
    }

    async fn supports_capability(
        &self,
        model: &str,
        capability: crate::model::ModelCapability,
    ) -> bool {
        let provider = self.model.read().await;
        provider.supports_capability(model, capability).await
    }

    async fn count_tokens(&self, model: &str, content: &str) -> crate::Result<usize> {
        let provider = self.model.read().await;
        provider.count_tokens(model, content).await
    }
}

/// A concrete agent implementation backed by the database
#[derive(Clone)]
pub struct DatabaseAgent<M, E> {
    /// The user who owns this agent
    pub user_id: UserId,
    /// The agent's context (includes all state)
    pub context: Arc<RwLock<AgentContext>>,

    /// model provider
    model: Arc<RwLock<M>>,

    /// model configuration
    pub chat_options: Arc<RwLock<Option<ResponseOptions>>>,
    /// Database connection
    db: Surreal<surrealdb::engine::any::Any>,

    /// Embedding provider for semantic search
    embeddings: Option<Arc<E>>,

    /// Channel for sending heartbeat requests
    pub heartbeat_sender: HeartbeatSender,

    /// Tool execution rules for this agent
    tool_rules: Arc<RwLock<crate::agent::tool_rules::ToolRuleEngine>>,

    // Cached values to avoid deadlock from block_on
    cached_id: AgentId,
    cached_name: String,
    cached_agent_type: AgentType,
}

impl<M, E> DatabaseAgent<M, E>
where
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    /// Build a canonical JSON representation by sorting object keys recursively
    fn canonicalize_json(value: &JsonValue) -> JsonValue {
        match value {
            JsonValue::Object(map) => {
                let mut sorted: BTreeMap<String, JsonValue> = BTreeMap::new();
                for (k, v) in map.iter() {
                    sorted.insert(k.clone(), Self::canonicalize_json(v));
                }
                serde_json::to_value(sorted).unwrap_or(JsonValue::Object(map.clone()))
            }
            JsonValue::Array(arr) => {
                JsonValue::Array(arr.iter().map(Self::canonicalize_json).collect())
            }
            _ => value.clone(),
        }
    }

    /// Compute a deduplication key for a tool call based on name and canonicalized arguments
    fn tool_call_dedupe_key(call: &crate::message::ToolCall) -> String {
        let canonical_args = Self::canonicalize_json(&call.fn_arguments);
        let args_str = serde_json::to_string(&canonical_args).unwrap_or_else(|_| "{}".into());
        format!("{}|{}", call.fn_name, args_str)
    }
    /// Clean up temporarily loaded memory blocks that are not pinned
    async fn cleanup_temporary_memory_blocks(&self, loaded_memory_labels: &[String]) {
        if !loaded_memory_labels.is_empty() {
            tracing::debug!(
                "🧹 Cleaning up {} temporarily loaded memory blocks",
                loaded_memory_labels.len()
            );
            let handle = self.handle().await;
            for label in loaded_memory_labels {
                // Check if we should remove the block (extract data and drop the guard!)
                let should_remove = if let Some(block) = handle.memory.get_block(label) {
                    // Only remove working memory blocks that are not pinned
                    block.memory_type == crate::memory::MemoryType::Working && !block.pinned
                } else {
                    false
                }; // Guard is dropped here!

                if should_remove {
                    if let Some(mut block) = handle.memory.get_block_mut(label) {
                        block.memory_type = MemoryType::Archival;
                    }

                    tracing::debug!("🗑️ Removed temporary memory block: {}", label);
                }
            }
        }
    }

    /// Retry model completion with exponential backoff for rate limit errors
    async fn complete_with_retry(
        model: &M,
        options: &ResponseOptions,
        mut request: Request,
        max_retries: u32,
    ) -> Result<Response> {
        let mut retries = 0;
        let mut backoff_ms = 10000; // Start with 10 seconds

        loop {
            match model.complete(options, request.clone()).await {
                Ok(response) => {
                    return Ok(response);
                }
                Err(e) => {
                    // Check if this is the Gemini empty candidates error (match structured error)
                    let is_gemini_empty_candidates = match &e {
                        CoreError::ModelProviderError { cause, .. } => {
                            if let genai::Error::WebModelCall { webc_error, .. } = cause {
                                match webc_error {
                                    genai::webc::Error::JsonValueExt(
                                        value_ext::JsonValueExtError::PropertyNotFound(path),
                                    ) => path == "/candidates/0/content/parts",
                                    _ => false,
                                }
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };

                    // Check if this is a rate limit error (429/529) via structured error
                    // and compute a precise wait from headers when possible.
                    let (is_rate_limit, rate_wait_ms, rate_wait_src): (
                        bool,
                        Option<u64>,
                        Option<String>,
                    ) = match &e {
                        CoreError::ModelProviderError { cause, .. } => {
                            if let genai::Error::WebModelCall { webc_error, .. } = cause {
                                if let genai::webc::Error::ResponseFailedStatus {
                                    status,
                                    headers,
                                    ..
                                } = webc_error
                                {
                                    let code = status.as_u16();
                                    let is_rl = code == 429 || code == 529;
                                    // Helper: parse Retry-After (seconds or HTTP-date)
                                    let parse_retry_after =
                                        |hdrs: &http::HeaderMap| -> Option<(u64, &'static str)> {
                                            if let Some(raw) = hdrs
                                                .get("retry-after")
                                                .and_then(|v| v.to_str().ok())
                                            {
                                                let s = raw.trim();
                                                // Numeric seconds
                                                if let Ok(secs) = s.parse::<u64>() {
                                                    return Some((
                                                        secs * 1000
                                                            + (rand::random::<u64>() % 1000),
                                                        "retry-after",
                                                    ));
                                                }
                                                // HTTP-date (RFC 7231 IMF-fixdate). Try RFC2822, then RFC3339.
                                                if let Ok(dt) =
                                                    chrono::DateTime::parse_from_rfc2822(s)
                                                {
                                                    let now = chrono::Utc::now();
                                                    let dur_ms = dt
                                                        .with_timezone(&chrono::Utc)
                                                        .signed_duration_since(now)
                                                        .num_milliseconds();
                                                    if dur_ms > 0 {
                                                        return Some((
                                                            dur_ms as u64
                                                                + (rand::random::<u64>() % 1000),
                                                            "retry-after",
                                                        ));
                                                    }
                                                }
                                                if let Ok(dt) =
                                                    chrono::DateTime::parse_from_rfc3339(s)
                                                {
                                                    let now = chrono::Utc::now();
                                                    let dur_ms = dt
                                                        .with_timezone(&chrono::Utc)
                                                        .signed_duration_since(now)
                                                        .num_milliseconds();
                                                    if dur_ms > 0 {
                                                        return Some((
                                                            dur_ms as u64
                                                                + (rand::random::<u64>() % 1000),
                                                            "retry-after",
                                                        ));
                                                    }
                                                }
                                            }
                                            None
                                        };

                                    // Helper: parse provider-specific reset headers (OpenAI/Groq-like)
                                    let parse_provider_reset =
                                        |hdrs: &http::HeaderMap| -> Option<(u64, String)> {
                                            let keys = [
                                                // OpenAI and similar
                                                "x-ratelimit-reset-requests",
                                                "x-ratelimit-reset-tokens",
                                                "x-ratelimit-reset-input-tokens",
                                                "x-ratelimit-reset-output-tokens",
                                                "x-ratelimit-reset-images-requests",
                                                // Generic fallbacks used by some providers
                                                "x-ratelimit-reset",
                                                "ratelimit-reset",
                                            ];
                                            for k in keys.iter() {
                                                if let Some(raw) =
                                                    hdrs.get(*k).and_then(|v| v.to_str().ok())
                                                {
                                                    let s = raw.trim();
                                                    // Try duration suffixes (ms, s, m, h)
                                                    let dur_ms = if let Some(stripped) =
                                                        s.strip_suffix("ms")
                                                    {
                                                        stripped.trim().parse::<u64>().ok()
                                                    } else if let Some(stripped) =
                                                        s.strip_suffix('s')
                                                    {
                                                        stripped
                                                            .trim()
                                                            .parse::<u64>()
                                                            .ok()
                                                            .map(|v| v * 1000)
                                                    } else if let Some(stripped) =
                                                        s.strip_suffix('m')
                                                    {
                                                        stripped
                                                            .trim()
                                                            .parse::<u64>()
                                                            .ok()
                                                            .map(|v| v * 60_000)
                                                    } else if let Some(stripped) =
                                                        s.strip_suffix('h')
                                                    {
                                                        stripped
                                                            .trim()
                                                            .parse::<u64>()
                                                            .ok()
                                                            .map(|v| v * 3_600_000)
                                                    } else {
                                                        None
                                                    };
                                                    if let Some(ms) = dur_ms {
                                                        return Some((
                                                            ms + (rand::random::<u64>() % 1000),
                                                            (*k).to_string(),
                                                        ));
                                                    }

                                                    // Try numeric: treat as seconds-until-reset (common)
                                                    if let Ok(secs) = s.parse::<u64>() {
                                                        return Some((
                                                            secs * 1000
                                                                + (rand::random::<u64>() % 1000),
                                                            (*k).to_string(),
                                                        ));
                                                    }

                                                    // Try RFC3339 or RFC2822 absolute times
                                                    if let Ok(dt) =
                                                        chrono::DateTime::parse_from_rfc3339(s)
                                                    {
                                                        let now = chrono::Utc::now();
                                                        let dur_ms = dt
                                                            .with_timezone(&chrono::Utc)
                                                            .signed_duration_since(now)
                                                            .num_milliseconds();
                                                        if dur_ms > 0 {
                                                            return Some((
                                                                dur_ms as u64
                                                                    + (rand::random::<u64>()
                                                                        % 1000),
                                                                (*k).to_string(),
                                                            ));
                                                        }
                                                    }
                                                    if let Ok(dt) =
                                                        chrono::DateTime::parse_from_rfc2822(s)
                                                    {
                                                        let now = chrono::Utc::now();
                                                        let dur_ms = dt
                                                            .with_timezone(&chrono::Utc)
                                                            .signed_duration_since(now)
                                                            .num_milliseconds();
                                                        if dur_ms > 0 {
                                                            return Some((
                                                                dur_ms as u64
                                                                    + (rand::random::<u64>()
                                                                        % 1000),
                                                                (*k).to_string(),
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                            None
                                        };

                                    let ra = parse_retry_after(headers.as_ref());
                                    let prov = parse_provider_reset(headers.as_ref());
                                    let wait = match (ra, prov) {
                                        (Some((ms, src)), _) => (Some(ms), Some(src.to_string())),
                                        (None, Some((ms, src))) => (Some(ms), Some(src)),
                                        (None, None) => (None, None),
                                    };
                                    (is_rl, wait.0, wait.1)
                                } else {
                                    (false, None, None)
                                }
                            } else {
                                (false, None, None)
                            }
                        }
                        _ => (false, None, None),
                    };

                    // Augment with CoreError helpers
                    let mut is_rate_limit = is_rate_limit;
                    let mut rate_wait_ms = rate_wait_ms;
                    let mut rate_wait_src = rate_wait_src;
                    if let Some((status, _headers, _body)) = e.provider_http_parts() {
                        if status == 429 || status == 529 {
                            is_rate_limit = true;
                        }
                    }
                    if rate_wait_ms.is_none() {
                        if let Some(dur) = e.rate_limit_hint() {
                            let jitter = rand::random::<u64>() % 1000;
                            rate_wait_ms = Some(dur.as_millis() as u64 + jitter);
                            rate_wait_src = Some("rate_limit_hint".to_string());
                        }
                    }

                    let mut is_anthropic_5h = match &e {
                        CoreError::ModelProviderError { cause, .. } => {
                            if let genai::Error::WebModelCall { webc_error, .. } = cause {
                                if let genai::webc::Error::ResponseFailedStatus {
                                    headers, ..
                                } = webc_error
                                {
                                    headers
                                        .get("anthropic-ratelimit-unified-5h-reset")
                                        .is_some()
                                        || headers
                                            .get("anthropic-ratelimit-unified-5h-status")
                                            .is_some()
                                        || headers
                                            .get("anthropic-ratelimit-unified-status")
                                            .is_some()
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if !is_anthropic_5h {
                        if let Some((_status, headers, _body)) = e.provider_http_parts() {
                            for (k, _v) in headers.iter() {
                                let lk = k.to_ascii_lowercase();
                                if lk == "anthropic-ratelimit-unified-5h-reset"
                                    || lk == "anthropic-ratelimit-unified-5h-status"
                                    || lk == "anthropic-ratelimit-unified-status"
                                {
                                    is_anthropic_5h = true;
                                    break;
                                }
                            }
                        }
                    }

                    // Handle Gemini empty candidates error with prompt modification
                    if is_gemini_empty_candidates && retries < max_retries {
                        retries += 1;
                        tracing::warn!(
                            "Gemini empty candidates error (attempt {}/{}), retrying with modified prompt",
                            retries,
                            max_retries
                        );

                        // Modify the last user message slightly to work around the bug
                        let mut modified_request = request.clone();
                        if let Some(last_msg) = modified_request
                            .messages
                            .iter_mut()
                            .rev()
                            .find(|m| m.role == crate::message::ChatRole::User)
                        {
                            // Append a space or punctuation to slightly change the prompt
                            if let MessageContent::Text(ref mut text) = last_msg.content {
                                // Rotate through different modifications
                                let suffix = match retries % 3 {
                                    0 => ".",
                                    1 => " ",
                                    _ => "!",
                                };
                                text.push_str(suffix);
                                tracing::debug!("Modified prompt with suffix: {:?}", suffix);
                            }
                        }

                        // Small delay before retry
                        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

                        // Update request for next iteration
                        request = modified_request;
                        continue;
                    }

                    if is_anthropic_5h {
                        // Try to extract precise reset time from headers in the underlying error
                        let mut wait_ms: Option<u64> = None;
                        let mut wait_src: Option<&str> = None;

                        if let CoreError::ModelProviderError { cause, .. } = &e {
                            if let genai::Error::WebModelCall { webc_error, .. } = cause {
                                match webc_error {
                                    genai::webc::Error::ResponseFailedStatus {
                                        headers, ..
                                    } => {
                                        let hdrs = headers.as_ref();
                                        // Prefer Anthropic unified 5h reset headers (epoch seconds)
                                        let mut reset_epoch: Option<u64> = None;
                                        let mut reset_src: Option<&str> = None;
                                        if let Some(v) = hdrs
                                            .get("anthropic-ratelimit-unified-5h-reset")
                                            .and_then(|v| v.to_str().ok())
                                        {
                                            if let Ok(val) = v.trim().parse::<u64>() {
                                                reset_epoch = Some(val);
                                                reset_src =
                                                    Some("anthropic-ratelimit-unified-5h-reset");
                                            }
                                        }
                                        if reset_epoch.is_none() {
                                            if let Some(v) = hdrs
                                                .get("anthropic-ratelimit-unified-reset")
                                                .and_then(|v| v.to_str().ok())
                                            {
                                                if let Ok(val) = v.trim().parse::<u64>() {
                                                    reset_epoch = Some(val);
                                                    reset_src =
                                                        Some("anthropic-ratelimit-unified-reset");
                                                }
                                            }
                                        }

                                        if let Some(reset_epoch) = reset_epoch {
                                            let now_epoch = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_secs())
                                                .unwrap_or(0);
                                            if reset_epoch > now_epoch {
                                                let mut delay = (reset_epoch - now_epoch) * 1000;
                                                delay += rand::random::<u64>() % 2000; // jitter 0-2s
                                                wait_ms = Some(delay);
                                                wait_src = reset_src;
                                            }
                                        }

                                        // Fallback to Retry-After (seconds)
                                        if wait_ms.is_none() {
                                            if let Some(v) = hdrs
                                                .get("retry-after")
                                                .and_then(|v| v.to_str().ok())
                                            {
                                                if let Ok(secs) = v.trim().parse::<u64>() {
                                                    let mut delay = secs * 1000;
                                                    delay += rand::random::<u64>() % 2000;
                                                    wait_ms = Some(delay);
                                                    wait_src = Some("retry-after");
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // If no precise wait computed, fall back to a conservative long backoff
                        let delay_ms = wait_ms.unwrap_or_else(|| {
                            // Prior behavior: exponential backoff plus 10 minutes
                            backoff_ms = backoff_ms * 2 + 600_000;
                            backoff_ms + (rand::random::<u64>() % 1000)
                        });

                        retries += 1;
                        tracing::warn!(
                            "Anthropic 5h limit error (attempt {}/{}), waiting {}ms before retry (source: {})",
                            retries,
                            max_retries,
                            delay_ms,
                            wait_src.unwrap_or("backoff")
                        );

                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;

                        continue;
                    }

                    if is_rate_limit && retries < max_retries {
                        retries += 1;
                        // Prefer server-provided wait (Retry-After or provider-specific), else use exponential backoff
                        let wait_ms = rate_wait_ms.unwrap_or(backoff_ms);
                        let wait_src = rate_wait_src.as_deref().unwrap_or("backoff");

                        tracing::warn!(
                            "Rate limit error (attempt {}/{}), waiting {}ms before retry (source: {})",
                            retries,
                            max_retries,
                            wait_ms,
                            wait_src
                        );

                        tokio::time::sleep(tokio::time::Duration::from_millis(wait_ms)).await;

                        // Exponential backoff with jitter for subsequent retries
                        backoff_ms = (wait_ms * 2).min(60000); // Cap at ~60 seconds
                        backoff_ms += rand::random::<u64>() % 1000; // Add 0-1s jitter

                        continue;
                    }

                    return Err(e);
                }
            }
        }
    }

    /// Run error recovery based on the error kind
    ///
    /// This method performs cleanup and recovery actions based on the type of error
    /// encountered, making the agent more resilient to API quirks and transient issues.
    async fn run_error_recovery(
        &self,
        context: &Arc<RwLock<AgentContext>>,
        error_kind: RecoverableErrorKind,
        error_msg: &str,
    ) {
        tracing::warn!("Running error recovery for {:?}: {}", error_kind, error_msg);

        match error_kind {
            RecoverableErrorKind::AnthropicThinkingOrder => {
                // Fix message ordering for Anthropic thinking mode
                let ctx = context.write().await;
                let _history = ctx.history.write().await;

                // TODO: Implement message reordering logic
                // Could use the extracted index from error context to target specific messages
                tracing::info!("Would fix Anthropic thinking message order");
            }
            RecoverableErrorKind::GeminiEmptyContents => {
                // Remove empty message arrays for Gemini
                let ctx = context.write().await;
                let mut history = ctx.history.write().await;

                // Finalize batches to clean up empty messages
                for batch in &mut history.batches {
                    let removed = batch.finalize();
                    if !removed.is_empty() {
                        tracing::info!("Removed {} empty messages from batch", removed.len());
                    }
                }
                tracing::info!("Cleaned up for Gemini empty contents error");
            }
            RecoverableErrorKind::UnpairedToolCalls
            | RecoverableErrorKind::UnpairedToolResponses => {
                // Clean up unpaired tool calls/responses
                let ctx = context.write().await;
                let mut history = ctx.history.write().await;

                // Finalize all batches to clean up unpaired calls
                for batch in &mut history.batches {
                    let removed = batch.finalize();
                    if !removed.is_empty() {
                        tracing::info!("Removed {} unpaired messages from batch", removed.len());
                    }
                }
            }
            RecoverableErrorKind::PromptTooLong => {
                // Force compression when prompt is too long
                tracing::info!("Prompt too long, forcing compression");
                let ctx = context.read().await;

                // Force compression by temporarily setting a very low message limit
                if let Err(e) = ctx.force_compression().await {
                    tracing::error!("Failed to force compression: {:?}", e);
                } else {
                    tracing::info!("Successfully compressed context to fit token limit");
                }
            }
            RecoverableErrorKind::MessageCompressionFailed => {
                // Reset compression state
                let ctx = context.write().await;
                let mut history = ctx.history.write().await;

                // Reset compression tracking to current time
                history.last_compression = chrono::Utc::now();
                tracing::info!("Reset compression state");
            }
            RecoverableErrorKind::ContextBuildFailed => {
                // Clear and rebuild context
                let ctx = context.write().await;
                let mut history = ctx.history.write().await;

                // Finalize incomplete batches
                for batch in &mut history.batches {
                    batch.finalize();
                }

                // Batches are already finalized above
                tracing::info!("Cleaned up context for rebuild");
            }
            RecoverableErrorKind::ModelApiError | RecoverableErrorKind::Unknown => {
                // Generic cleanup
                let ctx = context.write().await;
                let mut history = ctx.history.write().await;

                // Finalize any incomplete batches
                for batch in &mut history.batches {
                    batch.finalize();
                }
                tracing::info!("Generic error cleanup completed");
            }
        }

        // TODO: Persist any changes to database
        // This would involve updating messages in the database with their corrected state

        tracing::info!("Error recovery complete");
    }

    /// Create a new database-backed agent
    /// Create a new database agent
    ///
    /// This creates a new agent instance with the provided configuration.
    /// Memory blocks and message history should be loaded separately if needed.
    pub fn new(
        agent_id: AgentId,
        user_id: UserId,
        agent_type: AgentType,
        name: String,
        system_prompt: String,
        memory: Memory,
        db: Surreal<surrealdb::engine::any::Any>,
        model: Arc<RwLock<M>>,
        tools: ToolRegistry,
        embeddings: Option<Arc<E>>,
        heartbeat_sender: HeartbeatSender,
        tool_rules: Vec<ToolRule>,
    ) -> Self {
        let context_config = if !system_prompt.is_empty() {
            ContextConfig {
                base_instructions: system_prompt,
                ..Default::default()
            }
        } else {
            ContextConfig::default()
        };
        // Build AgentContext with tools
        let mut context = AgentContext::new(
            agent_id.clone(),
            name.clone(),
            agent_type.clone(),
            memory,
            tools,
            context_config,
        );
        context.handle.state = AgentState::Ready;

        // Add model provider for compression strategies
        let model_wrapper = ModelProviderWrapper {
            model: model.clone(),
        };
        context.model_provider = Some(Arc::new(model_wrapper));

        // Wire up database connection to handle for archival memory tools
        context.handle = context.handle.with_db(db.clone());

        // Create and wire up message router
        let router = crate::context::message_router::AgentMessageRouter::new(
            agent_id.clone(),
            name.clone(),
            db.clone(),
        );

        // Add router to handle - endpoints will be registered by the consumer
        context.handle = context.handle.with_message_router(router);

        // Register built-in tools
        let builtin = BuiltinTools::default_for_agent(context.handle());
        builtin.register_all(&context.tools);

        Self {
            user_id,
            context: Arc::new(RwLock::new(context)),
            model,
            chat_options: Arc::new(RwLock::new(None)),
            db,
            embeddings,
            heartbeat_sender,
            tool_rules: Arc::new(RwLock::new(ToolRuleEngine::new(tool_rules))),
            cached_id: agent_id,
            cached_name: name,
            cached_agent_type: agent_type,
        }
    }

    /// Get tool rules from the rule engine as context-compatible format
    async fn get_context_tool_rules(&self) -> Vec<crate::context::ToolRule> {
        let rule_engine = self.tool_rules.read().await;
        let descriptions = rule_engine.to_usage_descriptions();

        // Convert to context::ToolRule format
        descriptions
            .into_iter()
            .enumerate()
            .map(|(i, description)| crate::context::ToolRule {
                tool_name: format!("rule_{}", i), // Generic name since these are mixed rules
                rule: description,
            })
            .collect()
    }

    /// Compute tools that require consent from runtime rule engine
    async fn get_consent_required_tools(&self) -> Vec<String> {
        let engine = self.tool_rules.read().await;
        engine
            .get_rules()
            .iter()
            .filter_map(|r| match &r.rule_type {
                crate::agent::tool_rules::ToolRuleType::RequiresConsent { .. } => {
                    Some(r.tool_name.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Set the constellation activity tracker for shared context
    pub fn set_constellation_tracker(
        &self,
        tracker: Arc<crate::constellation_memory::ConstellationActivityTracker>,
    ) {
        // We need to make this async to write to the context
        let context = self.context.clone();
        // Use a timeout to detect potential deadlocks
        tokio::spawn(async move {
            match tokio::time::timeout(std::time::Duration::from_secs(5), context.write()).await {
                Ok(mut ctx) => {
                    ctx.set_constellation_tracker(tracker);
                }
                Err(_) => {
                    tracing::error!(
                        "Failed to acquire context write lock for constellation tracker - potential deadlock"
                    );
                }
            }
        });
    }

    pub async fn handle(&self) -> AgentHandle {
        self.context.read().await.handle.clone()
    }

    /// Set the heartbeat sender for this agent
    pub fn set_heartbeat_sender(&mut self, sender: HeartbeatSender) {
        self.heartbeat_sender = sender;
    }

    /// Get the embedding provider for this agent
    pub fn embedding_provider(&self) -> Option<Arc<E>> {
        self.embeddings.clone()
    }

    /// Create a DatabaseAgent from a persisted AgentRecord
    ///
    /// This reconstructs the runtime agent from its stored state.
    /// The model provider, tools, and embeddings are injected at runtime.
    pub async fn from_record(
        record: crate::agent::AgentRecord,
        db: Surreal<surrealdb::engine::any::Any>,
        model: Arc<RwLock<M>>,
        tools: ToolRegistry,
        embeddings: Option<Arc<E>>,
        heartbeat_sender: HeartbeatSender,
    ) -> Result<Self> {
        tracing::debug!(
            "Creating DatabaseAgent from record: agent_id={}, name={}",
            record.id,
            record.name
        );
        tracing::debug!(
            "Record has {} messages and {} memory blocks",
            record.messages.len(),
            record.memories.len()
        );

        // Create memory with the owner
        let memory = Memory::with_owner(&record.owner_id);

        // Load memory blocks from the record's relations
        for (memory_block, relation) in &record.memories {
            tracing::debug!(
                "Loading memory block: label={}, type={:?}, permission={:?}, size={} chars",
                memory_block.label,
                memory_block.memory_type,
                relation.access_level,
                memory_block.value.len()
            );
            memory.create_block(memory_block.label.clone(), memory_block.value.clone())?;
            if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                block.id = memory_block.id.clone();
                block.description = memory_block.description.clone();
                block.metadata = memory_block.metadata.clone();
                block.embedding_model = memory_block.embedding_model.clone();
                block.created_at = memory_block.created_at;
                block.updated_at = memory_block.updated_at;
                block.is_active = memory_block.is_active;
                // CRITICAL: Copy memory type and permission from database
                block.memory_type = memory_block.memory_type.clone();
                block.permission = relation.access_level.clone();
                block.owner_id = memory_block.owner_id.clone();
            }
        }

        // Get model info from provider to determine context window
        let max_context_tokens = if let Some(model_id) = &record.model_id {
            // Try to get model info from the provider
            let model_guard = model.read().await;
            let models = model_guard.list_models().await.unwrap_or_default();

            // Find the model with matching ID
            models
                .iter()
                .find(|m| m.id == *model_id || m.name == *model_id)
                .map(|m| {
                    // Use 90% of context window to leave room for output
                    (m.context_window - m.max_output_tokens.unwrap_or_else(|| 8192)) as usize
                })
        } else {
            None
        };

        // Build the context config from the record with token limit
        let mut context_config = record.to_context_config();
        context_config.max_context_tokens = max_context_tokens.or(Some(136_000)); // Fallback to 136k

        // Load tool rules from database record (with config fallback)
        let tool_rules = if !record.tool_rules.is_empty() {
            // Use rules from database
            record
                .tool_rules
                .iter()
                .map(|config_rule| config_rule.to_tool_rule())
                .collect::<Result<Vec<_>>>()
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to convert tool rules from database: {}", e);
                    Vec::new()
                })
        } else {
            // Fallback to configuration file for backward compatibility
            match crate::config::PatternConfig::load().await {
                Ok(config) => config
                    .get_agent_tool_rules(&record.name)
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            "Failed to load tool rules for agent {}: {}",
                            record.name,
                            e
                        );
                        Vec::new()
                    }),
                Err(e) => {
                    tracing::warn!("Failed to load configuration for tool rules: {}", e);
                    Vec::new()
                }
            }
        };

        // Log the loaded configuration
        tracing::info!(
            "Loaded agent {} config: max_messages={}, compression_threshold={}, compression_strategy={:?}",
            record.name,
            record.max_messages,
            record.compression_threshold,
            record.compression_strategy
        );

        // Create the agent with the loaded configuration
        let agent = Self::new(
            record.id.clone(),
            record.owner_id.clone(),
            record.agent_type.clone(),
            record.name.clone(),
            context_config.base_instructions.clone(),
            memory,
            db.clone(),
            model,
            tools,
            embeddings,
            heartbeat_sender,
            tool_rules,
        );

        // Restore the agent state
        {
            let mut context = agent.context.write().await;
            context.handle.state = AgentState::Ready;
        }

        // Restore message history and compression state
        {
            let context = agent.context.read().await;
            let mut history = context.history.write().await;
            history.compression_strategy = record.compression_strategy.clone();
            history.archive_summary = record.message_summary.clone();

            // Load active messages from relations (already ordered by position)
            let mut loaded_messages = 0;
            for (message, relation) in &record.messages {
                if relation.message_type == crate::message::MessageRelationType::Active {
                    let content_preview = match &message.content {
                        crate::message::MessageContent::Text(text) => {
                            text.chars().take(50).collect::<String>()
                        }
                        crate::message::MessageContent::ToolCalls(tool_calls) => {
                            format!("ToolCalls: {} calls", tool_calls.len())
                        }
                        crate::message::MessageContent::ToolResponses(tool_responses) => {
                            format!("ToolResponses: {} responses", tool_responses.len())
                        }
                        crate::message::MessageContent::Parts(parts) => {
                            format!("Parts: {} parts", parts.len())
                        }
                        crate::message::MessageContent::Blocks(blocks) => {
                            format!("Blocks: {} blocks", blocks.len())
                        }
                    };
                    tracing::trace!(
                        "Loading message: id={:?}, role={:?}, content_preview={:?}",
                        message.id,
                        message.role,
                        content_preview
                    );
                    let _updated_message = history.add_message(message.clone());

                    // // If sequence number was added/changed, update the database
                    // if updated_message.sequence_num != message.sequence_num {
                    //     tracing::debug!(
                    //         "Updating message {} with sequence_num: {:?} -> {:?}",
                    //         message.id,
                    //         message.sequence_num,
                    //         updated_message.sequence_num
                    //     );
                    //     // Use persist_agent_message to update both message and relation
                    //     let _ = crate::db::ops::persist_agent_message(
                    //         &db,
                    //         &record.id,
                    //         &updated_message,
                    //         relation.message_type,
                    //     )
                    //     .await
                    //     .inspect_err(|e| {
                    //         tracing::warn!(
                    //             "Failed to update message with sequence number: {:?}",
                    //             e
                    //         );
                    //     });
                    // }

                    loaded_messages += 1;
                }
            }

            // Debug: Check batch distribution
            let batch_count = history.batches.len();
            let message_distribution: Vec<usize> =
                history.batches.iter().map(|b| b.len()).collect();
            tracing::debug!(
                "Created {} batches from {} messages. Distribution: {:?} (showing first 10)",
                batch_count,
                loaded_messages,
                &message_distribution[..message_distribution.len().min(10)]
            );

            // Finalize and mark all loaded batches as complete - they're historical data
            for batch in history.batches.iter_mut() {
                batch.finalize();
                batch.mark_complete(); // Force complete since these are loaded from DB
            }
            for batch in history.archived_batches.iter_mut() {
                batch.finalize();
                batch.mark_complete(); // Force complete since these are loaded from DB
            }
            tracing::debug!("Loaded {} active messages into history", loaded_messages);
        }

        // Restore statistics
        {
            let context = agent.context.write().await;
            let mut metadata = context.metadata.write().await;
            metadata.created_at = record.created_at;
            metadata.last_active = record.last_active;
            metadata.total_messages = record.total_messages;
            metadata.total_tool_calls = record.total_tool_calls;
            metadata.context_rebuilds = record.context_rebuilds;
            metadata.compression_events = record.compression_events;
        }

        Ok(agent)
    }

    /// Update the context configuration of this agent
    ///
    /// This method allows updating the context config before the agent is wrapped in Arc.
    /// It updates both the in-memory config and optionally the compression strategy.
    pub async fn update_context_config(
        &self,
        context_config: ContextConfig,
        compression_strategy: Option<CompressionStrategy>,
    ) -> Result<()> {
        // Update the context config
        {
            let mut context = self.context.write().await;
            context.context_config = context_config;
        }

        // Update compression strategy if provided
        if let Some(strategy) = compression_strategy {
            let context = self.context.read().await;
            let mut history = context.history.write().await;
            history.compression_strategy = strategy;
        }

        Ok(())
    }

    /// Store the current agent state to the database
    pub async fn store(&self) -> Result<()> {
        // Create an AgentRecord from the current state
        let (agent_id, name, agent_type) = {
            let context = self.context.read().await;
            (
                context.handle.agent_id.clone(),
                context.handle.name.clone(),
                context.handle.agent_type.clone(),
            )
        };

        let (
            base_instructions,
            max_messages,
            memory_char_limit,
            enable_thinking,
            compression_strategy,
            message_summary,
        ) = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            (
                context.context_config.base_instructions.clone(),
                context.context_config.max_context_messages,
                context.context_config.memory_char_limit,
                context.context_config.enable_thinking,
                history.compression_strategy.clone(),
                history.archive_summary.clone(),
            )
        };

        let (total_messages, total_tool_calls, context_rebuilds, compression_events, last_active) = {
            let context = self.context.read().await;
            let metadata = context.metadata.read().await;
            (
                metadata.total_messages,
                metadata.total_tool_calls,
                metadata.context_rebuilds,
                metadata.compression_events,
                metadata.last_active,
            )
        };

        // Get current tool rules from the rule engine
        let tool_rules = {
            let rule_engine = self.tool_rules.read().await;
            rule_engine
                .get_rules()
                .iter()
                .map(|rule| crate::config::ToolRuleConfig::from_tool_rule(rule))
                .collect::<Vec<_>>()
        };
        // Get the current model_id from chat_options
        let model_id = {
            let options = self.chat_options.read().await;
            options.as_ref().map(|opt| opt.model_info.id.clone())
        };

        let now = chrono::Utc::now();
        let agent_record = crate::agent::AgentRecord {
            id: agent_id.clone(),
            name,
            agent_type,
            model_id,
            base_instructions,
            max_messages,
            memory_char_limit,
            enable_thinking,
            owner_id: self.user_id.clone(),
            tool_rules,
            total_messages,
            total_tool_calls,
            context_rebuilds,
            compression_events,
            created_at: now, // This will be overwritten if agent already exists
            updated_at: now,
            last_active,
            compression_strategy,
            message_summary,
            ..Default::default()
        };

        // ops::update_agent_context_config(&self.db, agent_id, &agent_record).await?;
        agent_record.store_with_relations(&self.db).await?;
        Ok(())
    }

    /// Start background task to sync memory updates via live queries
    pub async fn start_memory_sync(self: Arc<Self>) -> Result<()> {
        let (agent_id, memory) = {
            let context = self.context.read().await;
            (
                context.handle.agent_id.clone(),
                context.handle.memory.clone(),
            )
        };
        let db = self.db.clone();

        // Spawn background task to handle updates
        tokio::spawn(async move {
            use futures::StreamExt;
            tracing::debug!("Memory sync task started for agent {}", agent_id);

            // Subscribe to all memory changes for this agent
            let stream = match ops::subscribe_to_agent_memory_updates(&db, &agent_id).await {
                Ok(s) => {
                    tracing::debug!(
                        "Successfully subscribed to memory updates for agent {} - live query active",
                        agent_id
                    );
                    s
                }
                Err(e) => {
                    crate::log_error!("Failed to subscribe to memory updates", e);
                    return;
                }
            };

            futures::pin_mut!(stream);

            while let Some((action, memory_block)) = stream.next().await {
                tracing::debug!(
                    "🔔 Agent {} received live query notification: {:?} for block '{}' with content: '{}'",
                    agent_id,
                    action,
                    memory_block.label,
                    memory_block.value
                );

                match action {
                    surrealdb::Action::Create => {
                        tracing::debug!(
                            "Agent {} creating memory block '{}'",
                            agent_id,
                            memory_block.label
                        );
                        // Create new block
                        if let Err(e) =
                            memory.create_block(memory_block.label.clone(), &memory_block.value)
                        {
                            crate::log_error!(
                                format!("Failed to create memory block {}", memory_block.label),
                                e
                            );
                        }
                        // Update other fields if present
                        if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                            block.id = memory_block.id.clone();
                            block.owner_id = memory_block.owner_id;
                            block.description = memory_block.description;
                            block.metadata = memory_block.metadata;
                            block.embedding_model = memory_block.embedding_model;
                            block.created_at = memory_block.created_at;
                            block.updated_at = memory_block.updated_at;
                            block.is_active = memory_block.is_active;
                        }
                    }

                    surrealdb::Action::Update => {
                        // Update or create the block
                        if memory.get_block(&memory_block.label).is_some() {
                            tracing::debug!(
                                "✅ Agent {} updating existing memory '{}' with content: '{}'",
                                agent_id,
                                memory_block.label,
                                memory_block.value
                            );
                            // Update existing block
                            if let Err(e) =
                                memory.update_block_value(&memory_block.label, &memory_block.value)
                            {
                                crate::log_error!(
                                    format!("Failed to update memory block {}", memory_block.label),
                                    e
                                );
                            }
                        } else {
                            tracing::debug!(
                                "Agent {} creating new memory block '{}' (from update)",
                                agent_id,
                                memory_block.label
                            );
                            // Create new block
                            if let Err(e) =
                                memory.create_block(memory_block.label.clone(), &memory_block.value)
                            {
                                crate::log_error!(
                                    format!("Failed to create memory block {}", memory_block.label),
                                    e
                                );
                            }
                        }

                        // Update other fields if present
                        if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                            block.id = memory_block.id.clone();
                            block.owner_id = memory_block.owner_id;
                            block.description = memory_block.description;
                            block.metadata = memory_block.metadata;
                            block.embedding_model = memory_block.embedding_model;
                            block.created_at = memory_block.created_at;
                            block.updated_at = memory_block.updated_at;
                            block.is_active = memory_block.is_active;
                        }
                    }
                    surrealdb::Action::Delete => {
                        tracing::debug!(
                            "🗑️ Agent {} received DELETE for memory block '{}'",
                            agent_id,
                            memory_block.label
                        );
                        memory.remove_block(&memory_block.label);
                    }
                    _ => {
                        tracing::debug!("Ignoring action {:?} for memory block", action);
                    }
                }
            }

            tracing::warn!(
                "⚠️ Memory sync task exiting for agent {} - stream ended",
                agent_id
            );
        });

        Ok(())
    }

    /// Start background task to monitor incoming messages
    pub async fn start_message_monitoring(self: Arc<Self>) -> Result<()> {
        let db = self.db.clone();
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };

        // Create a weak reference for the spawned task
        let agent_clone = Arc::clone(&self);

        tokio::spawn(async move {
            // First, check for any existing unread messages
            let existing_query = format!(
                "SELECT * FROM queue_msg WHERE to_agent = $agent AND read = false ORDER BY created_at ASC",
            );

            let agent_record_id = RecordId::from(agent_id.clone());
            tracing::info!(
                "Checking for messages for agent {} (record: {:?})",
                agent_id,
                agent_record_id
            );

            match db
                .query(existing_query.clone())
                .bind(("agent", agent_record_id))
                .await
            {
                Ok(mut response) => {
                    if let Ok(messages) = response
                        .take::<Vec<<crate::message_queue::QueuedMessage as DbEntity>::DbModel>>(0)
                    {
                        let messages: Vec<_> = messages
                            .into_iter()
                            .map(|m| {
                                QueuedMessage::from_db_model(m).expect("should be db model type")
                            })
                            .collect();
                        if !messages.is_empty() {
                            tracing::info!(
                                "📬 Agent {} has {} unread messages",
                                agent_id,
                                messages.len()
                            );

                            // Process existing messages
                            for mut queued_msg in messages {
                                queued_msg.add_to_call_chain(agent_id.clone());

                                let mut message = if let Some(from_agent) = &queued_msg.from_agent {
                                    // Message from another agent - use User role but track origin
                                    let content = if let Some(origin) = &queued_msg.origin {
                                        origin.wrap_content(queued_msg.content.clone())
                                    } else {
                                        // Create a generic origin wrapper if none provided
                                        format!(
                                            "Message from agent {}:\n{}",
                                            from_agent, queued_msg.content
                                        )
                                    };
                                    Message::user(content)
                                } else if let Some(from_user) = &queued_msg.from_user {
                                    let mut msg = Message::user(queued_msg.content.clone());
                                    msg.owner_id = Some(from_user.clone());
                                    msg
                                } else {
                                    Message::system(queued_msg.content.clone())
                                };

                                if queued_msg.metadata
                                    != serde_json::Value::Object(Default::default())
                                {
                                    message.metadata = crate::message::MessageMetadata {
                                        custom: queued_msg.metadata.clone(),
                                        ..Default::default()
                                    };
                                }

                                // Mark message as read
                                queued_msg.mark_read();
                                let _ = queued_msg.store_with_relations(&db).await;

                                // Process the message
                                if let Err(e) =
                                    agent_clone.clone().process_message(message.clone()).await
                                {
                                    crate::log_error!("Failed to process queued message", e);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to check for existing messages: {}", e);
                }
            }

            // Now subscribe to new messages
            let stream = match crate::db::ops::subscribe_to_agent_messages(&db, &agent_id).await {
                Ok(stream) => stream,
                Err(e) => {
                    crate::log_error!("Failed to subscribe to agent messages", e);
                    return;
                }
            };

            tracing::info!("📬 Agent {} message monitoring started", agent_id);

            tokio::pin!(stream);
            loop {
                if let Some((action, mut queued_msg)) = stream.next().await {
                    let agent_clone = Arc::clone(&self);
                    match action {
                        surrealdb::Action::Create | surrealdb::Action::Update => {
                            tracing::info!(
                                "📨 Agent {}({}) received message from {:?}: {}",
                                agent_clone.name(),
                                agent_id,
                                queued_msg.origin,
                                queued_msg.content
                            );

                            // Update the call chain to include this agent
                            queued_msg.add_to_call_chain(agent_id.clone());

                            // Convert QueuedMessage to Message
                            let mut message = if let Some(_) = &queued_msg.from_agent {
                                // Message from another agent - use User role but track origin
                                let content = if let Some(origin) = queued_msg.origin {
                                    origin.wrap_content(queued_msg.content)
                                } else {
                                    queued_msg.content
                                };

                                Message::user(content)
                            } else if let Some(from_user) = &queued_msg.from_user {
                                // Message from a user
                                let mut msg = Message::user(queued_msg.content.clone());
                                msg.owner_id = Some(from_user.clone());
                                msg
                            } else {
                                // System message or unknown source
                                Message::user(queued_msg.content.clone())
                            };

                            // Add metadata if present
                            if queued_msg.metadata != serde_json::Value::Null {
                                message.metadata = crate::message::MessageMetadata {
                                    custom: queued_msg.metadata.clone(),
                                    ..Default::default()
                                };
                            }

                            // Extract and attach memory blocks from metadata
                            if let Some(blocks_value) = queued_msg.metadata.get("memory_blocks") {
                                if let Ok(memory_blocks) = serde_json::from_value::<
                                    Vec<(compact_str::CompactString, crate::memory::MemoryBlock)>,
                                >(
                                    blocks_value.clone()
                                ) {
                                    tracing::info!(
                                        "📝 Attaching {} memory blocks from data source to agent {}",
                                        memory_blocks.len(),
                                        agent_id
                                    );

                                    // Parallelize memory block storage and attachment
                                    let attachment_futures = memory_blocks.into_iter().map(|(label, block)| {
                                        let db = db.clone();
                                        let agent_id = agent_id.clone();

                                        async move {
                                            // First store the memory block if it doesn't exist
                                            let block_id = block.id.clone();
                                            if let Err(e) = block.store_with_relations(&db).await {
                                                tracing::warn!(
                                                    "Failed to store memory block {}: {}",
                                                    label,
                                                    e
                                                );
                                                return;
                                            }

                                            // Use the proper ops function to attach memory
                                            if let Err(e) = crate::db::ops::attach_memory_to_agent(
                                                &db,
                                                &agent_id,
                                                &block_id,
                                                block.permission,
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    "Failed to attach memory block {} to agent: {}",
                                                    label,
                                                    e
                                                );
                                            } else {
                                                tracing::debug!(
                                                    "✅ Attached memory block {} to agent {}",
                                                    label,
                                                    agent_id
                                                );
                                            }
                                        }
                                    });

                                    // Execute all attachments in parallel
                                    futures::future::join_all(attachment_futures).await;
                                }
                            }

                            // Mark message as read immediately to prevent reprocessing
                            if let Err(e) =
                                crate::db::ops::mark_message_as_read(&db, &queued_msg.id).await
                            {
                                crate::log_error!("Failed to mark message as read", e);
                            }

                            tracing::info!(
                                "💬 Agent {} processing message from {:?}{:?}",
                                agent_id,
                                queued_msg.from_agent,
                                queued_msg.from_user
                            );

                            // Call process_message through the Agent trait
                            match agent_clone.process_message(message).await {
                                Ok(response) => {
                                    tracing::debug!(
                                        "✅ Agent {} successfully processed incoming message, response has {} content parts",
                                        agent_id,
                                        response.content.len()
                                    );

                                    // If there's a response with actual content, we might want to send it back
                                    // For now, just log it
                                    for content in &response.content {
                                        match content {
                                            MessageContent::Text(text) => {
                                                tracing::info!("📤 Agent response: {}", text);
                                            }
                                            _ => {
                                                tracing::debug!(
                                                    "📤 Agent response contains non-text content"
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    crate::log_error_chain!(
                                        format!(
                                            "❌ Agent {} failed to process incoming message:",
                                            agent_id
                                        ),
                                        e
                                    );
                                    break;
                                }
                            }
                        }
                        _ => {
                            tracing::info!("Ignoring action {:?} for message queue", action);
                        }
                    }
                }
            }

            tracing::warn!(
                "⚠️ Message monitoring task exiting for agent {} - stream ended",
                agent_id
            );
        });

        Ok(())
    }

    /// Start background task to sync agent stats updates with UI
    pub async fn start_stats_sync(&self) -> Result<()> {
        let db = self.db.clone();
        let (agent_id, metadata, history) = {
            let context = self.context.read().await;
            (
                context.handle.agent_id.clone(),
                Arc::clone(&context.metadata),
                Arc::clone(&context.history),
            )
        };

        tokio::spawn(async move {
            let stream = match crate::db::ops::subscribe_to_agent_stats(&db, &agent_id).await {
                Ok(stream) => stream,
                Err(e) => {
                    crate::log_error!("Failed to subscribe to agent stats updates", e);
                    return;
                }
            };

            tracing::debug!("📊 Agent {} stats sync task started", agent_id);

            tokio::pin!(stream);
            while let Some((action, agent_record)) = stream.next().await {
                match action {
                    surrealdb::Action::Update => {
                        tracing::debug!(
                            "📊 Agent {} received stats update - messages: {}, tool calls: {}",
                            agent_id,
                            agent_record.total_messages,
                            agent_record.total_tool_calls
                        );

                        // Update metadata with the latest stats
                        let mut meta = metadata.write().await;
                        meta.total_messages = agent_record.total_messages;
                        meta.total_tool_calls = agent_record.total_tool_calls;
                        meta.last_active = agent_record.last_active;

                        // Update compression events if the message summary changed
                        let history_read = history.read().await;
                        if agent_record.message_summary != history_read.archive_summary {
                            meta.compression_events += 1;
                        }
                        drop(history_read);

                        // Update history if message summary changed
                        if agent_record.message_summary.is_some() {
                            let mut history_write = history.write().await;
                            history_write.archive_summary = agent_record.message_summary;
                        }
                    }
                    _ => {
                        tracing::debug!("Ignoring action {:?} for agent stats", action);
                    }
                }
            }

            tracing::warn!(
                "⚠️ Stats sync task exiting for agent {} - stream ended",
                agent_id
            );
        });

        Ok(())
    }

    /// Compress messages and persist archival to database
    pub async fn compress_messages_if_needed(&self) -> Result<()> {
        // Check if compression is needed
        let needs_compression = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            history.total_message_count() > context.context_config.max_context_messages
        };

        if !needs_compression {
            return Ok(());
        }

        // Get compression strategy and create compressor
        let compression_strategy = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            history.compression_strategy.clone()
        };

        // Use the module-level ModelProviderWrapper
        let model_wrapper = ModelProviderWrapper {
            model: self.model.clone(),
        };

        let compressor = crate::context::MessageCompressor::new(compression_strategy)
            .with_model_provider(Arc::new(model_wrapper));

        // Get batches to compress
        let batches_to_compress = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            history.batches.clone()
        };

        // Perform compression
        let (max_context_messages, max_context_tokens) = {
            let context = self.context.read().await;
            (
                context.context_config.max_context_messages,
                context.context_config.max_context_tokens,
            )
        };
        let compression_result = compressor
            .compress(
                batches_to_compress,
                max_context_messages,
                max_context_tokens,
            )
            .await?;

        // Collect archived message IDs from archived batches
        let archived_ids: Vec<crate::MessageId> = compression_result
            .archived_batches
            .iter()
            .flat_map(|batch| batch.messages.iter().map(|msg| msg.id.clone()))
            .collect();

        // Apply compression to state
        {
            let context = self.context.read().await;
            let mut history = context.history.write().await;

            // Move compressed batches to archive
            history
                .archived_batches
                .extend(compression_result.archived_batches);

            // Update active batches
            history.batches = compression_result.active_batches;

            // Update or append to summary
            if let Some(new_summary) = compression_result.summary {
                if let Some(existing_summary) = &mut history.archive_summary {
                    *existing_summary = format!("{}\n\n{}", existing_summary, new_summary);
                } else {
                    history.archive_summary = Some(new_summary.clone());
                }

                // Also update the agent record's message summary in background
                let db = self.db.clone();
                let agent_id = context.handle.agent_id.clone();
                let summary = new_summary;
                tokio::spawn(async move {
                    let query = r#"
                        UPDATE agent SET
                            message_summary = $summary,
                            updated_at = time::now()
                        WHERE id = $id
                    "#;

                    if let Err(e) = db
                        .query(query)
                        .bind(("id", surrealdb::RecordId::from(agent_id)))
                        .bind(("summary", summary))
                        .await
                    {
                        crate::log_error!("Failed to update agent message summary", e);
                    }
                });
            }

            history.last_compression = chrono::Utc::now();
        }

        // Update metadata
        {
            let context = self.context.read().await;
            let mut metadata = context.metadata.write().await;
            metadata.compression_events += 1;
        }

        // Persist archived message IDs to database in background
        if !archived_ids.is_empty() {
            let db = self.db.clone();
            let agent_id = {
                let context = self.context.read().await;
                context.handle.agent_id.clone()
            };
            tokio::spawn(async move {
                if let Err(e) =
                    crate::db::ops::archive_agent_messages(&db, &agent_id, &archived_ids).await
                {
                    crate::log_error!("Failed to archive messages in database", e);
                } else {
                    tracing::debug!(
                        "Successfully archived {} messages for agent {} in database",
                        archived_ids.len(),
                        agent_id
                    );
                }
            });
        }

        Ok(())
    }

    /// Persist memory changes to database
    pub async fn persist_memory_changes(&self) -> Result<()> {
        let context = self.context.read().await;
        let memory = &context.handle.memory;

        // Update constellation activity memory block if we have a tracker
        if let Some(tracker) = &context.constellation_tracker {
            let updated_content = tracker.format_as_memory_content().await;
            let owner_id = memory.owner_id.clone();
            let updated_block = crate::constellation_memory::create_constellation_activity_block(
                tracker.memory_id().clone(),
                owner_id,
                updated_content,
            );

            // Update the memory block in the agent's memory
            if let Err(e) = memory.update_block_value("constellation_activity", updated_block.value)
            {
                tracing::warn!("Failed to update constellation activity memory: {:?}", e);
            } else {
                tracing::trace!("Successfully updated constellation activity memory block");
            }
        }

        // Get the lists of blocks that need persistence
        let new_blocks = memory.get_new_blocks();
        let dirty_blocks = memory.get_dirty_blocks();

        tracing::info!(
            "Persisting memory changes for {}: {} new blocks, {} dirty blocks",
            context.handle.agent_id,
            new_blocks.len(),
            dirty_blocks.len()
        );

        // Log the actual block labels we're about to persist
        for block_id in &new_blocks {
            if let Some(block) = memory
                .get_all_blocks()
                .into_iter()
                .find(|b| &b.id == block_id)
            {
                tracing::debug!(
                    "  NEW block to persist: {} (type: {:?})",
                    block.label,
                    block.memory_type
                );
            }
        }
        for block_id in &dirty_blocks {
            if let Some(block) = memory
                .get_all_blocks()
                .into_iter()
                .find(|b| &b.id == block_id)
            {
                tracing::debug!(
                    "  DIRTY block to persist: {} (type: {:?})",
                    block.label,
                    block.memory_type
                );
            }
        }

        // Handle new blocks - need to create and attach to agent
        for block_id in &new_blocks {
            if let Some(block) = memory
                .get_all_blocks()
                .into_iter()
                .find(|b| &b.id == block_id)
            {
                tracing::debug!("Creating new memory block {} in database", block.label);

                // Use persist_agent_memory which handles store_with_relations + retry logic
                match ops::persist_agent_memory(
                    &self.db,
                    context.handle.agent_id.clone(),
                    &block,
                    block.permission,
                )
                .await
                {
                    Ok(_) => {
                        tracing::debug!(
                            "Successfully persisted memory block {} to database",
                            block.label
                        );
                        // Mark this specific block as persisted
                        memory.mark_block_persisted(block_id);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to persist new memory block {} to database: {:?}",
                            block.label,
                            e
                        );
                        // Don't return early - continue with other blocks
                    }
                }
            }
        }

        // Handle dirty blocks - just need to update
        for block_id in &dirty_blocks {
            if let Some(mut block) = memory
                .get_all_blocks()
                .into_iter()
                .find(|b| &b.id == block_id)
            {
                tracing::debug!("Updating dirty memory block {} in database", block.label);

                // Update the timestamp
                block.updated_at = chrono::Utc::now();

                // Use persist_agent_memory which handles store_with_relations + retry logic
                match ops::persist_agent_memory(
                    &self.db,
                    context.handle.agent_id.clone(),
                    &block,
                    block.permission,
                )
                .await
                {
                    Ok(_) => {
                        tracing::debug!("Successfully updated dirty memory block {}", block.label);
                        // Mark this specific block as persisted
                        memory.mark_block_persisted(block_id);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to update dirty memory block {}: {:?}",
                            block.label,
                            e
                        );
                        // Don't return early - continue with other blocks
                    }
                }
            }
        }

        tracing::debug!(
            "Completed memory persistence attempt for {} - blocks marked as persisted individually",
            context.handle.agent_id
        );

        Ok(())
    }

    /// Add messages to context and persist to database
    async fn persist_response_messages(
        &self,
        response: &Response,
        agent_id: &AgentId,
        batch_id: Option<crate::SnowflakePosition>,
        batch_type: Option<BatchType>,
    ) -> Result<()> {
        let response_messages = {
            let context = self.context.read().await;
            Message::from_response(response, &context.handle.agent_id, batch_id, batch_type)
        };

        for message in response_messages {
            if !message.content.is_empty() {
                tracing::debug!(
                    "Adding message with role: {:?}, content type: {:?}",
                    message.role,
                    match &message.content {
                        MessageContent::Text(_) => "Text",
                        MessageContent::Parts(_) => "Parts",
                        MessageContent::ToolCalls(_) => "ToolCalls",
                        MessageContent::ToolResponses(_) => "ToolResponses",
                        MessageContent::Blocks(_) => "Blocks",
                    }
                );

                // Add the message to context and get the updated version with sequence numbers
                let updated_message = {
                    let context = self.context.read().await;
                    context.add_message(message).await
                };

                // Persist the updated message to DB (now with sequence numbers!)
                let _ = crate::db::ops::persist_agent_message(
                    &self.db,
                    agent_id,
                    &updated_message,
                    crate::message::MessageRelationType::Active,
                )
                .await
                .inspect_err(|e| {
                    crate::log_error!("Failed to persist response message", e);
                });
            }
        }

        Ok(())
    }

    /// Execute tool calls with rule validation and state tracking
    async fn execute_tools_with_rules(
        &self,
        calls: &[crate::message::ToolCall],
    ) -> Result<(Vec<ToolResponse>, bool)> {
        let mut responses = Vec::new();
        let mut continuation_requested = false;

        // Get current rule engine state
        let mut rule_engine = self.tool_rules.write().await;

        // Build a recent dedupe set from execution history (last 5 minutes)
        let mut seen_keys: HashSet<String> = HashSet::new();
        {
            let now_recent = std::time::Duration::from_secs(5 * 60);
            let state = rule_engine.get_execution_state().clone();
            for exec in state.execution_history.iter().rev().take(200) {
                // Only consider recent successful executions
                if exec.timestamp.elapsed() <= now_recent {
                    if let Some(meta) = &exec.metadata {
                        // Expecting metadata to possibly contain canonical args under "args"
                        if let Some(args_val) = meta.get("args") {
                            // Rebuild a synthetic key using tool name + canonical args
                            let canonical_args = Self::canonicalize_json(args_val);
                            if let Ok(args_str) = serde_json::to_string(&canonical_args) {
                                seen_keys.insert(format!("{}|{}", exec.tool_name, args_str));
                            }
                        }
                    }
                } else {
                    // Older than window; break early as list is reverse chronological by insertion order
                    // (best-effort; if not strictly ordered, this still limits scan)
                    // Do not break if ordering is unsure
                }
            }
        }

        for call in calls {
            // Check for duplicate tool call by content (name + canonicalized args)
            let dedupe_key = Self::tool_call_dedupe_key(call);
            if seen_keys.contains(&dedupe_key) {
                tracing::warn!(
                    "♻️ Duplicate tool call suppressed: {} with identical arguments",
                    call.fn_name
                );
                // Return a normal tool response explaining suppression, then short-circuit
                responses.push(ToolResponse {
                    call_id: call.call_id.clone(),
                    content: format!(
                        "Duplicate tool call detected for '{}'; skipping re-execution to avoid repetition.",
                        call.fn_name
                    ),
                    is_error: Some(true),
                });

                // Short-circuit tool execution loop per design
                break;
            }

            // Check if tool can be executed according to rules
            match rule_engine.can_execute_tool(&call.fn_name) {
                Ok(_) => {
                    // Tool can be executed - proceed
                    tracing::debug!("✅ Tool {} passed rule validation", call.fn_name);
                }
                Err(violation) => {
                    // Rule violation - create error response
                    tracing::warn!(
                        "❌ Tool {} failed rule validation: {:?}",
                        call.fn_name,
                        violation
                    );
                    let error_response = ToolResponse {
                        call_id: call.call_id.clone(),
                        content: format!("Tool rule violation: {:?}", violation),
                        is_error: Some(true),
                    };
                    responses.push(error_response);
                    continue;
                }
            }

            // Check if tool requires heartbeat (optimization)
            let needs_heartbeat = rule_engine.requires_heartbeat(&call.fn_name);
            let has_heartbeat_param = check_heartbeat_request(&call.fn_arguments);

            // Check if tool has ContinueLoop rule (which means it should continue but doesn't need heartbeat param)
            let has_continue_rule = !needs_heartbeat; // If doesn't need heartbeat, it has ContinueLoop

            tracing::debug!(
                "🔍 Tool {} heartbeat check: needs_heartbeat={}, has_param={}, has_continue_rule={}, args={:?}",
                call.fn_name,
                needs_heartbeat,
                has_heartbeat_param,
                has_continue_rule,
                call.fn_arguments
            );

            // Track if ANY tool wants continuation - we'll handle it synchronously
            if (needs_heartbeat && has_heartbeat_param) || has_continue_rule {
                continuation_requested = true;
                tracing::debug!(
                    "💓 Continuation requested by tool {} (call_id: {}) - will handle synchronously",
                    call.fn_name,
                    call.call_id
                );
            }

            // Execute tool using the context method
            let ctx = self.context.read().await;
            let tool_response = ctx.process_tool_call(&call).await.unwrap_or_else(|e| {
                Some(ToolResponse {
                    call_id: call.call_id.clone(),
                    content: format!("Error executing tool: {:?}", e),
                    is_error: Some(true),
                })
            });

            if let Some(tool_response) = tool_response {
                // Record successful execution in rule engine
                let execution_success = !tool_response.content.starts_with("Error:");
                let execution = crate::agent::tool_rules::ToolExecution {
                    tool_name: call.fn_name.clone(),
                    call_id: call.call_id.clone(),
                    timestamp: std::time::Instant::now(),
                    success: execution_success,
                    // Store canonicalized args in metadata for future dedupe
                    metadata: Some(serde_json::json!({
                        "args": Self::canonicalize_json(&call.fn_arguments)
                    })),
                };
                rule_engine.record_execution(execution);

                responses.push(tool_response);

                // Mark this call's content as seen to dedupe subsequent identical calls
                seen_keys.insert(dedupe_key);

                // Check if we should exit after this tool
                if rule_engine.should_exit_loop() {
                    tracing::info!("Tool {} triggered exit loop rule", call.fn_name);
                    break;
                }
            }
        }

        Ok((responses, continuation_requested))
    }

    /// Execute start constraint tools if any are required
    async fn execute_start_constraint_tools(&self) -> Result<Vec<ToolResponse>> {
        let rule_engine = self.tool_rules.read().await;
        let start_tools = rule_engine.get_start_constraint_tools();

        if start_tools.is_empty() {
            return Ok(Vec::new());
        }

        tracing::info!("Executing {} start constraint tools", start_tools.len());

        // Create tool calls for start constraint tools
        let start_calls: Vec<crate::message::ToolCall> = start_tools
            .into_iter()
            .map(|tool_name| crate::message::ToolCall {
                call_id: format!("start_{}", uuid::Uuid::new_v4().simple()),
                fn_name: tool_name,
                fn_arguments: serde_json::json!({}), // Empty args for start tools
            })
            .collect();

        // Drop the read lock before calling execute_tools_with_rules
        drop(rule_engine);

        // Execute the start tools
        self.execute_tools_with_rules(&start_calls)
            .await
            .map(|(responses, _)| responses)
    }

    /// Execute required exit tools if any are needed
    async fn execute_required_exit_tools(&self) -> Result<Vec<ToolResponse>> {
        let rule_engine = self.tool_rules.read().await;
        let exit_tools = rule_engine.get_required_before_exit_tools();

        if exit_tools.is_empty() {
            return Ok(Vec::new());
        }

        tracing::info!("Executing {} required exit tools", exit_tools.len());

        // Create tool calls for exit tools
        let exit_calls: Vec<crate::message::ToolCall> = exit_tools
            .into_iter()
            .map(|tool_name| crate::message::ToolCall {
                call_id: format!("exit_{}", uuid::Uuid::new_v4().simple()),
                fn_name: tool_name,
                fn_arguments: serde_json::json!({}), // Empty args for exit tools
            })
            .collect();

        // Drop the read lock before calling execute_tools_with_rules
        drop(rule_engine);

        // Execute the exit tools
        self.execute_tools_with_rules(&exit_calls)
            .await
            .map(|(responses, _)| responses)
    }

    /// Process a message and stream responses as they happen
    pub async fn process_message_stream(
        self: Arc<Self>,
        mut message: Message,
    ) -> Result<impl Stream<Item = ResponseEvent>> {
        use tokio_stream::wrappers::ReceiverStream;

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Clone what we need for the spawned task
        let agent_id = self.id();
        let db = self.db.clone();
        let context = self.context.clone();
        let model = self.model.clone();
        let chat_options = self.chat_options.clone();
        let self_clone = self.clone();

        // Get model vendor from options
        let model_vendor = {
            let opts = chat_options.read().await;
            opts.as_ref()
                .map(|o| crate::model::ModelVendor::from_provider_string(&o.model_info.provider))
        };

        // Spawn the processing task
        tokio::spawn(async move {
            // Batch-aware waiting logic
            let (current_state, maybe_receiver) = self_clone.state().await;
            match current_state {
                AgentState::Ready => {
                    tracing::info!("pre-request: state=Ready; brief 10ms debounce");
                    // Ready to process, but maybe wait JUST a little
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                AgentState::Processing { active_batches: _ } => {
                    // Check if we're continuing an existing batch
                    if let Some(batch_id) = message.batch {
                        // Try to find and optionally wait on the batch's notifier
                        let notifier = {
                            let ctx = context.read().await;
                            let history = ctx.history.read().await;
                            history
                                .batches
                                .iter()
                                .find(|b| b.id == batch_id)
                                .map(|batch| batch.get_tool_pairing_notifier())
                        };

                        if let Some(notifier) = notifier {
                            tracing::info!(
                                "pre-request: state=Processing; batch={:?}; found notifier; waiting ≤180s",
                                batch_id
                            );
                            let t0 = std::time::Instant::now();
                            let _ =
                                tokio::time::timeout(Duration::from_secs(180), notifier.notified())
                                    .await;
                            tracing::info!(
                                "pre-request: notifier wait finished after {}ms (timeout or fired)",
                                t0.elapsed().as_millis()
                            );
                            if let Some(mut receiver) = maybe_receiver {
                                tracing::info!("pre-request: waiting for Ready state (≤5s)");
                                let timeout = tokio::time::timeout(
                                    Duration::from_secs(5),
                                    receiver.wait_for(|s| matches!(s, AgentState::Ready)),
                                );
                                let _ = timeout.await;
                            }
                        } else {
                            tracing::info!(
                                "pre-request: state=Processing; batch={:?}; no notifier found; proceeding",
                                batch_id
                            );
                        }
                    } else {
                        // Allow concurrent processing for new incoming messages; tiny debounce only
                        tracing::info!(
                            "pre-request: state=Processing; no batch id; proceeding concurrently (10ms debounce)"
                        );
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
                AgentState::Error {
                    kind,
                    message: error_msg,
                } => {
                    tracing::info!(
                        "pre-request: state=Error({:?}); sleeping 30s and attempting recovery",
                        kind
                    );
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    // Run error recovery based on kind
                    tracing::warn!("Agent in error state: {:?} - {}", kind, error_msg);

                    // Use the centralized recovery method
                    self_clone
                        .run_error_recovery(&context, kind, &error_msg)
                        .await;

                    // Reset to Ready state after recovery
                    self_clone.set_state(AgentState::Ready).await.ok();
                    tracing::info!("Error recovery complete, agent reset to Ready");
                }
                _ => {
                    // Cooldown or Suspended - wait for Ready
                    tracing::info!(
                        "pre-request: state={:?}; waiting for Ready (≤200s)",
                        current_state
                    );
                    if let Some(mut receiver) = maybe_receiver {
                        let timeout = tokio::time::timeout(
                            Duration::from_secs(200),
                            receiver.wait_for(|s| matches!(s, AgentState::Ready)),
                        );
                        let _ = timeout.await;
                    }
                }
            }

            // Generate batch ID early so we can track it in state
            let current_batch_id = Some(message.batch.unwrap_or(get_next_message_position_sync()));
            let current_batch_type = message.batch_type.clone().or(Some(BatchType::UserRequest));

            // Add this batch to active batches (preserving any existing ones)
            let (current_state, _) = self_clone.state().await;
            let mut active_batches = match current_state {
                AgentState::Processing { active_batches } => active_batches,
                _ => std::collections::HashSet::new(),
            };
            if let Some(batch_id) = current_batch_id {
                active_batches.insert(batch_id);
            }
            self_clone
                .set_state(AgentState::Processing { active_batches })
                .await
                .ok();

            // Helper to send events
            let send_event = |event: ResponseEvent| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(event).await;
                }
            };

            // Capture the incoming message ID and content for the completion event
            let incoming_message_id = message.id.clone();
            let incoming_message_role = message.role.clone();

            // Transform text content with image markers into multimodal parts
            if let crate::message::MessageContent::Text(ref text) = message.content {
                if let Some(parts) = crate::message::parse_multimodal_markers(text) {
                    tracing::info!(
                        "📸 Transforming text with {} image markers into multimodal content",
                        parts
                            .iter()
                            .filter(|p| matches!(p, crate::message::ContentPart::Image { .. }))
                            .count()
                    );
                    message.content = crate::message::MessageContent::Parts(parts);
                }
            }

            // Extract message text for user messages only
            let incoming_message_summary = if matches!(message.role, crate::message::ChatRole::User)
            {
                match &message.content {
                    crate::message::MessageContent::Text(text) => {
                        // Truncate to approximately 1000 chars total (500 from start, 500 from end)
                        let truncated = if text.len() > 1000 {
                            // Find safe char boundaries
                            let start_boundary = text
                                .char_indices()
                                .take_while(|(i, _)| *i <= 500)
                                .last()
                                .map(|(i, c)| i + c.len_utf8())
                                .unwrap_or(0);

                            let end_start = text.len().saturating_sub(500);
                            let end_boundary = text
                                .char_indices()
                                .skip_while(|(i, _)| *i < end_start)
                                .next()
                                .map(|(i, _)| i)
                                .unwrap_or(text.len());

                            let start = &text[..start_boundary];
                            let end = &text[end_boundary..];
                            format!("{}\n...\n{}", start, end)
                        } else {
                            text.clone()
                        };
                        Some(truncated)
                    }
                    _ => None,
                }
            } else {
                None
            };

            // Accumulate agent's response text for constellation logging
            let mut agent_response_text = String::new();

            // Track memory blocks loaded for this request (for cleanup later)
            let mut loaded_memory_labels = Vec::<String>::new();

            // Extract and attach memory blocks from metadata if present
            if let Some(blocks_value) = message.metadata.custom.get("memory_blocks") {
                if let Ok(memory_blocks) = serde_json::from_value::<
                    Vec<(compact_str::CompactString, crate::memory::MemoryBlock)>,
                >(blocks_value.clone())
                {
                    tracing::info!(
                        "📎 Attaching {} memory blocks from message metadata",
                        memory_blocks.len()
                    );

                    // Parallelize memory block insertion
                    let insertion_futures = memory_blocks.into_iter().map(|(label, block)| {
                        let handle = self_clone.handle();
                        let value = block.value.clone();

                        // Track this label for cleanup later
                        loaded_memory_labels.push(label.to_string());

                        async move {
                            let handle = handle.await;
                            match handle.insert_working_memory(&label, &value).await {
                                Ok(_) => {
                                    tracing::debug!(
                                        "Added memory block {} as working memory",
                                        label
                                    );
                                    Ok(())
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to add memory block {} as working memory: {:?}",
                                        label,
                                        e
                                    );
                                    Err(e)
                                }
                            }
                        }
                    });

                    // Execute all insertions in parallel
                    let _ = futures::future::join_all(insertion_futures).await;
                }
            }

            let _ = self.persist_memory_changes().await;

            // Update message with batch info if needed
            if message.batch.is_none() {
                message.position = current_batch_id;
                message.sequence_num = Some(0);
                message.batch_type = current_batch_type.clone();
            } else {
                // need to find the batch and get the correct sequence num?
            }
            message.batch = current_batch_id;

            // Update state and persist message
            {
                let mut ctx = context.write().await;

                // Add this batch to active batches (preserving any existing ones)
                let mut active_batches = match &ctx.handle.state {
                    AgentState::Processing { active_batches } => active_batches.clone(),
                    _ => std::collections::HashSet::new(),
                };
                if let Some(batch_id) = current_batch_id {
                    active_batches.insert(batch_id);
                }
                ctx.handle.state = AgentState::Processing { active_batches };

                // Add message to context first to get sequence numbers
                let updated_message = ctx.add_message(message).await;

                // Now persist with the sequence numbers
                let _ = crate::db::ops::persist_agent_message(
                    &db,
                    &agent_id,
                    &updated_message,
                    crate::message::MessageRelationType::Active,
                )
                .await
                .inspect_err(|e| {
                    crate::log_error!("Failed to persist incoming message", e);
                });
            }

            // Add tool rules from rule engine to context before building
            {
                let tool_rules = self.get_context_tool_rules().await;
                let consent_tools = self.get_consent_required_tools().await;
                let mut ctx = context.write().await;
                ctx.add_tool_rules(tool_rules);
                ctx.context_config.consent_required_tools = consent_tools;
            }

            // Build memory context with the current batch ID
            tracing::info!("Building memory context (batch {:?})", current_batch_id);
            let memory_context = match context.read().await.build_context(current_batch_id).await {
                Ok(ctx) => ctx,
                Err(e) => {
                    // Clean up errors and set state before exiting
                    let error_msg = format!("Failed to build context: {:?}", e);
                    context
                        .read()
                        .await
                        .cleanup_errors(current_batch_id, &error_msg)
                        .await;

                    // Clean up temporary memory blocks
                    self_clone
                        .cleanup_temporary_memory_blocks(&loaded_memory_labels)
                        .await;
                    {
                        let mut ctx = context.write().await;
                        ctx.handle.state = AgentState::Error {
                            kind: RecoverableErrorKind::ContextBuildFailed,
                            message: error_msg.clone(),
                        };
                    }

                    send_event(ResponseEvent::Error {
                        message: error_msg,
                        recoverable: false,
                    })
                    .await;
                    return;
                }
            };
            tracing::info!(
                "Context built: messages={}, tools={}",
                memory_context.messages().len(),
                memory_context.tools.len()
            );

            // Execute start constraint tools if any are required
            tracing::info!("Checking and executing start-constraint tools (if any)");
            match self_clone.execute_start_constraint_tools().await {
                Ok(start_responses) => {
                    if !start_responses.is_empty() {
                        tracing::info!(
                            "✅ Executed {} start constraint tools",
                            start_responses.len()
                        );
                        // Emit events for start constraint tool execution
                        send_event(ResponseEvent::ToolResponses {
                            responses: start_responses,
                        })
                        .await;
                    }
                }
                Err(e) => {
                    crate::log_error_chain!("❌ Failed to execute start constraint tools:", e);

                    // Clean up errors and set state before exiting
                    let error_msg = format!("Start constraint tools failed: {:?}", e);
                    context
                        .read()
                        .await
                        .cleanup_errors(current_batch_id, &error_msg)
                        .await;

                    // Clean up temporary memory blocks
                    self_clone
                        .cleanup_temporary_memory_blocks(&loaded_memory_labels)
                        .await;
                    {
                        let mut ctx = context.write().await;
                        ctx.handle.state = AgentState::Error {
                            kind: RecoverableErrorKind::ContextBuildFailed,
                            message: error_msg.clone(),
                        };
                    }

                    send_event(ResponseEvent::Error {
                        message: format!("Start constraint tools failed: {:?}", e),
                        recoverable: false,
                    })
                    .await;
                    return;
                }
            }

            // Create request
            tracing::info!(
                "Preparing model request (messages={}, tools={})",
                memory_context.messages().len(),
                memory_context.tools.len()
            );
            let request = Request {
                system: Some(vec![memory_context.system_prompt.clone()]),
                messages: memory_context.messages(),
                tools: Some(memory_context.tools),
            };

            let options = chat_options
                .read()
                .await
                .clone()
                .expect("should have options or default");

            // Wait for any pending tool responses with timeout
            if let Some(batch_id) = current_batch_id {
                let notifier = {
                    let ctx = context.read().await;
                    let history = ctx.history.read().await;
                    history
                        .batches
                        .iter()
                        .find(|b| b.id == batch_id)
                        .and_then(|batch| {
                            let pending = batch.get_pending_tool_calls();
                            if !pending.is_empty() {
                                tracing::info!(
                                    "Waiting for {} tool responses before LLM request (max 5s)",
                                    pending.len()
                                );
                                Some(batch.get_tool_pairing_notifier())
                            } else {
                                None
                            }
                        })
                };

                if let Some(notifier) = notifier {
                    // Wait for notification or timeout
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        notifier.notified(),
                    )
                    .await
                    {
                        Ok(_) => {
                            tracing::info!("Tool responses received, continuing with request");
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Timeout waiting for tool responses, cleaning up unpaired calls"
                            );
                            // Finalize batch to remove unpaired calls
                            let ctx = context.write().await;
                            let mut history = ctx.history.write().await;
                            if let Some(batch) =
                                history.batches.iter_mut().find(|b| b.id == batch_id)
                            {
                                let removed = batch.finalize();
                                if !removed.is_empty() {
                                    tracing::warn!(
                                        "Removed {} unpaired messages from batch",
                                        removed.len()
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Get response from model with retry logic
            let response = {
                let model = model.read().await;
                match Self::complete_with_retry(&*model, &options, request, 10).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        // Parse error to determine recovery type using reusable function
                        let error_str = e.to_string();
                        let error_kind = RecoverableErrorKind::from_error_str(&error_str);

                        // Extract additional context if available (like Anthropic's problematic index)
                        if let Some(context) =
                            RecoverableErrorKind::extract_error_context(&error_str)
                        {
                            tracing::debug!("Error context extracted: {:?}", context);
                            // TODO: Use this context for more targeted fixes
                        }

                        // Now clean up any remaining unpaired calls and set state
                        let error_msg = format!("Model error: {:?}", e);
                        context
                            .read()
                            .await
                            .cleanup_errors(current_batch_id, &error_msg)
                            .await;

                        // Clean up temporary memory blocks
                        self_clone
                            .cleanup_temporary_memory_blocks(&loaded_memory_labels)
                            .await;
                        {
                            let mut ctx = context.write().await;
                            ctx.handle.state = AgentState::Error {
                                kind: error_kind,
                                message: error_msg.clone(),
                            };
                        }

                        send_event(ResponseEvent::Error {
                            message: format!("Model error: {:?}", e),
                            recoverable: false,
                        })
                        .await;
                        return;
                    }
                }
            };

            let mut current_response = response;
            let mut should_continue_after_tools = false;

            loop {
                let has_unpaired_tool_calls = current_response.has_unpaired_tool_calls();
                if has_unpaired_tool_calls {
                    // Check if we have thinking blocks - if so, we'll emit reasoning from those instead
                    let has_thinking_blocks = current_response.content.iter().any(|content| {
                        matches!(content, MessageContent::Blocks(blocks) if blocks.iter().any(|b| matches!(b, ContentBlock::Thinking { .. })))
                    });

                    // Only emit aggregated reasoning if we don't have thinking blocks
                    if !has_thinking_blocks {
                        if let Some(reasoning) = &current_response.reasoning {
                            agent_response_text.push_str("\n[Reasoning]:\n");
                            agent_response_text.push_str(&reasoning);
                            send_event(ResponseEvent::ReasoningChunk {
                                text: reasoning.clone(),
                                is_final: true,
                            })
                            .await;
                        }
                    }
                    // Build the processed response manually since we already executed tools
                    let mut processed_response = current_response.clone();
                    processed_response.content.clear();

                    // Use peekable iterator to properly handle tool call/response pairing
                    let mut content_iter = current_response.content.iter().peekable();

                    while let Some(content) = content_iter.next() {
                        match content {
                            MessageContent::ToolCalls(calls) => {
                                // Always include the tool calls
                                processed_response
                                    .content
                                    .push(MessageContent::ToolCalls(calls.clone()));

                                // Check if next item is already tool responses (provider-handled tools)
                                let next_is_responses = content_iter
                                    .peek()
                                    .map(|next| matches!(next, MessageContent::ToolResponses(_)))
                                    .unwrap_or(false);

                                if next_is_responses {
                                    // Provider already included responses, pass them through
                                    // TODO: check if some of the tool calls here are ours, execute those,
                                    // then append out responses to them.

                                    if let Some(next_content) = content_iter.next() {
                                        processed_response.content.push(next_content.clone());
                                    }
                                } else {
                                    if !calls.is_empty() {
                                        send_event(ResponseEvent::ToolCalls {
                                            calls: calls.clone(),
                                        })
                                        .await;

                                        // Send individual tool call started events
                                        for call in calls {
                                            tracing::debug!(
                                                "🔧 Sending ToolCallStarted event for tool: {} (call_id: {})",
                                                call.fn_name,
                                                call.call_id
                                            );
                                            send_event(ResponseEvent::ToolCallStarted {
                                                call_id: call.call_id.clone(),
                                                fn_name: call.fn_name.clone(),
                                                args: call.fn_arguments.clone(),
                                            })
                                            .await;
                                        }

                                        // Execute tools with rule validation
                                        match self_clone.execute_tools_with_rules(calls).await {
                                            Ok((our_responses, needs_continuation)) => {
                                                // Track if we need continuation after ALL tools are done
                                                if needs_continuation {
                                                    tracing::info!(
                                                        "📍 Marking for continuation after tool execution"
                                                    );
                                                    should_continue_after_tools = true;
                                                }

                                                // Log tool executions to constellation tracker
                                                if let Some(tracker) =
                                                    &context.read().await.constellation_tracker
                                                {
                                                    for (call, response) in
                                                        calls.iter().zip(&our_responses)
                                                    {
                                                        use crate::constellation_memory::{
                                                            ConstellationEvent,
                                                            ConstellationEventType,
                                                        };

                                                        // Truncate to approximately 1000 chars total (500 from start, 500 from end)
                                                        let truncated = if response.content.len()
                                                            > 1000
                                                        {
                                                            // Find safe char boundaries
                                                            let start_boundary = response
                                                                .content
                                                                .char_indices()
                                                                .take_while(|(i, _)| *i <= 500)
                                                                .last()
                                                                .map(|(i, c)| i + c.len_utf8())
                                                                .unwrap_or(0);

                                                            let end_start = response
                                                                .content
                                                                .len()
                                                                .saturating_sub(500);
                                                            let end_boundary = response
                                                                .content
                                                                .char_indices()
                                                                .skip_while(|(i, _)| *i < end_start)
                                                                .next()
                                                                .map(|(i, _)| i)
                                                                .unwrap_or(response.content.len());

                                                            let start =
                                                                &response.content[..start_boundary];
                                                            let end =
                                                                &response.content[end_boundary..];
                                                            format!("{}\n...\n{}", start, end)
                                                        } else {
                                                            response.content.clone()
                                                        };

                                                        // Special handling for send_message to show the actual message
                                                        let action = if call.fn_name
                                                            == "send_message"
                                                        {
                                                            // Try to extract the message content from the tool call
                                                            if let Ok(params) =
                                                                serde_json::from_value::<
                                                                    serde_json::Value,
                                                                >(
                                                                    call.fn_arguments.clone()
                                                                )
                                                            {
                                                                let message_text = params
                                                                    .get("content")
                                                                    .and_then(|m| m.as_str())
                                                                    .unwrap_or("<no content>");

                                                                // Extract target info
                                                                let target_info = if let Some(
                                                                    target,
                                                                ) =
                                                                    params.get("target")
                                                                {
                                                                    if let Some(target_type) =
                                                                        target
                                                                            .get("target_type")
                                                                            .and_then(|t| {
                                                                                t.as_str()
                                                                            })
                                                                    {
                                                                        let target_id = target
                                                                            .get("target_id")
                                                                            .and_then(|i| {
                                                                                i.as_str()
                                                                            });
                                                                        match target_type {
                                                                            "user" => " to user"
                                                                                .to_string(),
                                                                            "agent" => {
                                                                                if let Some(id) =
                                                                                    target_id
                                                                                {
                                                                                    format!(
                                                                                        " to agent: {}",
                                                                                        id
                                                                                    )
                                                                                } else {
                                                                                    " to agent"
                                                                                        .to_string()
                                                                                }
                                                                            }
                                                                            "group" => {
                                                                                if let Some(id) =
                                                                                    target_id
                                                                                {
                                                                                    format!(
                                                                                        " to group: {}",
                                                                                        id
                                                                                    )
                                                                                } else {
                                                                                    " to group"
                                                                                        .to_string()
                                                                                }
                                                                            }
                                                                            "channel" => {
                                                                                if let Some(id) =
                                                                                    target_id
                                                                                {
                                                                                    format!(
                                                                                        " to channel: {}",
                                                                                        id
                                                                                    )
                                                                                } else {
                                                                                    " to channel"
                                                                                        .to_string()
                                                                                }
                                                                            }
                                                                            "bluesky" => {
                                                                                " to bluesky"
                                                                                    .to_string()
                                                                            }
                                                                            _ => format!(
                                                                                " to {}",
                                                                                target_type
                                                                            ),
                                                                        }
                                                                    } else {
                                                                        "".to_string()
                                                                    }
                                                                } else {
                                                                    "".to_string()
                                                                };

                                                                format!(
                                                                    "Sent{}: \"{}\"",
                                                                    target_info, message_text
                                                                )
                                                            } else {
                                                                format!("Success")
                                                            }
                                                        } else if response
                                                            .content
                                                            .starts_with("Error:")
                                                        {
                                                            format!("Failed: {}", truncated)
                                                        } else {
                                                            format!("Success")
                                                        };

                                                        let event = ConstellationEvent {
                                                            timestamp: chrono::Utc::now(),
                                                            agent_id: agent_id.clone(),
                                                            agent_name: self_clone.name().to_string(),
                                                            event_type: ConstellationEventType::ToolExecuted {
                                                                tool_name: call.fn_name.clone(),
                                                                action,
                                                            },
                                                            description: format!("Executed tool: {}", call.fn_name),
                                                            metadata: None,
                                                        };

                                                        tracing::debug!(
                                                            "Adding tool execution event to constellation tracker"
                                                        );
                                                        tracker.add_event(event).await;
                                                    }
                                                }

                                                // Send completion events for each response
                                                for response in &our_responses {
                                                    let tool_result =
                                                        if response.content.starts_with("Error:")
                                                            || response
                                                                .content
                                                                .contains("rule violation")
                                                        {
                                                            Err(response.content.clone())
                                                        } else {
                                                            Ok(response.content.clone())
                                                        };

                                                    send_event(ResponseEvent::ToolCallCompleted {
                                                        call_id: response.call_id.clone(),
                                                        result: tool_result,
                                                    })
                                                    .await;
                                                }

                                                // Add responses to processed response
                                                if !our_responses.is_empty() {
                                                    processed_response.content.push(
                                                        MessageContent::ToolResponses(
                                                            our_responses,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                crate::log_error_chain!(
                                                    "Failed to execute tools with rules: ",
                                                    e
                                                );
                                                // Create error responses for all calls
                                                let error_responses: Vec<ToolResponse> = calls
                                                    .iter()
                                                    .map(|call| ToolResponse {
                                                        call_id: call.call_id.clone(),
                                                        content: format!(
                                                            "Tool execution failed: {:?}",
                                                            e
                                                        ),
                                                        is_error: Some(true),
                                                    })
                                                    .collect();

                                                if !error_responses.is_empty() {
                                                    processed_response.content.push(
                                                        MessageContent::ToolResponses(
                                                            error_responses,
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            MessageContent::ToolResponses(_) => {
                                // Ignore them here, because we have already handled them
                                // Either they were pre-existing and added when we peeked at them,
                                // or we literally just added them after executing the tools.
                            }
                            MessageContent::Text(text) => {
                                // Accumulate text for constellation logging
                                if !agent_response_text.is_empty() {
                                    agent_response_text.push('\n');
                                }
                                agent_response_text.push_str(text);

                                send_event(ResponseEvent::TextChunk {
                                    text: text.clone(),
                                    is_final: true,
                                })
                                .await;
                                processed_response.content.push(content.clone());
                            }
                            MessageContent::Parts(parts) => {
                                for part in parts {
                                    match part {
                                        ContentPart::Text(text) => {
                                            // Accumulate text for constellation logging
                                            if !agent_response_text.is_empty() {
                                                agent_response_text.push('\n');
                                            }
                                            agent_response_text.push_str(text);

                                            send_event(ResponseEvent::TextChunk {
                                                text: text.clone(),
                                                is_final: false,
                                            })
                                            .await;
                                        }
                                        ContentPart::Image {
                                            content_type,
                                            source,
                                        } => {
                                            let source_string = match source {
                                                ImageSource::Url(url) => url.clone(),
                                                ImageSource::Base64(_) => {
                                                    "base64-encoded image".to_string()
                                                }
                                            };
                                            send_event(ResponseEvent::TextChunk {
                                                text: format!(
                                                    "Image ({}): {}",
                                                    content_type, source_string
                                                ),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                    }
                                }

                                processed_response.content.push(content.clone());
                            }
                            MessageContent::Blocks(blocks) => {
                                // Extract tool calls from blocks if any
                                let mut block_tool_calls = Vec::new();
                                let mut other_blocks = Vec::new();

                                for block in blocks {
                                    match block {
                                        ContentBlock::Text { text, .. } => {
                                            // Accumulate text for constellation logging
                                            if !agent_response_text.is_empty() {
                                                agent_response_text.push('\n');
                                            }
                                            agent_response_text.push_str(text);

                                            send_event(ResponseEvent::TextChunk {
                                                text: text.clone(),
                                                is_final: false,
                                            })
                                            .await;
                                            other_blocks.push(block.clone());
                                        }
                                        ContentBlock::Thinking { text, .. } => {
                                            agent_response_text.push_str("\n[Reasoning]:\n");
                                            agent_response_text.push_str(&text.to_lowercase());
                                            send_event(ResponseEvent::ReasoningChunk {
                                                text: text.clone(),
                                                is_final: false,
                                            })
                                            .await;
                                            other_blocks.push(block.clone());
                                        }
                                        ContentBlock::RedactedThinking { .. } => {
                                            other_blocks.push(block.clone());
                                        }
                                        ContentBlock::ToolUse {
                                            id, name, input, ..
                                        } => {
                                            // Convert to ToolCall
                                            let tool_call = ToolCall {
                                                call_id: id.clone(),
                                                fn_name: name.clone(),
                                                fn_arguments: input.clone(),
                                            };
                                            other_blocks.push(block.clone());
                                            block_tool_calls.push(tool_call);
                                        }
                                        ContentBlock::ToolResult { .. } => {
                                            // Tool results shouldn't appear in assistant responses
                                            tracing::warn!(
                                                "Unexpected tool result in assistant message"
                                            );
                                            other_blocks.push(block.clone());
                                        }
                                    }
                                }

                                // If we found tool calls in blocks, execute them
                                if !block_tool_calls.is_empty() {
                                    // Emit tool call event
                                    send_event(ResponseEvent::ToolCalls {
                                        calls: block_tool_calls.clone(),
                                    })
                                    .await;

                                    // Send individual tool call started events
                                    for call in &block_tool_calls {
                                        // Only log if not send_message or if send_message failed
                                        if call.fn_name != "send_message" {
                                            tracing::debug!(
                                                "🔧 Sending ToolCallStarted event for block tool: {} (call_id: {})",
                                                call.fn_name,
                                                call.call_id
                                            );
                                        }
                                        send_event(ResponseEvent::ToolCallStarted {
                                            call_id: call.call_id.clone(),
                                            fn_name: call.fn_name.clone(),
                                            args: call.fn_arguments.clone(),
                                        })
                                        .await;
                                    }

                                    // Execute the tools with rule validation and heartbeat support
                                    let tool_responses = match self_clone
                                        .execute_tools_with_rules(&block_tool_calls)
                                        .await
                                    {
                                        Ok((responses, needs_continuation)) => {
                                            // Track if we need continuation after ALL tools are done
                                            if needs_continuation {
                                                tracing::info!(
                                                    "📍 Marking for continuation after ContentBlock tool execution"
                                                );
                                                should_continue_after_tools = true;
                                            }
                                            responses
                                        }
                                        Err(e) => {
                                            // If execution fails, create error responses for all tools
                                            block_tool_calls
                                                .iter()
                                                .map(|call| ToolResponse {
                                                    call_id: call.call_id.clone(),
                                                    content: format!("Error: {:?}", e),
                                                    is_error: Some(true),
                                                })
                                                .collect()
                                        }
                                    };
                                    // Add the non-tool blocks to processed response
                                    if !other_blocks.is_empty() {
                                        processed_response
                                            .content
                                            .push(MessageContent::Blocks(other_blocks));
                                    }
                                    // Emit tool responses event but don't add to assistant message
                                    // Tool responses should be in a separate message with role Tool
                                    if !tool_responses.is_empty() {
                                        send_event(ResponseEvent::ToolResponses {
                                            responses: tool_responses.clone(),
                                        })
                                        .await;

                                        // Send individual tool call completed events
                                        for response in &tool_responses {
                                            // Log tool completion at debug level
                                            tracing::debug!(
                                                "🔧 Sending ToolCallCompleted event for call_id: {}",
                                                response.call_id
                                            );
                                            send_event(ResponseEvent::ToolCallCompleted {
                                                call_id: response.call_id.clone(),
                                                result: Ok(response.content.clone()),
                                            })
                                            .await;
                                        }

                                        processed_response
                                            .content
                                            .push(MessageContent::ToolResponses(tool_responses));
                                    }
                                } else {
                                    // No tool calls, just add the blocks as-is
                                    processed_response.content.push(content.clone());
                                }
                            }
                        }
                    }

                    // Persist any memory changes from tool execution
                    if let Err(e) = self_clone.persist_memory_changes().await {
                        send_event(ResponseEvent::Error {
                            message: format!("Failed to persist response: {:?}", e),
                            recoverable: true,
                        })
                        .await;
                    }

                    // Persist response
                    if let Err(e) = self_clone
                        .persist_response_messages(
                            &processed_response,
                            &agent_id,
                            current_batch_id,
                            current_batch_type,
                        )
                        .await
                    {
                        send_event(ResponseEvent::Error {
                            message: format!("Failed to persist response: {:?}", e),
                            recoverable: true,
                        })
                        .await;
                    }

                    // Check if we should continue (either unpaired tool calls or continuation requested)
                    if !has_unpaired_tool_calls && !should_continue_after_tools {
                        break;
                    }

                    // If we're continuing due to tools requesting it, reset the flag
                    if should_continue_after_tools && !has_unpaired_tool_calls {
                        tracing::info!(
                            "💓 Continuing conversation loop due to tool continuation request"
                        );
                        should_continue_after_tools = false;
                    }

                    // Add tool rules from rule engine before rebuilding
                    {
                        let tool_rules = self.get_context_tool_rules().await;
                        let consent_tools = self.get_consent_required_tools().await;
                        let mut ctx = context.write().await;
                        ctx.add_tool_rules(tool_rules);
                        ctx.context_config.consent_required_tools = consent_tools;

                        // // Determine role based on vendor
                        // let role = match model_vendor {
                        //     Some(vendor) if vendor.is_openai_compatible() => ChatRole::System,
                        //     Some(crate::model::ModelVendor::Gemini) => ChatRole::User,
                        //     _ => ChatRole::User, // Anthropic and default
                        // };

                        // // Create continuation message in same batch
                        // let content = format!(
                        //     "{}Function call finished, returning control",
                        //     NON_USER_MESSAGE_PREFIX
                        // );
                        // let mut message = match role {
                        //     ChatRole::System => Message::system(content),
                        //     ChatRole::Assistant => Message::agent(content),
                        //     _ => Message::user(content),
                        // };
                        // message.batch = current_batch_id;
                        // let updated_message = ctx.add_message(message).await;

                        // let _ = crate::db::ops::persist_agent_message(
                        //     &self.db,
                        //     &agent_id,
                        //     &updated_message,
                        //     crate::message::MessageRelationType::Active,
                        // )
                        // .await
                        // .inspect_err(|e| {
                        //     crate::log_error!("Failed to persist response message", e);
                        // });
                    }

                    // IMPORTANT: Rebuild context to get fresh memory state after tool execution
                    // This ensures agents see updated memory blocks immediately
                    let context_lock = context.read().await;
                    let memory_context = match context_lock.build_context(current_batch_id).await {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            let error_msg = format!("Failed to rebuild context: {:?}", e);
                            context
                                .read()
                                .await
                                .cleanup_errors(current_batch_id, &error_msg)
                                .await;
                            {
                                let mut ctx = context.write().await;
                                ctx.handle.state = AgentState::Error {
                                    kind: RecoverableErrorKind::ContextBuildFailed,
                                    message: error_msg.clone(),
                                };
                            }
                            send_event(ResponseEvent::Error {
                                message: error_msg,
                                recoverable: false,
                            })
                            .await;
                            break;
                        }
                    };
                    drop(context_lock);

                    let request_with_tools = memory_context.into_request();

                    current_response = {
                        let model = model.read().await;
                        match Self::complete_with_retry(&*model, &options, request_with_tools, 10)
                            .await
                        {
                            Ok(resp) => resp,
                            Err(e) => {
                                // Clean up batch and set error state
                                let error_msg = format!("Model error during continuation: {:?}", e);
                                context
                                    .read()
                                    .await
                                    .cleanup_errors(current_batch_id, &error_msg)
                                    .await;
                                {
                                    let mut ctx = context.write().await;
                                    ctx.handle.state = AgentState::Error {
                                        kind: RecoverableErrorKind::ContextBuildFailed,
                                        message: error_msg.clone(),
                                    };
                                }

                                send_event(ResponseEvent::Error {
                                    message: format!("Model error in continuation: {:?}", e),
                                    recoverable: false,
                                })
                                .await;
                                // Log the full context for debugging
                                crate::log_error!("Model error during continuation", e);
                                tracing::debug!("Full context on error: {:?}", &memory_context);
                                break;
                            }
                        }
                    };

                    if current_response.num_tool_calls() == 0 {
                        // Persist any memory changes from tool execution
                        if let Err(e) = self_clone.persist_memory_changes().await {
                            send_event(ResponseEvent::Error {
                                message: format!("Failed to persist response: {:?}", e),
                                recoverable: true,
                            })
                            .await;
                        }

                        // Persist response
                        if let Err(e) = self_clone
                            .persist_response_messages(
                                &current_response,
                                &agent_id,
                                current_batch_id,
                                current_batch_type,
                            )
                            .await
                        {
                            send_event(ResponseEvent::Error {
                                message: format!("Failed to persist response: {:?}", e),
                                recoverable: true,
                            })
                            .await;
                        }
                        break;
                    }
                } else {
                    // Check if we have thinking blocks - if so, we'll emit reasoning from those instead
                    let has_thinking_blocks = current_response.content.iter().any(|content| {
                        matches!(content, MessageContent::Blocks(blocks) if blocks.iter().any(|b| matches!(b, ContentBlock::Thinking { .. })))
                    });

                    // Only emit aggregated reasoning if we don't have thinking blocks
                    if !has_thinking_blocks {
                        if let Some(reasoning) = &current_response.reasoning {
                            agent_response_text.push_str("[Reasoning]:");
                            agent_response_text.push_str(&reasoning.to_lowercase());
                            send_event(ResponseEvent::ReasoningChunk {
                                text: reasoning.clone(),
                                is_final: true,
                            })
                            .await;
                        }
                    }
                    // No tool calls, emit final content
                    for content in &current_response.content {
                        match content {
                            MessageContent::Text(text) => {
                                // Accumulate text for constellation logging
                                if !agent_response_text.is_empty() {
                                    agent_response_text.push('\n');
                                }
                                agent_response_text.push_str(text);

                                send_event(ResponseEvent::TextChunk {
                                    text: text.clone(),
                                    is_final: true,
                                })
                                .await;
                            }
                            MessageContent::Parts(parts) => {
                                for part in parts {
                                    match part {
                                        ContentPart::Text(text) => {
                                            // Accumulate text for constellation logging
                                            if !agent_response_text.is_empty() {
                                                agent_response_text.push('\n');
                                            }
                                            agent_response_text.push_str(text);

                                            send_event(ResponseEvent::TextChunk {
                                                text: text.clone(),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                        ContentPart::Image {
                                            content_type,
                                            source,
                                        } => {
                                            let source_string = match source {
                                                ImageSource::Url(url) => url.clone(),
                                                ImageSource::Base64(_) => {
                                                    "base64-encoded image".to_string()
                                                }
                                            };
                                            send_event(ResponseEvent::TextChunk {
                                                text: format!(
                                                    "Image ({}): {}",
                                                    content_type, source_string
                                                ),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                    }
                                }
                            }
                            MessageContent::Blocks(blocks) => {
                                // Process blocks in sequence
                                for block in blocks {
                                    match block {
                                        ContentBlock::Text { text, .. } => {
                                            // Accumulate text for constellation logging
                                            if !agent_response_text.is_empty() {
                                                agent_response_text.push('\n');
                                            }
                                            agent_response_text.push_str(text);

                                            send_event(ResponseEvent::TextChunk {
                                                text: text.clone(),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                        ContentBlock::Thinking { text, .. } => {
                                            // Thinking content is already handled via reasoning_content
                                            // but we could emit it as a ReasoningChunk if not already done
                                            agent_response_text.push_str("\n[Reasoning]:\n");
                                            agent_response_text.push_str(&text.to_lowercase());
                                            send_event(ResponseEvent::ReasoningChunk {
                                                text: text.clone(),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                        ContentBlock::RedactedThinking { .. } => {
                                            // Skip redacted thinking in the stream
                                        }
                                        ContentBlock::ToolUse {
                                            id, name, input, ..
                                        } => {
                                            // Convert to ToolCall and emit
                                            let tool_call = ToolCall {
                                                call_id: id.clone(),
                                                fn_name: name.clone(),
                                                fn_arguments: input.clone(),
                                            };
                                            send_event(ResponseEvent::ToolCalls {
                                                calls: vec![tool_call],
                                            })
                                            .await;
                                        }
                                        ContentBlock::ToolResult { .. } => {
                                            // Tool results shouldn't appear in assistant messages
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Persist final response
                    let persist_result = self_clone
                        .persist_response_messages(
                            &current_response,
                            &agent_id,
                            current_batch_id,
                            current_batch_type,
                        )
                        .await;
                    if let Err(e) = persist_result {
                        send_event(ResponseEvent::Error {
                            message: format!("Failed to persist final response: {:?}", e),
                            recoverable: true,
                        })
                        .await;
                    }

                    break;
                }
            }

            // Check if we exited the loop with pending continuation request
            if should_continue_after_tools {
                // CRITICAL: Persist ALL state before sending async heartbeat
                tracing::info!("💾 Persisting all state before async continuation");

                // 1. Persist memory changes
                if let Err(e) = self_clone.persist_memory_changes().await {
                    tracing::error!("Failed to persist memory before heartbeat: {:?}", e);
                }

                // Get next sequence number from current batch
                let next_seq_num = {
                    let ctx = context.read().await;
                    let history = ctx.history.read().await;
                    current_batch_id.and_then(|batch_id| {
                        history
                            .batches
                            .iter()
                            .find(|b| b.id == batch_id)
                            .map(|b| b.next_sequence_num())
                    })
                };

                // NOW send the single heartbeat for background processing
                use crate::context::heartbeat::HeartbeatRequest;
                let heartbeat_req = HeartbeatRequest {
                    agent_id: agent_id.clone(),
                    tool_name: "continuation".to_string(),
                    tool_call_id: format!("cont_{}", uuid::Uuid::new_v4().simple()),
                    batch_id: current_batch_id,
                    next_sequence_num: next_seq_num,
                    model_vendor,
                };

                if let Err(e) = self_clone.heartbeat_sender.try_send(heartbeat_req) {
                    tracing::warn!("Failed to send async heartbeat: {:?}", e);
                }
            } else {
                // Not continuing - mark the batch as complete
                if let Some(batch_id) = current_batch_id {
                    let ctx = context.write().await;
                    let mut history = ctx.history.write().await;
                    if let Some(batch) = history.batches.iter_mut().find(|b| b.id == batch_id) {
                        batch.finalize();
                        batch.mark_complete();
                        tracing::info!("✅ Marked batch {} as complete", batch_id);
                    }
                }
            }

            // Update stats and complete
            let stats = context.read().await.get_stats().await;
            let _ = crate::db::ops::update_agent_stats(&db, agent_id.clone(), &stats).await;

            // Execute required exit tools before completing
            match self_clone.execute_required_exit_tools().await {
                Ok(exit_responses) => {
                    if !exit_responses.is_empty() {
                        tracing::info!("✅ Executed {} required exit tools", exit_responses.len());
                        // Emit events for exit tool execution
                        send_event(ResponseEvent::ToolResponses {
                            responses: exit_responses,
                        })
                        .await;
                    }
                }
                Err(e) => {
                    crate::log_error!("❌ Failed to execute required exit tools: ", e);
                    send_event(ResponseEvent::Error {
                        message: format!("Required exit tools failed: {:?}", e),
                        recoverable: true, // Don't fail the whole conversation
                    })
                    .await;
                }
            }

            // Remove batch from active set and reset state if no other batches
            {
                let mut ctx = context.write().await;

                // Remove current batch from active set
                if let AgentState::Processing {
                    ref mut active_batches,
                } = ctx.handle.state
                {
                    if let Some(batch_id) = current_batch_id {
                        active_batches.remove(&batch_id);
                    }

                    // If no more active batches, set to Ready
                    if active_batches.is_empty() {
                        ctx.handle.state = AgentState::Ready;
                    }
                } else {
                    // Fallback to Ready if not in Processing state
                    ctx.handle.state = AgentState::Ready;
                }
            }

            // Log to constellation activity tracker if present
            if let Some(tracker) = &context.read().await.constellation_tracker {
                tracing::debug!("Constellation tracker present, logging activity");
                use crate::constellation_memory::{ConstellationEvent, ConstellationEventType};

                // Create summary combining user message and agent response
                let summary = if let Some(user_msg) = incoming_message_summary {
                    if !agent_response_text.is_empty() {
                        // Truncate agent response if needed
                        let agent_summary = if agent_response_text.len() > 100 {
                            format!("{}...", &agent_response_text[..100])
                        } else {
                            agent_response_text.clone()
                        };
                        format!("User: {}\nAgent: {}", user_msg, agent_summary)
                    } else {
                        user_msg
                    }
                } else if !agent_response_text.is_empty() {
                    // Just agent response
                    if agent_response_text.len() > 200 {
                        format!("Agent: {}...", &agent_response_text[..200])
                    } else {
                        format!("Agent: {}", agent_response_text)
                    }
                } else {
                    format!("Processed {} message", incoming_message_role)
                };

                let event = ConstellationEvent {
                    timestamp: chrono::Utc::now(),
                    agent_id: agent_id.clone(),
                    agent_name: self_clone.name().to_string(),
                    event_type: ConstellationEventType::MessageProcessed { summary },
                    description: format!("Completed processing {} message", incoming_message_role),
                    metadata: None,
                };

                tracing::debug!("Adding message processed event to constellation tracker");
                tracker.add_event(event).await;
            }

            // Clean up temporarily loaded memory blocks that are not pinned
            self_clone
                .cleanup_temporary_memory_blocks(&loaded_memory_labels)
                .await;

            // Send completion event with the incoming message ID
            send_event(ResponseEvent::Complete {
                message_id: incoming_message_id,
                metadata: current_response.metadata,
            })
            .await;
        });

        Ok(ReceiverStream::new(rx))
    }
}

#[cfg(test)]
mod tool_rules_integration_tests {
    use crate::agent::tool_rules::ToolRule;
    use std::time::Duration;

    #[tokio::test]
    async fn test_tool_rules_initialization() {
        // Create tool rules for testing
        let tool_rules = vec![
            // Start constraint - context tool must be called first
            ToolRule::start_constraint("context".to_string()),
            // Continue loop - search tool doesn't need heartbeat
            ToolRule::continue_loop("search".to_string()),
            // Exit loop - send_message should end conversation
            ToolRule::exit_loop("send_message".to_string()),
            // Required before exit - recall should be called before ending
            ToolRule::required_before_exit("recall".to_string()),
            // Max calls - limit search to 3 calls
            ToolRule::max_calls("search".to_string(), 3),
            // Cooldown - wait 1 second between recall calls
            ToolRule::cooldown("recall".to_string(), Duration::from_secs(1)),
        ];

        // Create rule engine and test basic functionality
        let rule_engine = crate::agent::tool_rules::ToolRuleEngine::new(tool_rules);

        // Test heartbeat requirements
        assert!(!rule_engine.requires_heartbeat("search")); // ContinueLoop rule
        assert!(rule_engine.requires_heartbeat("recall")); // No rule, default behavior

        // Test start constraint tools
        let start_tools = rule_engine.get_start_constraint_tools();
        assert_eq!(start_tools, vec!["context"]);

        // Test required exit tools
        let exit_tools = rule_engine.get_required_before_exit_tools();
        assert_eq!(exit_tools, vec!["recall"]);

        println!("✅ Tool rules initialization test passed");
    }

    #[tokio::test]
    async fn test_rule_validation_logic() {
        let tool_rules = vec![
            ToolRule::max_calls("limited_tool".to_string(), 2),
            ToolRule::cooldown("slow_tool".to_string(), Duration::from_millis(100)),
        ];

        let mut rule_engine = crate::agent::tool_rules::ToolRuleEngine::new(tool_rules);

        // Test max calls enforcement
        assert!(rule_engine.can_execute_tool("limited_tool").is_ok());

        // Record first execution
        rule_engine.record_execution(crate::agent::tool_rules::ToolExecution {
            tool_name: "limited_tool".to_string(),
            call_id: "call_1".to_string(),
            timestamp: std::time::Instant::now(),
            success: true,
            metadata: None,
        });

        // Should still allow second call
        assert!(rule_engine.can_execute_tool("limited_tool").is_ok());

        // Record second execution
        rule_engine.record_execution(crate::agent::tool_rules::ToolExecution {
            tool_name: "limited_tool".to_string(),
            call_id: "call_2".to_string(),
            timestamp: std::time::Instant::now(),
            success: true,
            metadata: None,
        });

        // Should reject third call (max calls exceeded)
        assert!(rule_engine.can_execute_tool("limited_tool").is_err());

        println!("✅ Rule validation logic test passed");
    }
}

/// Builder for creating DatabaseAgent instances with fluent configuration
pub struct DatabaseAgentBuilder<M, E> {
    agent_id: Option<AgentId>,
    user_id: Option<UserId>,
    agent_type: Option<AgentType>,
    name: Option<String>,
    system_prompt: Option<String>,
    memory: Option<Memory>,
    db: Option<Surreal<surrealdb::engine::any::Any>>,
    model: Option<Arc<RwLock<M>>>,
    tools: Option<ToolRegistry>,
    embeddings: Option<Arc<E>>,
    heartbeat_sender: Option<HeartbeatSender>,
    tool_rules: Vec<ToolRule>,
}

impl<M, E> DatabaseAgentBuilder<M, E>
where
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            agent_id: None,
            user_id: None,
            agent_type: None,
            name: None,
            system_prompt: None,
            memory: None,
            db: None,
            model: None,
            tools: None,
            embeddings: None,
            heartbeat_sender: None,
            tool_rules: Vec::new(),
        }
    }

    /// Set the agent ID
    pub fn with_agent_id(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the user ID
    pub fn with_user_id(mut self, user_id: UserId) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Set the agent type
    pub fn with_agent_type(mut self, agent_type: AgentType) -> Self {
        self.agent_type = Some(agent_type);
        self
    }

    /// Set the agent name
    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    /// Set the system prompt
    pub fn with_system_prompt(mut self, system_prompt: String) -> Self {
        self.system_prompt = Some(system_prompt);
        self
    }

    /// Set the memory
    pub fn with_memory(mut self, memory: Memory) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Set the database connection
    pub fn with_db(mut self, db: Surreal<surrealdb::engine::any::Any>) -> Self {
        self.db = Some(db);
        self
    }

    /// Set the model provider
    pub fn with_model(mut self, model: Arc<RwLock<M>>) -> Self {
        self.model = Some(model);
        self
    }

    /// Set the tool registry
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the embedding provider
    pub fn with_embeddings(mut self, embeddings: Arc<E>) -> Self {
        self.embeddings = Some(embeddings);
        self
    }

    /// Set the heartbeat sender
    pub fn with_heartbeat_sender(mut self, heartbeat_sender: HeartbeatSender) -> Self {
        self.heartbeat_sender = Some(heartbeat_sender);
        self
    }

    /// Add a single tool rule
    pub fn with_tool_rule(mut self, rule: ToolRule) -> Self {
        self.tool_rules.push(rule);
        self
    }

    /// Add multiple tool rules
    pub fn with_tool_rules(mut self, rules: Vec<ToolRule>) -> Self {
        self.tool_rules.extend(rules);
        self
    }

    /// Load tool rules from configuration for the specified agent name
    pub async fn with_tool_rules_from_config(mut self, agent_name: &str) -> Result<Self> {
        let config = crate::config::PatternConfig::load().await?;
        let rules = config.get_agent_tool_rules(agent_name)?;
        self.tool_rules.extend(rules);
        Ok(self)
    }

    /// Add common tool rules for performance optimization
    pub fn with_performance_rules(mut self) -> Self {
        use crate::agent::tool_rules::ToolRule;

        // Add common fast tools that don't need heartbeats
        let fast_tools = vec![
            "search",
            "context",
            "recall",
            "calculate",
            "format_text",
            "validate_json",
        ];

        for tool in fast_tools {
            self.tool_rules
                .push(ToolRule::continue_loop(tool.to_string()));
        }

        self
    }

    /// Add common ETL workflow rules
    pub fn with_etl_rules(mut self) -> Self {
        use crate::agent::tool_rules::ToolRule;
        use std::time::Duration;

        self.tool_rules.extend(vec![
            ToolRule::start_constraint("connect_database".to_string()),
            ToolRule::requires_preceding_tools(
                "validate_data".to_string(),
                vec!["extract_data".to_string()],
            ),
            ToolRule::requires_preceding_tools(
                "transform_data".to_string(),
                vec!["validate_data".to_string()],
            ),
            ToolRule::exit_loop("load_to_warehouse".to_string()),
            ToolRule::required_before_exit("close_connections".to_string()),
            ToolRule::max_calls("api_request".to_string(), 10),
            ToolRule::cooldown("heavy_compute".to_string(), Duration::from_secs(2)),
        ]);

        self
    }

    /// Build the DatabaseAgent
    pub fn build(self) -> Result<DatabaseAgent<M, E>> {
        let agent_id = self.agent_id.unwrap_or_else(AgentId::generate);
        let user_id = self
            .user_id
            .ok_or_else(|| crate::CoreError::AgentInitFailed {
                agent_type: "DatabaseAgent".to_string(),
                cause: "user_id is required".to_string(),
            })?;
        let agent_type = self.agent_type.unwrap_or(AgentType::Generic);
        let name = self.name.unwrap_or_else(|| "Agent".to_string());
        let system_prompt = self.system_prompt.unwrap_or_default();
        let memory = self.memory.unwrap_or_else(|| Memory::with_owner(&user_id));
        let db = self.db.ok_or_else(|| crate::CoreError::AgentInitFailed {
            agent_type: "DatabaseAgent".to_string(),
            cause: "database connection is required".to_string(),
        })?;
        let model = self
            .model
            .ok_or_else(|| crate::CoreError::AgentInitFailed {
                agent_type: "DatabaseAgent".to_string(),
                cause: "model provider is required".to_string(),
            })?;
        let tools = self.tools.unwrap_or_else(ToolRegistry::new);
        let heartbeat_sender =
            self.heartbeat_sender
                .ok_or_else(|| crate::CoreError::AgentInitFailed {
                    agent_type: "DatabaseAgent".to_string(),
                    cause: "heartbeat sender is required".to_string(),
                })?;

        Ok(DatabaseAgent::new(
            agent_id,
            user_id,
            agent_type,
            name,
            system_prompt,
            memory,
            db,
            model,
            tools,
            self.embeddings,
            heartbeat_sender,
            self.tool_rules,
        ))
    }
}

impl<M, E> DatabaseAgent<M, E>
where
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    /// Create a new builder for this agent type
    pub fn builder() -> DatabaseAgentBuilder<M, E> {
        DatabaseAgentBuilder::new()
    }
}

impl<M, E> std::fmt::Debug for DatabaseAgent<M, E>
where
    M: ModelProvider,
    E: EmbeddingProvider,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Can't await in Debug, so we'll use try_read
        if let Ok(context) = self.context.try_read() {
            f.debug_struct("DatabaseAgent")
                .field("agent_id", &context.handle.agent_id)
                .field("name", &context.handle.name)
                .field("agent_type", &context.handle.agent_type)
                .field("user_id", &self.user_id)
                .field("state", &context.handle.state)
                .field("has_embeddings", &self.embeddings.is_some())
                .field("memory_blocks", &context.handle.memory.list_blocks().len())
                .finish()
        } else {
            f.debug_struct("DatabaseAgent")
                .field("user_id", &self.user_id)
                .field("has_embeddings", &self.embeddings.is_some())
                .field("<locked>", &"...")
                .finish()
        }
    }
}

#[async_trait]
impl<M, E> Agent for DatabaseAgent<M, E>
where
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    fn id(&self) -> AgentId {
        self.cached_id.clone()
    }

    fn name(&self) -> String {
        self.cached_name.clone()
    }

    fn agent_type(&self) -> AgentType {
        self.cached_agent_type.clone()
    }

    async fn handle(&self) -> crate::context::state::AgentHandle {
        self.handle().await
    }

    async fn last_active(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        // Get the last_active timestamp from the agent context metadata
        let context = self.context.read().await;
        let metadata = context.metadata.read().await;
        Some(metadata.last_active)
    }

    async fn process_message(self: Arc<Self>, message: Message) -> Result<Response> {
        use crate::message::ResponseMetadata;
        use futures::StreamExt;

        let mut stream = DatabaseAgent::process_message_stream(self.clone(), message).await?;

        // Collect all events from the stream
        let mut content = Vec::new();
        let mut reasoning: Option<String> = None;
        let mut metadata = ResponseMetadata::default();
        let mut has_error = false;

        while let Some(event) = stream.next().await {
            match event {
                ResponseEvent::TextChunk { text, is_final: _ } => {
                    // Append text to response
                    if let Some(MessageContent::Text(existing_text)) = content.last_mut() {
                        existing_text.push_str(&text);
                    } else {
                        content.push(MessageContent::Text(text));
                    }
                }
                ResponseEvent::ReasoningChunk { text, is_final: _ } => {
                    // Append reasoning
                    if let Some(ref mut existing_reasoning) = reasoning {
                        existing_reasoning.push_str(&text);
                    } else {
                        reasoning = Some(text);
                    }
                }
                ResponseEvent::ToolCalls { calls } => {
                    content.push(MessageContent::ToolCalls(calls));
                }
                ResponseEvent::ToolResponses { responses } => {
                    content.push(MessageContent::ToolResponses(responses));
                }
                ResponseEvent::Complete {
                    message_id: _,
                    metadata: event_metadata,
                } => {
                    metadata = event_metadata;
                }
                ResponseEvent::Error { recoverable, .. } => {
                    if !recoverable {
                        has_error = true;
                    }
                }
                _ => {} // Ignore other events for backward compatibility
            }
        }

        if has_error {
            Err(CoreError::from_genai_error(
                "genai",
                "",
                genai::Error::NoChatResponse {
                    model_iden: metadata.model_iden,
                },
            ))
        } else {
            Ok(Response {
                content,
                reasoning,
                metadata,
            })
        }
    }

    async fn get_memory(&self, key: &str) -> Result<Option<MemoryBlock>> {
        let context = self.context.read().await;
        Ok(context.handle.memory.get_block(key).map(|b| b.clone()))
    }

    async fn update_memory(&self, key: &str, memory: MemoryBlock) -> Result<()> {
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };
        tracing::debug!("🔧 Agent {} updating memory key '{}'", agent_id, key);

        // Update in context immediately - upsert the complete block
        {
            let context = self.context.write().await;
            context.handle.memory.upsert_block(key, memory.clone())?;
            context.metadata.write().await.last_active = Utc::now();
        };

        // Persist memory block to database
        tracing::debug!("persisting memory key {}", key);

        // Retry logic for concurrent upserts
        let mut attempts = 0;
        const MAX_RETRIES: u32 = 3;

        loop {
            match crate::db::ops::persist_agent_memory(
                &self.db,
                agent_id.clone(),
                &memory,
                crate::memory::MemoryPermission::ReadWrite, // Agent has full access to its own memory
            )
            .await
            {
                Ok(_) => break,
                Err(e) => {
                    // Check if it's a duplicate key error in the full-text search index
                    if let crate::db::DatabaseError::QueryFailed(ref surreal_err) = e {
                        let error_str = surreal_err.to_string();
                        if error_str.contains("Duplicate insert key")
                            && error_str.contains("mem_value_search")
                        {
                            attempts += 1;
                            if attempts < MAX_RETRIES {
                                tracing::warn!(
                                    "Duplicate key error updating memory '{}' (attempt {}/{}), retrying...",
                                    key,
                                    attempts,
                                    MAX_RETRIES
                                );
                                // Small delay with jitter to reduce contention
                                let delay_ms = 50u64 + (attempts as u64 * 100);
                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms))
                                    .await;
                                continue;
                            }
                        }
                    }
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Get the tool from the registry
        let (tool, agent_id) = {
            let context = self.context.read().await;
            let tool = context.tools.get(tool_name).ok_or_else(|| {
                CoreError::tool_not_found(
                    tool_name,
                    context
                        .tools
                        .list_tools()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
            })?;
            (tool.clone(), context.handle.agent_id.clone())
        };

        // Record start time
        let start_time = std::time::Instant::now();
        let created_at = chrono::Utc::now();

        // Execute the tool with minimal meta (no consent yet in this path)
        let meta = crate::tool::ExecutionMeta {
            permission_grant: None,
            request_heartbeat: params
                .get("request_heartbeat")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            caller_user: None,
            call_id: None,
            route_metadata: None,
        };
        let mut params_clean = params.clone();
        if let serde_json::Value::Object(ref mut map) = params_clean {
            map.remove("request_heartbeat");
        }
        let result = tool.execute(params_clean, &meta).await;

        // Calculate duration
        let duration_ms = start_time.elapsed().as_millis() as i64;

        // Log memory operations to constellation tracker if this is a memory tool
        if matches!(tool_name, "context" | "recall") && result.is_ok() {
            let context = self.context.read().await;
            if let Some(tracker) = &context.constellation_tracker {
                use crate::constellation_memory::{
                    ConstellationEvent, ConstellationEventType, MemoryChangeType,
                };
                use crate::tool::builtin::{
                    ArchivalMemoryOperationType, ContextInput, CoreMemoryOperationType, RecallInput,
                };

                // Try to extract operation details from the result
                if let Ok(result_value) = &result {
                    if let Some(success) = result_value.get("success").and_then(|v| v.as_bool()) {
                        if success {
                            // Determine memory operation type from params
                            let event_opt = if tool_name == "context" {
                                // Context tool operations
                                if let Ok(input) =
                                    serde_json::from_value::<ContextInput>(params.clone())
                                {
                                    let label = input
                                        .name
                                        .or(input.archival_label)
                                        .unwrap_or_else(|| "unknown".to_string());
                                    let change_type = match input.operation {
                                        CoreMemoryOperationType::Append
                                        | CoreMemoryOperationType::Replace => {
                                            MemoryChangeType::Updated
                                        }
                                        CoreMemoryOperationType::Archive => {
                                            MemoryChangeType::Archived
                                        }
                                        CoreMemoryOperationType::Load => MemoryChangeType::Created,
                                        CoreMemoryOperationType::Swap => MemoryChangeType::Updated,
                                    };
                                    Some((label, change_type))
                                } else {
                                    None
                                }
                            } else {
                                // Recall tool operations
                                if let Ok(input) =
                                    serde_json::from_value::<RecallInput>(params.clone())
                                {
                                    let label = input
                                        .label
                                        .clone()
                                        .unwrap_or_else(|| "unknown".to_string());
                                    match input.operation {
                                        ArchivalMemoryOperationType::Insert => {
                                            Some((label, MemoryChangeType::Created))
                                        }
                                        ArchivalMemoryOperationType::Append => {
                                            Some((label, MemoryChangeType::Updated))
                                        }
                                        ArchivalMemoryOperationType::Read => None, // Skip logging read operations
                                        ArchivalMemoryOperationType::Delete => {
                                            Some((label, MemoryChangeType::Deleted))
                                        }
                                    }
                                } else {
                                    None
                                }
                            };

                            if let Some((memory_label, change_type)) = event_opt {
                                let event = ConstellationEvent {
                                    timestamp: chrono::Utc::now(),
                                    agent_id: agent_id.clone(),
                                    agent_name: self.name().to_string(),
                                    event_type: ConstellationEventType::MemoryUpdated {
                                        memory_label,
                                        change_type,
                                    },
                                    description: format!("Memory operation via {} tool", tool_name),
                                    metadata: None,
                                };

                                tracing::info!(
                                    "Adding memory update event to constellation tracker"
                                );
                                tracker.add_event(event).await;
                            }
                        }
                    }
                }
            }
        }

        // Update tool call count in metadata
        {
            let context = self.context.read().await;
            let mut metadata = context.metadata.write().await;
            metadata.total_tool_calls += 1;
        }

        let tool_call = schema::ToolCall {
            id: crate::id::ToolCallId::generate(),
            agent_id,
            tool_name: tool_name.to_string(),
            parameters: params,
            result: result.as_ref().unwrap_or(&serde_json::json!({})).clone(),
            error: result.as_ref().err().map(|e| e.to_string()),
            duration_ms,
            created_at,
        };

        let _ = crate::db::ops::create_entity(&self.db, &tool_call)
            .await
            .inspect_err(|e| {
                crate::log_error!("Failed to persist tool call", e);
            });

        // Return the result
        result
    }

    async fn list_memory_keys(&self) -> Result<Vec<CompactString>> {
        let context = self.context.read().await;
        Ok(context.handle.memory.list_blocks())
    }

    async fn share_memory_with(
        &self,
        memory_key: &str,
        target_agent_id: AgentId,
        access_level: crate::memory::MemoryPermission,
    ) -> Result<()> {
        // First verify the memory block exists
        let memory_block = {
            let context = self.context.read().await;
            context
                .handle
                .memory
                .get_block(memory_key)
                .ok_or_else(|| {
                    CoreError::memory_not_found(
                        &context.handle.agent_id,
                        memory_key,
                        context.handle.memory.list_blocks(),
                    )
                })?
                .clone()
        };

        // Create the memory relation for the target agent
        let db = self.db.clone();
        let memory_id = memory_block.id.clone();
        let relation = AgentMemoryRelation {
            id: RelationId::nil(),
            in_id: target_agent_id.clone(),
            out_id: memory_id.clone(),
            access_level,
            created_at: chrono::Utc::now(),
        };

        // Persist the sharing relationship in background
        let target_id_clone = target_agent_id.clone();
        let _handle = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::create_relation_typed(&db, &relation).await {
                crate::log_error!("Failed to share memory block", e);
            } else {
                tracing::debug!(
                    "Shared memory block {} with agent {} (access: {:?})",
                    memory_id,
                    target_id_clone,
                    access_level
                );
            }
        });

        Ok(())
    }

    async fn get_shared_memories(&self) -> Result<Vec<(AgentId, CompactString, MemoryBlock)>> {
        let db = self.db.clone();
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };

        // Query for all memory blocks shared with this agent
        let query = r#"
            SELECT
                out AS memory_block,
                in AS source_agent_id,
                access_level
            FROM agent_memories
            WHERE in = $agent_id
            FETCH out, in
        "#;

        let mut response = db
            .query(query)
            .bind(("agent_id", surrealdb::RecordId::from(&agent_id)))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Extract the shared memory relationships
        let results: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let mut shared_memories = Vec::new();

        for result in results {
            if let (Some(memory_value), Some(source_id)) =
                (result.get("memory_block"), result.get("source_agent_id"))
            {
                // Parse the memory block
                let memory_block: MemoryBlock = serde_json::from_value(memory_value.clone())
                    .map_err(|e| DatabaseError::SerdeProblem(e))?;

                // Parse the source agent ID
                let source_agent_id: AgentId = serde_json::from_value(source_id.clone())
                    .map_err(|e| DatabaseError::SerdeProblem(e))?;

                shared_memories.push((source_agent_id, memory_block.label.clone(), memory_block));
            }
        }

        Ok(shared_memories)
    }

    async fn system_prompt(&self) -> Vec<String> {
        // Add workflow rules from the rule engine before building context
        {
            let tool_rules = self.get_context_tool_rules().await;
            let consent_tools = self.get_consent_required_tools().await;
            let mut ctx = self.context.write().await;
            ctx.add_tool_rules(tool_rules);
            ctx.context_config.consent_required_tools = consent_tools;
        }

        let context = self.context.read().await;
        match context.build_context(None).await {
            Ok(memory_context) => {
                // Split the built system prompt into logical sections
                memory_context
                    .system_prompt
                    .split("\n\n")
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            Err(_) => {
                // If context building fails, return base instructions
                vec![context.context_config.base_instructions.clone()]
            }
        }
    }

    async fn available_tools(&self) -> Vec<Box<dyn DynamicTool>> {
        let context = self.context.read().await;
        context.tools.get_all_as_dynamic()
    }

    async fn state(&self) -> (AgentState, Option<tokio::sync::watch::Receiver<AgentState>>) {
        let context = self.context.read().await;
        (
            context.handle.state.clone(),
            context.handle.state_receiver(),
        )
    }

    /// Set the agent's state
    async fn set_state(&self, state: AgentState) -> Result<()> {
        // Update the state in the context
        {
            let mut context = self.context.write().await;
            context.handle.update_state(state.clone());
            let mut metadata = context.metadata.write().await;
            metadata.last_active = chrono::Utc::now();
        }

        // Persist only the last_active timestamp. The 'state' field is no longer stored on AgentRecord.
        let db = self.db.clone();
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };

        let _ = db
            .query("UPDATE agent SET last_active = $last_active WHERE id = $id")
            .bind(("id", surrealdb::RecordId::from(&agent_id)))
            .bind(("last_active", surrealdb::Datetime::from(chrono::Utc::now())))
            .await
            .map_err(|e| {
                crate::log_error!("Failed to persist agent last_active update", e);
                crate::CoreError::from(crate::db::DatabaseError::QueryFailed(e))
            })?;

        Ok(())
    }

    async fn register_endpoint(
        &self,
        name: String,
        endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
    ) -> Result<()> {
        let context = self.context.read().await;

        // Get the message router from the handle
        let router = context.handle.message_router().ok_or_else(|| {
            crate::CoreError::ToolExecutionFailed {
                tool_name: "register_endpoint".to_string(),
                cause: "Message router not configured for this agent".to_string(),
                parameters: serde_json::json!({ "name": name }),
            }
        })?;

        // Register the endpoint
        router.register_endpoint(name, endpoint).await;
        Ok(())
    }

    async fn set_default_user_endpoint(
        &self,
        endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
    ) -> Result<()> {
        let context = self.context.read().await;
        if let Some(router) = &context.handle.message_router {
            router.set_default_user_endpoint(endpoint).await;
            tracing::info!("default endpoint set for {}", self.id());
            Ok(())
        } else {
            tracing::error!("default endpoint for {} failed to set", self.id());
            Err(crate::CoreError::AgentInitFailed {
                agent_type: self.agent_type().as_str().to_string(),
                cause: "Message router not initialized".to_string(),
            })
        }
    }

    async fn process_message_stream(
        self: Arc<Self>,
        message: Message,
    ) -> Result<Box<dyn Stream<Item = ResponseEvent> + Send + Unpin>>
    where
        Self: 'static,
    {
        // Call our actual streaming implementation
        let stream = DatabaseAgent::process_message_stream(self, message).await?;
        Ok(Box::new(stream) as Box<dyn Stream<Item = ResponseEvent> + Send + Unpin>)
    }
}

// Extension trait for database operations
#[async_trait]
pub trait AgentDbExt<C: surrealdb::Connection> {
    async fn create_tool_call(&self, tool_call: schema::ToolCall) -> Result<()>;
}

#[async_trait]
impl<T, C> AgentDbExt<C> for T
where
    T: AsRef<surrealdb::Surreal<C>> + Sync,
    C: surrealdb::Connection,
{
    async fn create_tool_call(&self, tool_call: schema::ToolCall) -> Result<()> {
        // Store the tool call in the database
        crate::db::ops::create_entity(self.as_ref(), &tool_call).await?;
        Ok(())
    }
}
