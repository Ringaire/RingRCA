use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use crate::protocol::*;
use crate::adapter::dispatcher::Dispatcher;

/// A connected RingCLI instance.
pub struct CliSession {
    pub client_id: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub labels: Vec<String>,
    pub tx: mpsc::UnboundedSender<Envelope>,
    pub connected_at: i64,
}

impl CliSession {
    pub fn new(
        client_id: String,
        version: String,
        capabilities: Vec<String>,
        labels: Vec<String>,
    ) -> (Self, mpsc::UnboundedReceiver<Envelope>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Self {
            client_id,
            version,
            capabilities,
            labels,
            tx,
            connected_at: chrono::Utc::now().timestamp_millis(),
        };
        (session, rx)
    }
}

/// A conversation route: platform conversation → CLI session.
#[derive(Debug, Clone)]
pub struct ConversationRoute {
    pub cli_session_id: Uuid,
    pub platform: String,
    pub platform_user_id: String,
    pub conversation_id: String,
}

/// Core message router: connects CLI sessions with platform conversations.
pub struct Router {
    /// Connected RingCLI instances, keyed by session_id.
    cli_sessions: RwLock<HashMap<Uuid, CliSession>>,
    /// Platform conversation → CLI session.
    conversation_routes: RwLock<HashMap<String, ConversationRoute>>,
    /// CLI session → its active conversation keys (for cleanup).
    session_conv_index: RwLock<HashMap<Uuid, Vec<String>>>,
    /// Result dispatcher for platform adapters.
    pub dispatcher: Arc<Dispatcher>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            cli_sessions: RwLock::new(HashMap::new()),
            conversation_routes: RwLock::new(HashMap::new()),
            session_conv_index: RwLock::new(HashMap::new()),
            dispatcher: Arc::new(Dispatcher::new()),
        }
    }

    /// Register a RingCLI session.
    pub async fn register(&self, session_id: Uuid, session: CliSession) {
        self.cli_sessions.write().await.insert(session_id, session);
    }

    /// Unregister a RingCLI session, cleaning up all its routes.
    pub async fn unregister(&self, session_id: &Uuid) {
        self.cli_sessions.write().await.remove(session_id);
        let mut routes = self.conversation_routes.write().await;
        let mut index = self.session_conv_index.write().await;
        if let Some(conv_keys) = index.remove(session_id) {
            for key in conv_keys {
                routes.remove(&key);
            }
        }
    }

    /// Send envelope to a specific CLI session.
    pub async fn send_to_cli(&self, session_id: &Uuid, msg: Envelope) -> Result<(), String> {
        let sessions = self.cli_sessions.read().await;
        match sessions.get(session_id) {
            Some(s) => s.tx.send(msg).map_err(|e| format!("send failed: {e}")),
            None => Err(format!("CLI session {session_id} not found")),
        }
    }

    /// Route an incoming platform message to the correct CLI session.
    pub async fn route_to_cli(
        &self,
        platform: &str,
        conversation_id: &str,
        msg: Envelope,
    ) -> Result<(), String> {
        let key = route_key(platform, conversation_id);
        let routes = self.conversation_routes.read().await;
        match routes.get(&key) {
            Some(route) => self.send_to_cli(&route.cli_session_id, msg).await,
            None => Err(format!("no CLI session bound to {key}")),
        }
    }

    /// Bind a conversation to a CLI session (after first message).
    pub async fn bind_conversation(
        &self,
        session_id: Uuid,
        platform: &str,
        platform_user_id: &str,
        conversation_id: &str,
    ) {
        let key = route_key(platform, conversation_id);
        let route = ConversationRoute {
            cli_session_id: session_id,
            platform: platform.to_string(),
            platform_user_id: platform_user_id.to_string(),
            conversation_id: conversation_id.to_string(),
        };
        self.conversation_routes.write().await.insert(key.clone(), route);
        self.session_conv_index
            .write()
            .await
            .entry(session_id)
            .or_default()
            .push(key);
    }

    /// Pick an available CLI session (simple round-robin via first available).
    pub async fn pick_cli(&self) -> Option<Uuid> {
        let sessions = self.cli_sessions.read().await;
        sessions.keys().next().copied()
    }

    /// List connected CLI sessions.
    pub async fn list_clients(&self) -> Vec<(Uuid, String, Vec<String>)> {
        let sessions = self.cli_sessions.read().await;
        sessions
            .iter()
            .map(|(id, s)| (*id, s.client_id.clone(), s.capabilities.clone()))
            .collect()
    }

    /// Look up conversation routes for a CLI session.
    pub async fn routes_for_session(&self, session_id: &Uuid) -> Vec<ConversationRoute> {
        let index = self.session_conv_index.read().await;
        let routes = self.conversation_routes.read().await;
        match index.get(session_id) {
            Some(keys) => keys.iter().filter_map(|k| routes.get(k).cloned()).collect(),
            None => Vec::new(),
        }
    }

    /// Dispatch a TaskResult to the appropriate platform adapter.
    pub async fn dispatch_result(&self, session_id: &Uuid, result: &TaskResult) {
        let routes = self.routes_for_session(session_id).await;
        for route in &routes {
            self.dispatcher
                .dispatch(&route.conversation_id, &route.platform, result)
                .await;
        }
    }
}

fn route_key(platform: &str, conversation_id: &str) -> String {
    format!("{platform}/{conversation_id}")
}
