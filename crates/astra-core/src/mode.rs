use serde::{Deserialize, Serialize};

use crate::span::Span;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Core,
    App,
    Web,
    System,
}

impl Mode {

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::App => "app",
            Self::Web => "web",
            Self::System => "system",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "core" => Some(Self::Core),
            "app" => Some(Self::App),
            "web" => Some(Self::Web),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    #[must_use]
    pub const fn allows(self, required: Self) -> bool {
        matches!(
            (self, required),
            (Self::Core, Self::Core)
                | (Self::App, Self::Core | Self::App)
                | (Self::Web, Self::Core | Self::App | Self::Web)
                | (Self::System, Self::Core | Self::App | Self::System)
        )
    }

    #[must_use]
    pub fn requirement_for_capability(tag: &str) -> Option<Self> {
        match tag {
            "console" | "args" | "time" | "env" => Some(Self::App),
            "dom" | "fetch" | "worker" | "storage" | "websocket" => Some(Self::Web),
            "fs" | "net" | "process" | "ffi" | "thread" | "ptr" | "memory" | "os" => {
                Some(Self::System)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeRequirement {
    pub mode: Mode,
    pub span: Span,
}
