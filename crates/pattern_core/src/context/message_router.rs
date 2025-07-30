use std::collections::HashMap;
use std::sync::Arc;

use atrium_api::app::bsky::feed::post::{ReplyRef, ReplyRefData};
use atrium_api::com::atproto::repo::strong_ref;
use atrium_api::types::TryFromUnknown;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::Surreal;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::atproto_identity::resolve_handle_to_pds;
use crate::db::client;
use crate::error::Result;
use crate::id::{AgentId, GroupId, UserId};
use crate::message::{ContentPart, Message, MessageContent};
use crate::message_queue::QueuedMessage;
use crate::tool::builtin::{MessageTarget, TargetType};

/// Describes the origin of a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageOrigin {
    /// Data source ingestion
    DataSource {
        source_id: String,
        source_type: String,
        item_id: Option<String>,
        cursor: Option<Value>,
    },

    /// Discord message
    Discord {
        server_id: String,
        channel_id: String,
        user_id: String,
        message_id: String,
    },

    /// CLI interaction
    Cli {
        session_id: String,
        command: Option<String>,
    },

    /// API request
    Api {
        client_id: String,
        request_id: String,
        endpoint: String,
    },

    /// Bluesky/ATProto
    Bluesky {
        handle: String,
        did: String,
        post_uri: Option<String>,
        is_mention: bool,
        is_reply: bool,
    },

    /// Agent-initiated (no external origin)
    Agent { agent_id: AgentId, reason: String },

    /// Other origin types
    Other {
        origin_type: String,
        source_id: String,
        metadata: Value,
    },
}

impl MessageOrigin {
    /// Get a human-readable description of the origin
    pub fn description(&self) -> String {
        match self {
            Self::DataSource {
                source_id,
                source_type,
                ..
            } => format!("Data from {} ({})", source_id, source_type),
            Self::Discord {
                server_id,
                channel_id,
                user_id,
                ..
            } => format!(
                "Discord message from user {} in {}/{}",
                user_id, server_id, channel_id
            ),
            Self::Cli {
                session_id,
                command,
            } => format!(
                "CLI session {} - {}",
                session_id,
                command.as_deref().unwrap_or("interactive")
            ),
            Self::Api {
                client_id,
                endpoint,
                ..
            } => format!("API request from {} to {}", client_id, endpoint),
            Self::Bluesky {
                handle,
                is_mention,
                is_reply,
                post_uri,
                ..
            } => {
                let mut post_framing = if *is_mention {
                    format!("Mentioned by @{}", handle)
                } else if *is_reply {
                    format!("Reply from @{}", handle)
                } else {
                    format!("Post from @{}", handle)
                };

                if let Some(post_uri) = post_uri {
                    post_framing.push_str(&format!("aturi: {}", post_uri));
                }
                post_framing
            }
            Self::Agent { agent_id, reason } => format!("Agent {} - {}", agent_id, reason),
            Self::Other {
                origin_type,
                source_id,
                ..
            } => format!("{} from {}", origin_type, source_id),
        }
    }
}

/// Trait for message delivery endpoints
#[async_trait::async_trait]
pub trait MessageEndpoint: Send + Sync {
    /// Send a message to this endpoint
    async fn send(
        &self,
        message: Message,
        metadata: Option<Value>,
        origin: Option<&MessageOrigin>,
    ) -> Result<()>;

    /// Get the endpoint type name
    fn endpoint_type(&self) -> &'static str;
}

/// Routes messages from agents to their destinations
#[derive(Clone)]
pub struct AgentMessageRouter<C: surrealdb::Connection = surrealdb::engine::any::Any> {
    /// The agent this router belongs to
    agent_id: AgentId,

    /// Database connection for queuing messages
    db: Surreal<C>,

    /// Map of endpoint types to their implementations
    endpoints: Arc<RwLock<HashMap<String, Arc<dyn MessageEndpoint>>>>,

    /// Default endpoint for user messages
    default_user_endpoint: Arc<RwLock<Option<Arc<dyn MessageEndpoint>>>>,
}

impl<C: surrealdb::Connection> AgentMessageRouter<C> {
    /// Create a new message router for an agent
    pub fn new(agent_id: AgentId, db: Surreal<C>) -> Self {
        Self {
            agent_id,
            db,
            endpoints: Arc::new(RwLock::new(HashMap::new())),
            default_user_endpoint: Arc::new(RwLock::new(None)),
        }
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// Set the default endpoint for user messages (builder pattern)
    pub fn with_default_user_endpoint(self, endpoint: Arc<dyn MessageEndpoint>) -> Self {
        // Can't modify self here since we moved it, so we'll lock and set
        *self.default_user_endpoint.blocking_write() = Some(endpoint);
        self
    }

    /// Set the default user endpoint at runtime
    pub async fn set_default_user_endpoint(&self, endpoint: Arc<dyn MessageEndpoint>) {
        let mut default_endpoint = self.default_user_endpoint.write().await;
        *default_endpoint = Some(endpoint);
    }

    /// Register an endpoint
    pub async fn register_endpoint(&self, name: String, endpoint: Arc<dyn MessageEndpoint>) {
        let mut endpoints = self.endpoints.write().await;
        endpoints.insert(name, endpoint);
    }

    /// Send a message to the specified target
    pub async fn send_message(
        &self,
        target: MessageTarget,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    ) -> Result<()> {
        match target.target_type {
            TargetType::User => {
                let user_id = target
                    .target_id
                    .as_ref()
                    .and_then(|id| id.parse::<UserId>().ok())
                    .unwrap_or_else(UserId::nil);
                self.send_to_user(user_id, content, metadata, origin).await
            }
            TargetType::Agent => {
                let agent_id = target
                    .target_id
                    .as_ref()
                    .and_then(|id| id.parse::<AgentId>().ok())
                    .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                        tool_name: "send_message".to_string(),
                        cause: "Agent ID required for agent target".to_string(),
                        parameters: serde_json::json!({ "target": target }),
                    })?;
                self.send_to_agent(agent_id, content, metadata, origin)
                    .await
            }
            TargetType::Group => {
                let group_id = target
                    .target_id
                    .as_ref()
                    .and_then(|id| id.parse::<GroupId>().ok())
                    .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                        tool_name: "send_message".to_string(),
                        cause: "Group ID required for group target".to_string(),
                        parameters: serde_json::json!({ "target": target }),
                    })?;
                self.send_to_group(group_id, content, metadata, origin)
                    .await
            }
            TargetType::Channel => {
                let channel_info = metadata
                    .clone()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                self.send_to_channel(channel_info, content, metadata, origin)
                    .await
            }
            TargetType::Bluesky => {
                self.send_to_bluesky(target.target_id, content, metadata, origin)
                    .await
            }
        }
    }

    /// Send a message to a user
    async fn send_to_user(
        &self,
        user_id: UserId,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    ) -> Result<()> {
        debug!(
            "Routing message from agent {} to user {}",
            self.agent_id, user_id
        );

        // If we have a default user endpoint, use it
        let default_endpoint = self.default_user_endpoint.read().await;
        if let Some(endpoint) = default_endpoint.as_ref() {
            // Create message with role based on origin
            let message = match &origin {
                Some(MessageOrigin::Agent { .. }) => Message::agent(content),
                _ => Message::user(content), // External origins use User role
            };
            endpoint.send(message, metadata, origin.as_ref()).await?;
        } else {
            // Queue the message for later delivery
            let queued = QueuedMessage::agent_to_agent(
                self.agent_id.clone(),
                // TODO: We need to look up the user's primary agent or notification agent
                // For now, just log it
                AgentId::nil(),
                content,
                metadata,
                origin,
            );

            warn!(
                "No user endpoint configured, would queue message: {:?}",
                queued
            );
        }

        Ok(())
    }

    /// Send a message to another agent
    async fn send_to_agent(
        &self,
        target_agent_id: AgentId,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    ) -> Result<()> {
        debug!(
            "Routing message from agent {} to agent {}",
            self.agent_id, target_agent_id
        );

        // Create the queued message
        let queued = QueuedMessage::agent_to_agent(
            self.agent_id.clone(),
            target_agent_id.clone(),
            content,
            metadata,
            origin,
        );

        // Check for loops - if this agent is already in the call chain, prevent sending
        if queued.is_in_call_chain(&target_agent_id) {
            warn!(
                "Preventing message loop: agent {} is already in call chain {:?}",
                target_agent_id, queued.call_chain
            );
            return Ok(());
        }

        // Store the message in the database
        self.store_queued_message(queued).await?;

        Ok(())
    }

    /// Send a message to a group
    async fn send_to_group(
        &self,
        group_id: GroupId,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    ) -> Result<()> {
        debug!(
            "Routing message from agent {} to group {}",
            self.agent_id, group_id
        );

        // Get the group with its coordination pattern
        let group =
            crate::coordination::groups::AgentGroup::load_with_relations(&self.db, &group_id)
                .await?
                .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: "send_to_group".to_string(),
                    cause: format!("Group {} not found", group_id),
                    parameters: serde_json::json!({ "group_id": group_id }),
                })?;

        let members = group.members;
        if members.is_empty() {
            warn!("Group {} has no members", group_id);
            return Ok(());
        }

        info!(
            "Sending message to group {} with {} members using {:?} pattern",
            group_id,
            members.len(),
            group.coordination_pattern
        );

        // Route based on coordination pattern
        use crate::coordination::types::CoordinationPattern;
        match &group.coordination_pattern {
            CoordinationPattern::Supervisor { leader_id, .. } => {
                if let Some(MessageOrigin::Agent { agent_id, .. }) = &origin {
                    if agent_id == leader_id {
                        // Leader can broadcast to all group members
                        let mut sent_count = 0;
                        for (agent_record, membership) in members {
                            if !membership.is_active {
                                debug!("Skipping inactive member {}", agent_record.id);
                                continue;
                            }

                            if &agent_record.id == leader_id {
                                debug!("Leader doesn't message itself {}", agent_record.id);
                                continue;
                            }

                            let queued = QueuedMessage::agent_to_agent(
                                self.agent_id.clone(),
                                agent_record.id.clone(),
                                content.clone(),
                                metadata.clone(),
                                origin.clone(),
                            );

                            if let Err(e) = self.store_queued_message(queued).await {
                                warn!(
                                    "Failed to queue message for group member {}: {}",
                                    agent_record.id, e
                                );
                            } else {
                                sent_count += 1;
                            }
                        }
                        info!(
                            "Broadcast message to {} non-supervisor members of group {}",
                            sent_count, group_id
                        );
                    } else {
                        // Send only to the leader
                        let queued = QueuedMessage::agent_to_agent(
                            self.agent_id.clone(),
                            leader_id.clone(),
                            content,
                            metadata,
                            origin.clone(),
                        );
                        self.store_queued_message(queued).await?;
                        info!("Sent message to group supervisor {}", leader_id);
                    }
                } else {
                    // Send only to the leader
                    let queued = QueuedMessage::agent_to_agent(
                        self.agent_id.clone(),
                        leader_id.clone(),
                        content,
                        metadata,
                        origin,
                    );
                    self.store_queued_message(queued).await?;
                    info!("Sent message to group supervisor {}", leader_id);
                }
            }

            CoordinationPattern::RoundRobin {
                current_index,
                skip_unavailable,
            } => {
                // Send to the agent whose turn it is
                let active_members: Vec<_> = members
                    .iter()
                    .filter(|(_, m)| !skip_unavailable || m.is_active)
                    .collect();

                if !active_members.is_empty() {
                    let target_idx = current_index % active_members.len();
                    let (agent_record, _) = &active_members[target_idx];

                    let queued = QueuedMessage::agent_to_agent(
                        self.agent_id.clone(),
                        agent_record.id.clone(),
                        content,
                        metadata,
                        origin,
                    );
                    self.store_queued_message(queued).await?;
                    info!(
                        "Sent message to round-robin member {} at index {}",
                        agent_record.id, target_idx
                    );

                    // TODO: Update group state to increment current_index
                }
            }

            CoordinationPattern::Pipeline { stages, .. } => {
                // Send to the first stage's first agent
                // TODO: see about better routing here.
                if let Some(first_stage) = stages.first() {
                    if let Some(first_agent) = first_stage.agent_ids.first() {
                        let queued = QueuedMessage::agent_to_agent(
                            self.agent_id.clone(),
                            first_agent.clone(),
                            content,
                            metadata,
                            origin,
                        );
                        self.store_queued_message(queued).await?;
                        info!("Sent message to pipeline first stage agent {}", first_agent);
                    }
                }
            }

            _ => {
                // Default behavior: broadcast to all active members
                let mut sent_count = 0;
                for (agent_record, membership) in members {
                    if !membership.is_active {
                        debug!("Skipping inactive member {}", agent_record.id);
                        continue;
                    }

                    let queued = QueuedMessage::agent_to_agent(
                        self.agent_id.clone(),
                        agent_record.id.clone(),
                        content.clone(),
                        metadata.clone(),
                        origin.clone(),
                    );

                    if let Err(e) = self.store_queued_message(queued).await {
                        warn!(
                            "Failed to queue message for group member {}: {}",
                            agent_record.id, e
                        );
                    } else {
                        sent_count += 1;
                    }
                }
                info!(
                    "Broadcast message to {} active members of group {}",
                    sent_count, group_id
                );
            }
        }

        info!("Completed group message routing for group {}", group_id);

        Ok(())
    }

    /// Send a message to a channel (Discord, etc)
    async fn send_to_channel(
        &self,
        channel_info: Value,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    ) -> Result<()> {
        debug!("Routing message from agent {} to channel", self.agent_id);

        // Extract channel type from the info
        let channel_type = channel_info
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Look for appropriate endpoint
        let endpoints = self.endpoints.read().await;
        if let Some(endpoint) = endpoints.get(channel_type) {
            // Create message with role based on origin
            let message = match &origin {
                Some(MessageOrigin::Agent { .. }) => Message::agent(content),
                _ => Message::user(content), // External origins use User role
            };
            endpoint.send(message, metadata, origin.as_ref()).await?;
        } else {
            warn!("No endpoint registered for channel type: {}", channel_type);
        }

        Ok(())
    }

    /// Send a message to Bluesky
    async fn send_to_bluesky(
        &self,
        target_uri: Option<String>,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    ) -> Result<()> {
        debug!("Routing message from agent {} to Bluesky", self.agent_id);

        // Look for Bluesky endpoint
        let endpoints = self.endpoints.read().await;
        if let Some(endpoint) = endpoints.get("bluesky") {
            let message = Message::agent(content);

            // Include the target URI in metadata if it's a reply
            let mut final_metadata = metadata.unwrap_or_else(|| Value::Object(Default::default()));
            if let Some(uri) = target_uri {
                if let Some(obj) = final_metadata.as_object_mut() {
                    obj.insert("reply_to".to_string(), Value::String(uri));
                }
            }

            endpoint
                .send(message, Some(final_metadata), origin.as_ref())
                .await?;
        } else {
            warn!("No Bluesky endpoint registered");
        }

        Ok(())
    }

    /// Store a queued message in the database
    async fn store_queued_message(&self, message: QueuedMessage) -> Result<()> {
        info!(
            "Storing queued message from {} to {}",
            message.from_agent.as_ref().unwrap_or(&AgentId::nil()),
            message.to_agent
        );

        // Store the message
        message.store_with_relations(&self.db).await?;

        Ok(())
    }
}

impl<C: surrealdb::Connection> std::fmt::Debug for AgentMessageRouter<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMessageRouter")
            .field("agent_id", &self.agent_id)
            .field(
                "has_default_endpoint",
                &self.default_user_endpoint.blocking_read().is_some(),
            )
            .finish()
    }
}

// CLI endpoint moved to pattern_cli crate for better separation of concerns

/// An endpoint that queues messages to the database
pub struct QueueEndpoint {
    pub db: Surreal<surrealdb::engine::any::Any>,
    pub from_agent: AgentId,
}

impl QueueEndpoint {
    pub fn new(db: Surreal<surrealdb::engine::any::Any>, from_agent: AgentId) -> Self {
        Self { db, from_agent }
    }
}

#[async_trait::async_trait]
impl MessageEndpoint for QueueEndpoint {
    async fn send(
        &self,
        _message: Message,
        _metadata: Option<Value>,
        _origin: Option<&MessageOrigin>,
    ) -> Result<()> {
        // For now, we'll need to extract the target from metadata
        // This is a fallback endpoint
        warn!("QueueEndpoint used - this should be replaced with proper routing");
        Ok(())
    }

    fn endpoint_type(&self) -> &'static str {
        "queue"
    }
}

/// Create a message router from the global database connection
pub async fn create_router_with_global_db(agent_id: AgentId) -> Result<AgentMessageRouter> {
    // Clone the global DB instance
    let db = client::DB.clone();
    Ok(AgentMessageRouter::new(agent_id, db))
}

// ===== Bluesky Endpoint Implementation =====

/// Endpoint for sending messages to Bluesky/ATProto
#[derive(Clone)]
pub struct BlueskyEndpoint {
    agent: Arc<tokio::sync::RwLock<bsky_sdk::BskyAgent>>,
    #[allow(dead_code)]
    handle: String,
    #[allow(dead_code)]
    did: String,
}

impl BlueskyEndpoint {
    /// Create a new Bluesky endpoint with authentication
    pub async fn new(
        credentials: crate::atproto_identity::AtprotoAuthCredentials,
        handle: String,
    ) -> Result<Self> {
        let pds_url = match resolve_handle_to_pds(&handle).await {
            Ok(url) => url,
            Err(url) => url,
        };

        let agent = bsky_sdk::BskyAgent::builder()
            .config(bsky_sdk::agent::config::Config {
                endpoint: pds_url,
                ..Default::default()
            })
            .build()
            .await
            .map_err(|e| crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_endpoint".to_string(),
                cause: format!("Failed to create BskyAgent: {:?}", e),
                parameters: serde_json::json!({}),
            })?;

        info!("credentials:{:?}", credentials);

        // Authenticate based on credential type
        let session = match credentials {
            crate::atproto_identity::AtprotoAuthCredentials::OAuth { access_token: _ } => {
                // TODO: OAuth support - for now, return error
                return Err(crate::CoreError::ToolExecutionFailed {
                    tool_name: "bluesky_endpoint".to_string(),
                    cause: "OAuth authentication not yet implemented for BskyAgent".to_string(),
                    parameters: serde_json::json!({}),
                });
            }
            crate::atproto_identity::AtprotoAuthCredentials::AppPassword {
                identifier,
                password,
            } => agent.login(identifier, password).await.map_err(|e| {
                crate::CoreError::ToolExecutionFailed {
                    tool_name: "bluesky_endpoint".to_string(),
                    cause: format!("Login failed: {:?}", e),
                    parameters: serde_json::json!({}),
                }
            })?,
        };

        info!("Authenticated to Bluesky as {:?}", session.handle);

        Ok(Self {
            agent: Arc::new(tokio::sync::RwLock::new(agent)),
            handle: handle,
            did: session.did.to_string(),
        })
    }

    /// Create proper reply references with both parent and root
    async fn create_reply_refs(
        &self,
        reply_to_uri: &str,
    ) -> Result<atrium_api::app::bsky::feed::post::ReplyRefData> {
        let agent = self.agent.write().await;

        // Fetch the post thread to get reply information
        let post_result = agent
            .api
            .app
            .bsky
            .feed
            .get_posts(
                atrium_api::app::bsky::feed::get_posts::ParametersData {
                    uris: vec![reply_to_uri.to_string()],
                }
                .into(),
            )
            .await
            .map_err(|e| crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_endpoint".to_string(),
                cause: format!("Failed to fetch post for reply: {}", e),
                parameters: serde_json::json!({ "reply_to": reply_to_uri }),
            })?;

        let post = post_result.posts.iter().next();

        let new_parent_ref = post.map(|parent_post| strong_ref::MainData {
            cid: parent_post.cid.clone(),
            uri: parent_post.uri.clone(),
        });

        let parent_ref: Option<ReplyRef> = post.and_then(|post| {
            atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(post.record.clone())
                .ok()
                .and_then(|post| post.reply)
        });

        match (parent_ref, new_parent_ref) {
            // Parent post isn't a reply
            (None, Some(new_parent_ref)) => Ok(ReplyRefData {
                parent: new_parent_ref.clone().into(),
                root: new_parent_ref.into(),
            }),
            // parent post is a reply
            (Some(parent_ref), Some(new_parent_ref)) => Ok(ReplyRefData {
                parent: new_parent_ref.into(),
                root: parent_ref.root.clone(),
            }),
            // something went wrong
            (None, None) => Err(crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_endpoint".to_string(),
                cause: format!("Failed to get post: {}", reply_to_uri),
                parameters: serde_json::json!({}),
            }),
            // something went VERY wrong
            (Some(_), None) => Err(crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_endpoint".to_string(),
                cause: format!("Failed to get post: {}", reply_to_uri),
                parameters: serde_json::json!({}),
            }),
        }
    }
}

#[async_trait::async_trait]
impl MessageEndpoint for BlueskyEndpoint {
    async fn send(
        &self,
        message: Message,
        metadata: Option<Value>,
        _origin: Option<&MessageOrigin>,
    ) -> Result<()> {
        let text = match &message.content {
            MessageContent::Text(t) => t.clone(),
            MessageContent::Parts(parts) => {
                // Extract text from parts
                parts
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => "[Non-text content]".to_string(),
        };

        debug!("Sending message to Bluesky: {}", text);

        // Check if this is a reply
        let is_reply = if let Some(meta) = &metadata {
            if let Some(reply_to) = meta.get("reply_to").and_then(|v| v.as_str()) {
                info!("Creating reply to: {}", reply_to);
                true
            } else {
                false
            }
        } else {
            false
        };

        // Create reply reference if needed
        let reply = if is_reply {
            if let Some(meta) = &metadata {
                if let Some(reply_to) = meta.get("reply_to").and_then(|v| v.as_str()) {
                    Some(self.create_reply_refs(reply_to).await?)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Create the post
        let agent = self.agent.read().await;
        let text_copy = text.clone();
        let result = agent
            .create_record(atrium_api::app::bsky::feed::post::RecordData {
                created_at: atrium_api::types::string::Datetime::now(),
                text,
                reply: reply.map(|r| r.into()),
                embed: None,
                entities: None,
                facets: None,
                labels: None,
                langs: None,
                tags: None,
            })
            .await
            .map_err(|e| crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_endpoint".to_string(),
                cause: format!("Failed to create post: {}", e),
                parameters: serde_json::json!({ "text": text_copy }),
            })?;

        info!(
            "Posted to Bluesky: {} ({})",
            result.uri,
            if is_reply { "reply" } else { "new post" }
        );

        Ok(())
    }

    fn endpoint_type(&self) -> &'static str {
        "bluesky"
    }
}

/// Create a Bluesky endpoint from stored credentials
pub async fn create_bluesky_endpoint_from_identity(
    identity: &crate::atproto_identity::AtprotoIdentity,
) -> Result<BlueskyEndpoint> {
    let credentials =
        identity
            .get_auth_credentials()
            .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_endpoint".to_string(),
                cause: "No authentication credentials available for ATProto identity".to_string(),
                parameters: serde_json::json!({}),
            })?;

    BlueskyEndpoint::new(credentials, identity.handle.clone()).await
}
