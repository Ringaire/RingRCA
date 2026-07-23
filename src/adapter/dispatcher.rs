use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::error;

use crate::protocol::*;

/// Boxed async sender: token, conversation_id, result → () 
pub type BoxedSender = Box<dyn Fn(String, String, TaskResult) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Routes TaskResults back to the correct platform adapter.
pub struct Dispatcher {
    senders: RwLock<Vec<(String, Arc<BoxedSender>)>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            senders: RwLock::new(Vec::new()),
        }
    }

    /// Register a platform sender.
    pub async fn register(&self, platform: &str, sender: BoxedSender) {
        self.senders.write().await.push((platform.to_string(), Arc::new(sender)));
    }

    /// Unregister a platform sender (on disconnect).
    pub async fn unregister(&self, platform: &str) {
        self.senders.write().await.retain(|(p, _)| p != platform);
    }

    /// Dispatch a TaskResult to the appropriate platform.
    pub async fn dispatch(&self, conversation_id: &str, platform: &str, result: &TaskResult) {
        let senders = self.senders.read().await;
        let r = result.clone();
        let c = conversation_id.to_string();
        for (p, sender) in senders.iter() {
            if p == platform {
                let s = sender.clone();
                tokio::spawn(async move {
                    let fut = (s)(String::new(), c, r);
                    fut.await;
                });
                return;
            }
        }
        error!("no sender registered for platform: {platform}");
    }
}
