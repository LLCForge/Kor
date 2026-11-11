//! Second model call for `WebFetch`, using whichever Kor provider the session already has.

use std::sync::Arc;

use async_trait::async_trait;
use kor_core::{Message, Role};
use kor_providers::{CancellationToken, ChatRequest, Provider};
use kor_tools::PageSummarizer;

use crate::subagent::collect_text;

pub struct ProviderSummarizer {
    pub provider: Arc<dyn Provider>,
    pub model: String,
}

impl std::fmt::Debug for ProviderSummarizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSummarizer")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PageSummarizer for ProviderSummarizer {
    async fn summarize(&self, instructions: &str) -> Result<String, String> {
        let req = ChatRequest {
            model: self.model.clone(),
            system: Some("Follow the instructions in the user message. Text only.".into()),
            messages: vec![Message::text(Role::User, instructions)],
            tools: Vec::new(),
            max_tokens: Some(1024),
            temperature: None,
            reasoning: None,
        };
        let stream = self
            .provider
            .complete(req, CancellationToken::new())
            .await
            .map_err(|e| e.message)?;
        let text = collect_text(stream).await.map_err(|e| e.message)?;
        if text.trim().is_empty() {
            Err("summarizer returned no text".into())
        } else {
            Ok(text)
        }
    }
}
