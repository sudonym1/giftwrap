use std::fmt;

/// High-level action requested by CLI flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAction {
    Run,
    PrintCommand,
    PrintContext,
    PrintImage,
    ShowConfig,
    Help,
}

/// Parsed CLI options before config/compose processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub action: CliAction,
    /// Force a specific context SHA/tag.
    pub use_ctx: Option<String>,
    /// Override the image name/tag.
    pub override_image: Option<String>,
    /// Rebuild the image before running.
    pub rebuild: bool,
    /// Extra args supplied via --gw-extra-args.
    pub extra_args: Vec<String>,
    /// Docker/Podman args provided before the `--` delimiter.
    pub docker_args: Vec<String>,
}

/// User command captured after the `--` delimiter (or remaining args).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCommand {
    pub argv: Vec<String>,
}

#[derive(Debug)]
pub struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

pub fn parse_args(args: &[String]) -> Result<(CliOptions, UserCommand), CliError> {
    let mut action = CliAction::Run;
    let mut use_ctx = None;
    let mut override_image = None;
    let mut rebuild = false;
    let mut extra_args = Vec::new();
    let mut docker_args = Vec::new();

    let mut idx = 0;
    let mut terminal_action = false;
    while idx < args.len() {
        let arg = &args[idx];
        if !arg.starts_with("--gw-") {
            break;
        }

        if arg == "--gw-print" {
            action = CliAction::PrintCommand;
        } else if arg == "--gw-ctx" {
            action = CliAction::PrintContext;
            terminal_action = true;
        } else if arg == "--gw-print-image" {
            action = CliAction::PrintImage;
            terminal_action = true;
        } else if let Some(rest) = arg.strip_prefix("--gw-use-ctx=") {
            use_ctx = Some(rest.to_string());
        } else if let Some(rest) = arg.strip_prefix("--gw-img=") {
            override_image = Some(rest.to_string());
        } else if arg == "--gw-rebuild" {
            rebuild = true;
        } else if let Some(rest) = arg.strip_prefix("--gw-extra-args=") {
            let parts = shell_words::split(rest).map_err(|err| {
                CliError::new(format!("Error: failed to parse --gw-extra-args: {err}"))
            })?;
            extra_args.extend(parts);
        } else if arg == "--gw-show-config" {
            action = CliAction::ShowConfig;
            terminal_action = true;
        } else if arg == "--gw-help" {
            action = CliAction::Help;
            terminal_action = true;
        }

        idx += 1;
        if terminal_action {
            break;
        }
    }

    let remaining = if terminal_action {
        Vec::new()
    } else {
        args[idx..].to_vec()
    };

    let user_cmd = if terminal_action {
        Vec::new()
    } else if let Some(pos) = remaining.iter().position(|arg| arg == "--") {
        docker_args = remaining[..pos].to_vec();
        remaining[pos + 1..].to_vec()
    } else {
        remaining
    };

    Ok((
        CliOptions {
            action,
            use_ctx,
            override_image,
            rebuild,
            extra_args,
            docker_args,
        },
        UserCommand { argv: user_cmd },
    ))
}
