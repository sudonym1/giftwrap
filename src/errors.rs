use std::process::ExitStatus;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GiftwrapError {
    #[error("{message}")]
    Usage {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    Config {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    Tooling {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    Build {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    Runtime {
        message: String,
        hint: Option<String>,
    },
    #[error("{message}")]
    Cache {
        message: String,
        hint: Option<String>,
    },
    #[error("cache lock timeout for {ctx_sha}")]
    CacheLockTimeout { ctx_sha: String },
}

impl GiftwrapError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
            hint: None,
        }
    }

    pub fn usage_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
            hint: None,
        }
    }

    pub fn config_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn tooling(message: impl Into<String>) -> Self {
        Self::Tooling {
            message: message.into(),
            hint: None,
        }
    }

    pub fn tooling_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Tooling {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self::Build {
            message: message.into(),
            hint: None,
        }
    }

    pub fn build_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Build {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime {
            message: message.into(),
            hint: None,
        }
    }

    pub fn runtime_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Runtime {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn cache(message: impl Into<String>) -> Self {
        Self::Cache {
            message: message.into(),
            hint: None,
        }
    }

    pub fn cache_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Cache {
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn cache_lock_timeout(ctx_sha: impl Into<String>) -> Self {
        Self::CacheLockTimeout {
            ctx_sha: ctx_sha.into(),
        }
    }

    pub fn build_command_failed(phase: &str, status: &ExitStatus) -> Self {
        Self::build(format!("failed to {phase} ({})", exit_status_desc(status)))
    }

    pub fn runtime_command_failed(phase: &str, status: &ExitStatus) -> Self {
        Self::runtime(format!("failed to {phase} ({})", exit_status_desc(status)))
    }

    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::Usage { hint, .. }
            | Self::Config { hint, .. }
            | Self::Tooling { hint, .. }
            | Self::Build { hint, .. }
            | Self::Runtime { hint, .. }
            | Self::Cache { hint, .. } => hint.as_deref(),
            Self::CacheLockTimeout { .. } => {
                Some("another giftwrap process is building this context")
            }
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } | Self::Config { .. } | Self::Tooling { .. } => 2,
            Self::Build { .. } => 3,
            Self::Cache { .. } | Self::CacheLockTimeout { .. } => 4,
            Self::Runtime { .. } => 1,
        }
    }
}

pub fn exit_status_desc(status: &ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit {code}"),
        None => "terminated by signal".to_string(),
    }
}
