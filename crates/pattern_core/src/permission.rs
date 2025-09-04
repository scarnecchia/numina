use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionScope {
    MemoryEdit {
        key: String,
    },
    MemoryBatch {
        prefix: String,
    },
    ToolExecution {
        tool: String,
        args_digest: Option<String>,
    },
    DataSourceAction {
        source_id: String,
        action: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionGrant {
    pub id: String,
    pub scope: PermissionScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub agent_id: crate::AgentId,
    pub tool_name: String,
    pub scope: PermissionScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionDecisionKind {
    Deny,
    ApproveOnce,
    ApproveForDuration(std::time::Duration),
    ApproveForScope,
}

#[derive(Clone)]
pub struct PermissionBroker {
    tx: broadcast::Sender<PermissionRequest>,
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<PermissionDecisionKind>>>>,
}

impl PermissionBroker {
    fn new() -> Self {
        let (tx, _rx) = broadcast::channel(64);
        Self {
            tx,
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PermissionRequest> {
        self.tx.subscribe()
    }

    pub async fn request(
        &self,
        agent_id: crate::AgentId,
        tool_name: String,
        scope: PermissionScope,
        reason: Option<String>,
        timeout: std::time::Duration,
    ) -> Option<PermissionGrant> {
        let id = Uuid::new_v4().to_string();
        let (tx_decision, rx_decision) = oneshot::channel();
        {
            let mut p = self.pending.write().await;
            p.insert(id.clone(), tx_decision);
        }
        let _ = self.tx.send(PermissionRequest {
            id: id.clone(),
            agent_id: agent_id.clone(),
            tool_name: tool_name.clone(),
            scope: scope.clone(),
            reason,
        });

        match tokio::time::timeout(timeout, rx_decision).await {
            Ok(Ok(decision)) => match decision {
                PermissionDecisionKind::Deny => None,
                PermissionDecisionKind::ApproveOnce => Some(PermissionGrant {
                    id,
                    scope,
                    expires_at: None,
                }),
                PermissionDecisionKind::ApproveForScope => Some(PermissionGrant {
                    id,
                    scope,
                    expires_at: None,
                }),
                PermissionDecisionKind::ApproveForDuration(dur) => Some(PermissionGrant {
                    id,
                    scope,
                    expires_at: Some(
                        chrono::Utc::now() + chrono::Duration::from_std(dur).unwrap_or_default(),
                    ),
                }),
            },
            _ => None, // timeout or channel closed
        }
    }

    pub async fn resolve(&self, request_id: &str, decision: PermissionDecisionKind) -> bool {
        let tx_opt = { self.pending.write().await.remove(request_id) };
        if let Some(tx) = tx_opt {
            let _ = tx.send(decision);
            true
        } else {
            false
        }
    }
}

use std::sync::OnceLock;
static BROKER: OnceLock<PermissionBroker> = OnceLock::new();

pub fn broker() -> &'static PermissionBroker {
    BROKER.get_or_init(|| PermissionBroker::new())
}
