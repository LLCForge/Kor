//! Shadow mode (Master Build Prompt §5.1): after a finished task a cheap model answers the same
//! prompt in the background and the agreement is measured. The policy (which model, where the
//! history lives, when to suggest switching) is `kor_runtime::consensus::ShadowRunner`.

/// Called by the agent after every finished task.
#[async_trait::async_trait]
pub trait Shadow: Send + Sync {
    /// Compare `answer` (the main model `main`'s final text for `prompt`) with the cheap model's.
    /// Returns a suggestion to show the user, if one is due.
    async fn observe(&self, prompt: &str, answer: &str, main: &str) -> Option<String>;
    /// A suggestion that arrived after the last task's events were closed; shown with the next.
    fn take_pending(&self) -> Option<String>;
}
