//! AI assistant foundation: the Anthropic API key, the Messages API client,
//! settings, pricing and the monthly budget.
//!
//! The chat (conversations, the streaming tool loop and the read tools)
//! builds on these; the auto-update AI recovery shares the same key and
//! budget.

pub mod actions;
pub mod budget;
pub mod chat;
pub mod client;
pub mod cluster;
pub mod conversations;
pub mod k8s_shape;
pub mod key_store;
pub mod pricing;
pub mod prompt;
pub mod retention;
pub mod settings;
pub mod stream;
pub mod tools;

use std::sync::Arc;

use crate::state::SharedK8sClient;

pub use budget::{ensure_budget, record_usage, UsageRecord};
pub use client::{ClientError, MessagesClient};
pub use key_store::{KeyStore, MemoryKeyStore, SecretKeyStore};
pub use pricing::AssistantModel;
pub use settings::{AssistantSettings, Effort};

/// What the assistant needs from the outside world, swappable in tests.
#[derive(Clone)]
pub struct AssistantDeps {
    pub key_store: Arc<dyn KeyStore>,
    pub client: MessagesClient,
}

impl AssistantDeps {
    /// The key in a Secret in the backend's namespace; Anthropic's production API.
    pub fn new(k8s: SharedK8sClient) -> Self {
        Self {
            key_store: Arc::new(SecretKeyStore::from_env(k8s)),
            client: MessagesClient::anthropic(),
        }
    }
}
