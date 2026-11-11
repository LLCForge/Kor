//! Shared, dependency-light types used by every Kor crate.
//!
//! Nothing in here performs I/O except [`net`], the network guard every HTTP client is built
//! from. Higher layers (providers, router, agent, daemon) exchange these types so that
//! provider-specific wire formats never leak upward.

/// Product version (what users install and `kor update` compares). Crates also have their own
/// component versions in their Cargo.toml; see `scripts/version.py` and ADR 0012.
pub const VERSION: &str = env!("KOR_VERSION");

mod error;
/// Typed IPC contract `schema/ipc.json` and its validator (§16.3, ADR 0061).
pub mod ipc;
/// `KOR_IPC_TRACE` / `kor doctor --ipc-trace` (§17.2).
pub mod ipc_trace;
mod message;
mod model;
pub mod net;
mod permission;
mod privacy;
mod reasoning;
pub mod schemas;
mod stream;
pub mod theme;
mod usage;

pub use error::{ProviderError, ProviderErrorKind};
pub use message::{ContentPart, ImageSource, Message, Role, ToolSpec};
pub use model::{ModelRef, ModelRefError, is_valid_provider_id};
pub use permission::PermissionMode;
pub use privacy::{PrivacyRequirement, PrivacyTier};
pub use reasoning::{Reasoning, ReasoningEffort};
pub use stream::{FinishReason, StreamEvent};
pub use usage::{Cost, Usage};
