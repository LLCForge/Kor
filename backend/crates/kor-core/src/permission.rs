use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How much the agent may do without asking.
///
/// Kor config uses kebab-case (`accept-edits`). `settings.json` may use
/// camelCase (`acceptEdits`, `bypassPermissions`, `dontAsk`); both deserialize.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Reads are free; edits and commands ask.
    #[default]
    #[serde(rename = "default")]
    Default,
    /// Read-only: edits and commands are denied.
    #[serde(rename = "plan")]
    Plan,
    /// Edits are free; commands ask.
    #[serde(rename = "accept-edits", alias = "acceptEdits", alias = "accept_edits")]
    AcceptEdits,
    /// Everything allowed except deny rules and dangerous commands. Requires an OS sandbox.
    #[serde(rename = "auto")]
    Auto,
    /// Fail closed: anything that would ask is denied (`dontAsk`).
    #[serde(rename = "dont-ask", alias = "dontAsk")]
    DontAsk,
    /// Skip prompts, including the dangerous-command guard. Deny rules still apply.
    /// `bypassPermissions`: do not stop for prompts.
    #[serde(rename = "bypass-permissions", alias = "bypassPermissions")]
    BypassPermissions,
}

impl PermissionMode {
    pub const ALL: [Self; 6] = [
        Self::Default,
        Self::Plan,
        Self::AcceptEdits,
        Self::Auto,
        Self::DontAsk,
        Self::BypassPermissions,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::AcceptEdits => "accept-edits",
            Self::Auto => "auto",
            Self::DontAsk => "dont-ask",
            Self::BypassPermissions => "bypass-permissions",
        }
    }

    /// A mode that grants more than the default (reads free, everything else asks).
    pub fn widens(self) -> bool {
        matches!(
            self,
            Self::AcceptEdits | Self::Auto | Self::BypassPermissions
        )
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PermissionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|m| {
                m.as_str() == s
                    || matches!(
                        (m, s),
                        (Self::AcceptEdits, "acceptEdits")
                            | (Self::DontAsk, "dontAsk")
                            | (Self::BypassPermissions, "bypassPermissions")
                    )
            })
            .ok_or_else(|| format!("unknown permission mode `{s}`"))
    }
}
